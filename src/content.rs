//! Shaping results for the model. Text that other people wrote (invitation
//! descriptions, file contents, and later email bodies) is returned as data
//! with invisible formatting removed (RFC R6); results name these fields in a
//! `provenance` member. Long text is truncated and long lists are paged.
//! Downloaded content rests in one private folder per process for at most an
//! hour (RFC R10).

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config::cache_dir;
use crate::privacy::Fault;
use crate::privacy::error::Param;

/// Unicode's `Default_Ignorable_Code_Point` (`DerivedCoreProperties` 18.0.0):
/// characters a renderer shows as nothing, tag characters and bidi
/// overrides and isolates among them.
fn ignorable(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x034F | 0x061C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F
        | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A
        | 0xE0000..=0xE0FFF)
}

/// Characters removed from text other people wrote (R6): the ignorable ones,
/// which can hide text, make it display differently than it reads, or split
/// a word a reader or a filter looks for; and control characters other than
/// tab and line breaks, since ESC drives a terminal that prints the result.
/// Kept: the joiners, variation selectors, grapheme joiner and Hangul jamo
/// fillers that emoji and some scripts are written with.
fn is_hidden(c: char) -> bool {
    let shaping = matches!(c as u32,
        0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF | 0x034F | 0x115F | 0x1160);
    (ignorable(c) && !shaping) || (c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

/// Whether `clean` removes `c`: for detection, which must not let a hidden
/// character split a name (RFC section 6, Replacement).
pub fn hidden(c: char) -> bool {
    is_hidden(c)
}

/// Characters in an identifier (a Drive path) that a reader cannot see or
/// type, so they are shown as escapes: every ignorable one, controls, and
/// whitespace other than the plain space, and the braille blank, which
/// shows as nothing.
fn invisible(c: char) -> bool {
    ignorable(c) || c.is_control() || (c.is_whitespace() && c != ' ') || c == '\u{2800}'
}

/// `s` without hidden characters; `removed` counts what was dropped.
pub fn clean(s: &str, removed: &mut usize) -> String {
    s.chars()
        .filter(|&c| {
            let hidden = is_hidden(c);
            *removed += usize::from(hidden);
            !hidden
        })
        .collect()
}

/// Identifiers such as Drive paths cannot lose characters without breaking the
/// round trip, so characters a reader cannot see or type are shown in them as
/// visible `\u{200B}` escapes, which `unescape_hidden` turns back into the
/// real name. A backslash that would begin such an escape is itself shown as
/// `\u{5C}`, so a name that really contains the text `\u{202E}` comes back as
/// that text.
pub fn escape_hidden(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        if invisible(c) || (c == '\\' && s[i + 1..].starts_with("u{")) {
            write!(out, "\\u{{{:X}}}", c as u32).expect("writing to a String cannot fail");
        } else {
            out.push(c);
        }
    }
    out
}

/// Undo `escape_hidden`, and read any `\u{...}` of 1 to 6 hex digits as its
/// character, so a caller can type a name holding one. A name that really
/// holds the text `\u{41}` is shown with its backslash escaped, and so comes
/// back as that text.
pub fn unescape_hidden(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\u{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 3..];
        let hidden = after.find('}').and_then(|j| {
            let hex = &after[..j];
            let digits = (1..=6).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit());
            let c = char::from_u32(u32::from_str_radix(hex, 16).ok().filter(|_| digits)?)?;
            Some((c, j))
        });
        if let Some((c, j)) = hidden {
            out.push(c);
            rest = &after[j + 1..];
        } else {
            out.push_str("\\u{");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// A tool's result: its JSON, and a file the host is to give the model as
/// it is, which the JSON describes.
#[derive(Debug)]
pub struct Reply {
    pub json: Value,
    pub attached: Vec<Attached>,
}

#[derive(Debug)]
pub enum Attached {
    /// PNG, JPEG, GIF or WebP, which hosts show the model, after its label
    /// (such as "Page 3:") when there is one.
    Image {
        mime: &'static str,
        bytes: Vec<u8>,
        label: Option<String>,
    },
    /// Any other file, as an embedded resource, which not every host takes.
    Blob {
        uri: String,
        mime: &'static str,
        bytes: Vec<u8>,
    },
}

impl From<Value> for Reply {
    fn from(json: Value) -> Self {
        Self {
            json,
            attached: Vec::new(),
        }
    }
}

/// Cut `s` to at most `max` characters, or in aliases mode fewer, or past
/// a word or mention that starts the text (`cut_at`). Returns true if anything was cut.
pub fn truncate(s: &mut String, max: usize) -> bool {
    match s.char_indices().nth(max) {
        Some((i, _)) => {
            let i = cut_at(s, i, 0);
            let cut = i < s.len();
            s.truncate(i);
            cut
        }
        None => false,
    }
}

/// Split a query on spaces outside double quotes; `from:"Ann Lee"` and
/// `-"Focus time"` each stay one token.
pub fn tokens(query: &str) -> Result<Vec<String>> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in query.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if quoted {
        bail!(Invalid::rule("unbalanced double quote in the query"));
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

pub fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// The offset a `pageToken` names, and the token for the page after this one.
pub fn page(token: Option<&str>, size: usize, total: usize) -> Result<(usize, Option<String>)> {
    let offset: usize = token
        .map(str::parse)
        .transpose()
        .map_err(|e| Invalid::page_token(format!("invalid pageToken: {e}")))?
        .unwrap_or(0);
    let end = offset.saturating_add(size);
    Ok((offset, (end < total).then(|| end.to_string())))
}

/// `f` on the blocking pool. A panic there becomes an error that withholds
/// its message, as the panic hook does (R25): tokio's own error quotes it,
/// and it would reach stderr and the tool's error text.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        let what = if e.is_panic() {
            "panicked"
        } else {
            "was cancelled"
        };
        anyhow::anyhow!("internal error: a background task {what}; its message is withheld")
    })
}

/// Aliases mode's detectors at a cut: the span, in byte offsets, of the
/// mention a cut at the given byte of the text would split, if any.
pub type Cut = Arc<dyn Fn(&str, usize) -> Option<(usize, usize)> + Send + Sync>;

tokio::task_local! {
    /// Set for a call in aliases mode, where no content may reach the disk
    /// (RFC R10), no image is returned (R22), and text is cut only where
    /// `Cut` finds no mention (R13).
    static RESTRICTED: Cut;
}

/// Run `call` as aliases mode does: the download folder refused, since
/// every write of content goes through `downloads()`, so this one check
/// covers a saved attachment and a Drive file alike, which reaches the
/// Drive CLI only through `memory_folder`; no image
/// made, since none would be returned; and every cut of text moved off the
/// mentions `cut` finds, through `cut_at`.
pub async fn restricted<F: Future>(cut: Cut, call: F) -> F::Output {
    RESTRICTED.scope(cut, call).await
}

/// A `Cut` that finds nothing, for tests of the rest of aliases mode.
#[cfg(test)]
pub fn no_mentions() -> Cut {
    Arc::new(|_, _| None)
}

fn is_restricted() -> bool {
    RESTRICTED.try_with(|_| ()).is_ok()
}

/// How far `word_edge` looks for the whitespace around a word: further
/// than any word of a name.
const WORD: usize = 256;

/// `at`, or, when it falls inside a word, the start of that word if it
/// starts after `floor`, else, when `forward`, the word's end. A word too
/// long for `WORD`, as in a script written without spaces, keeps `at`.
fn word_edge(text: &str, at: usize, floor: usize, forward: bool) -> usize {
    let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
    if space(text[..at].chars().next_back()) || space(text[at..].chars().next()) {
        return at;
    }
    let lo = text.floor_char_boundary(at.saturating_sub(WORD)).max(floor);
    if let Some((i, c)) = text[lo..at].char_indices().rfind(|(_, c)| c.is_whitespace()) {
        return lo + i + c.len_utf8();
    }
    let hi = text.ceil_char_boundary(at.saturating_add(WORD));
    match text[at..hi].find(char::is_whitespace) {
        Some(i) if forward => at + i,
        _ => at,
    }
}

/// Where to cut `text` near byte `at`: at `at`, or in aliases mode back to
/// the start of the word it falls in (past its end when the word starts the
/// piece), then off any mention that would split, back to the mention's
/// start, or past its end when the mention starts at or before `floor`
/// (where the piece begins), so a page always moves on. The word comes
/// first because the pipeline finds names `Cut` does not: the result's own
/// headers, the query's, and short forms (R13).
pub fn cut_at(text: &str, at: usize, floor: usize) -> usize {
    let (at, mention) = RESTRICTED
        .try_with(|cut| {
            let at = word_edge(text, at, floor, true);
            (at, cut(text, at))
        })
        .unwrap_or((at, None));
    match mention {
        Some((start, _)) if start > floor => start,
        Some((_, end)) => end,
        None => at,
    }
}

/// Where a page of `text` asked to start at byte `at` starts: at `at`, or
/// in aliases mode at the start of the word, then of the mention, `at`
/// falls inside, so the page shows the whole name, which the pipeline
/// replaces, rather than its tail, which no detector finds (R13).
pub fn start_at(text: &str, at: usize) -> usize {
    let (at, mention) = RESTRICTED
        .try_with(|cut| {
            let at = word_edge(text, at, 0, false);
            (at, cut(text, at))
        })
        .unwrap_or((at, None));
    mention.map_or(at, |(start, _)| start)
}

/// False inside `restricted`, so a reader can answer without the file
/// rather than fail.
pub fn disk_allowed() -> bool {
    !is_restricted()
}

/// False inside `restricted`: a reader gives a reason instead of images.
pub fn images_allowed() -> bool {
    !is_restricted()
}

/// What a reader adds to a reason it cannot read a file for: the tool that
/// saves it, where there is one.
pub fn save_hint() -> &'static str {
    if disk_allowed() {
        "download_file saves it"
    } else {
        "the user can open it in Proton"
    }
}

/// The download folder was asked for inside `restricted`.
#[derive(Debug)]
pub struct DiskForbidden;

impl std::fmt::Display for DiskForbidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("aliases mode writes no content to disk (RFC R10)")
    }
}

impl std::error::Error for DiskForbidden {}

/// A parameter the caller sent is wrong. Off mode shows `text`, which can
/// quote the caller's input; aliases mode shows only `fault`, whose text
/// quotes nothing, since the input can hold the value a ref opened to (R13).
#[derive(Debug)]
pub struct Invalid {
    pub fault: Fault,
    pub text: String,
}

impl Invalid {
    /// A broken rule whose text quotes nothing, the same in both modes.
    pub fn rule(rule: &'static str) -> Self {
        Self::quoting(rule, rule)
    }

    /// A broken rule, with off mode's text, which may quote the input.
    pub fn quoting(rule: &'static str, text: impl Into<String>) -> Self {
        Self {
            fault: Fault::InvalidArgument(rule),
            text: text.into(),
        }
    }

    /// A page token that does not open, or belongs to another query.
    pub fn page_token(text: impl Into<String>) -> Self {
        Self {
            fault: Fault::InvalidPageToken,
            text: text.into(),
        }
    }
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Invalid {}

/// An item that does not exist, or is excluded (R7), so aliases mode can
/// answer `not_found` without quoting the message, which can name it.
#[derive(Debug)]
pub struct Missing {
    /// The parameter that names no item; `None` leaves it to the tool.
    pub param: Option<Param>,
    /// Off mode's text.
    pub text: String,
}

impl Missing {
    /// An item the tool's own parameter names (a Drive path or handle).
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            param: None,
            text: text.into(),
        }
    }

    pub fn of(param: Param, text: impl Into<String>) -> Self {
        Self {
            param: Some(param),
            text: text.into(),
        }
    }
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for Missing {}

/// protonctl's cache folder. Unit tests share one process and the real
/// HOME; keep them out of it.
fn cache_base() -> PathBuf {
    if cfg!(test) {
        std::env::temp_dir().join("protonctl-test")
    } else {
        cache_dir()
    }
}

fn downloads_path() -> PathBuf {
    cache_base().join(format!("downloads/{}", std::process::id()))
}

/// A new folder for one cloud-only Drive file that aliases mode reads, on
/// this process's memory disk (RFC R10, Q14). The reader deletes it once the
/// file is read; the disk goes at exit, with `remove_downloads`. Where there
/// is no such disk, the error is `DiskForbidden`, with why.
pub fn memory_folder() -> Result<PathBuf> {
    let mount = cache_base().join(format!("memory/{}", std::process::id()));
    let disk = crate::platform::memory_disk(&mount)
        .map_err(|e| anyhow::Error::new(DiskForbidden).context(format!("{e:#}")))?;
    Ok(tempfile::Builder::new()
        .prefix("drive-")
        .tempdir_in(disk)?
        .keep())
}

/// This process's folder for downloaded mail attachments and Drive files, made
/// 0700 on first use. Nothing else on disk holds their content (RFC R10).
pub fn downloads() -> Result<PathBuf> {
    if !disk_allowed() {
        return Err(DiskForbidden.into());
    }
    let dir = downloads_path();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// A new folder in this process's download folder for one download, named
/// `{kind}-{seconds}-{random}`, where `seconds` is the wall-clock time it was
/// made, in Unix seconds. The sweep reads a folder's age from its name,
/// because not every filesystem records when a folder was made.
pub fn download_folder(kind: &str) -> Result<PathBuf> {
    let made = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(tempfile::Builder::new()
        .prefix(&format!("{kind}-{made}-"))
        .tempdir_in(downloads()?)?
        .keep())
}

/// When a `drive-` or `mail-` download folder was made, from its name.
fn made_at(name: &str) -> Option<SystemTime> {
    let rest = name
        .strip_prefix("drive-")
        .or_else(|| name.strip_prefix("mail-"))?;
    let (seconds, _) = rest.split_once('-')?;
    // checked_add: a name's number can be too large for a SystemTime.
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds.parse().ok()?))
}

/// Delete this process's download folder and detach its memory disk; every
/// command ends with this. A failure is reported, since either can hold
/// message and file content.
pub fn remove_downloads() {
    crate::platform::remove_memory_disk();
    remove(&downloads_path());
}

fn remove(dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("protonctl: cannot delete {}: {e}", dir.display());
    }
}

/// How long one download stays on disk (RFC R10). A server in the Claude app
/// can run for days, so waiting for exit is not enough.
const DOWNLOAD_LIFE: Duration = Duration::from_mins(60);

/// Whether a download folder created at `created` is past its life at `now`.
/// Both are wall-clock times: `Instant` and tokio's timers stop while the Mac
/// sleeps, so a night's sleep would not count towards the hour. A creation
/// time in the future, after the clock was set back, is not expired.
pub fn expired(created: SystemTime, now: SystemTime) -> bool {
    now.duration_since(created)
        .is_ok_and(|age| age > DOWNLOAD_LIFE)
}

/// Delete this process's expired download folders. `serve` runs this at the
/// start of every tool call and once a minute.
pub fn sweep_downloads() {
    sweep(&downloads_path(), SystemTime::now());
}

/// Delete the folders `download_folder` made directly in `dir` that are
/// expired at `now`, by the time in their names. Nothing else is touched:
/// not other names, not symlinks, and not `--out` folders, which lie outside.
fn sweep(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // no download yet
    };
    for entry in entries.flatten() {
        let made = entry.file_name().to_str().and_then(made_at);
        // symlink_metadata: a link is not a folder of ours, whatever it points to.
        let folder = std::fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir());
        if folder && made.is_some_and(|m| expired(m, now)) {
            remove(&entry.path());
        }
    }
}

/// Files and bytes in this process's download folder, for `get_status`.
pub fn downloads_usage() -> Value {
    usage(&downloads_path())
}

fn usage(dir: &Path) -> Value {
    fn walk(dir: &Path, files: &mut u64, bytes: &mut u64) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(m) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if m.is_dir() {
                walk(&entry.path(), files, bytes);
            } else {
                *files += 1;
                *bytes += m.len();
            }
        }
    }
    let (mut files, mut bytes) = (0, 0);
    walk(dir, &mut files, &mut bytes);
    json!({ "files": files, "bytes": bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Text mixing hidden characters, escapes of them written out literally,
    /// stray escape syntax and any other character.
    fn tricky_text() -> impl Strategy<Value = String> {
        let token = prop_oneof![
            any::<char>().prop_map(String::from),
            Just("\u{202E}".to_string()),
            Just("\u{E0041}".to_string()),
            Just("\\u{202E}".to_string()),
            Just("\\u{E0041}".to_string()),
            Just("\\u{41}".to_string()),
            Just("\\u{".to_string()),
            Just("\u{200B}".to_string()),
            Just("\u{A0}".to_string()),
            Just("\u{1B}".to_string()),
            Just("\u{200D}".to_string()),
            Just("\\".to_string()),
        ];
        prop::collection::vec(token, 0..12).prop_map(|t| t.concat())
    }

    proptest! {
        #[test]
        fn escaping_hidden_characters_round_trips(s in tricky_text()) {
            let shown = escape_hidden(&s);
            prop_assert!(!shown.chars().any(invisible), "{:?}", shown);
            prop_assert_eq!(unescape_hidden(&shown), s);
        }

        #[test]
        fn clean_removes_exactly_the_hidden_characters(s in tricky_text()) {
            let mut removed = 0;
            let out = clean(&s, &mut removed);
            prop_assert_eq!(removed, s.chars().filter(|&c| is_hidden(c)).count());
            prop_assert_eq!(out, s.chars().filter(|&c| !is_hidden(c)).collect::<String>());
        }
    }

    #[test]
    fn removes_hidden_characters_and_keeps_what_text_is_written_with() {
        let mut n = 0;
        // "hi" + tag-encoded "x" + RLO + "ok" + PDI; the ZWJ emoji must survive.
        let s = "hi\u{E0078}\u{202E}ok\u{2069} 👨\u{200D}👩";
        assert_eq!(clean(s, &mut n), "hiok 👨\u{200D}👩");
        assert_eq!(n, 3);
        // Zero-width space, word joiner, BOM, soft hyphen, LRM, ESC and BEL
        // go; tab, line breaks, a no-break space, ZWNJ and an emoji's
        // variation selector stay.
        let mut n = 0;
        let s = "pa\u{200B}ss\u{2060}wo\u{FEFF}rd\u{AD}\u{200E} \u{1B}[31mred\u{7}";
        assert_eq!(clean(s, &mut n), "password [31mred");
        assert_eq!(n, 7);
        let kept = "a\tb\r\nc\u{A0}d\u{200C}e ❤\u{FE0F}";
        assert_eq!(clean(kept, &mut 0), kept);
    }

    /// RFC R13: in aliases mode a cut moves off a mention, back to its
    /// start, or past its end when it starts the text.
    #[tokio::test]
    async fn aliases_mode_truncates_beside_a_mention() {
        // A stand-in detector: the mention is bytes 3 to 9.
        let cut: Cut = Arc::new(|_, at| (3 < at && at < 9).then_some((3, 9)));
        let (inside, before) = restricted(cut.clone(), async {
            let mut a = "to ann@x.io now".to_string();
            let mut b = "ann@x.io now".to_string();
            (
                (truncate(&mut a, 6), a),
                restricted(Arc::new(|_, at| (at < 8).then_some((0, 8))), async {
                    (truncate(&mut b, 4), b)
                })
                .await,
            )
        })
        .await;
        assert_eq!(inside, (true, "to ".to_string()));
        assert_eq!(before, (true, "ann@x.io".to_string()));
        let mut off = "to ann@x.io now".to_string();
        assert!(truncate(&mut off, 6));
        assert_eq!(off, "to ann");
    }

    /// RFC R13 (#69): in aliases mode a cut or a start inside a word moves
    /// to the word's start, or a cut past its end when the word starts the
    /// piece, even where `Cut` finds nothing; a word longer than `WORD`
    /// keeps the cut.
    #[tokio::test]
    async fn aliases_mode_cuts_between_words() {
        let text = "Thanks, Dana Okonkwo will sign";
        let long = "x".repeat(WORD + 10);
        let (cuts, starts, from_floor, unspaced) = restricted(no_mentions(), async {
            (
                [9, 12, 13, 17].map(|at| cut_at(text, at, 0)),
                [9, 12, 13, 17].map(|at| start_at(text, at)),
                cut_at(text, 15, 13),
                (cut_at(&long, 5, 0), start_at(&long, WORD)),
            )
        })
        .await;
        assert_eq!(cuts, [8, 12, 13, 13]);
        assert_eq!(starts, [8, 12, 13, 13]);
        assert_eq!(from_floor, 20);
        assert_eq!(unspaced, (5, WORD));
        assert_eq!((cut_at(text, 17, 0), start_at(text, 17)), (17, 17));
    }

    #[test]
    fn truncates_on_character_boundaries() {
        let mut s = String::from("héllo");
        assert!(truncate(&mut s, 2));
        assert_eq!(s, "hé");
        let mut t = String::from("ab");
        assert!(!truncate(&mut t, 5));
    }

    #[test]
    fn hidden_characters_in_identifiers_round_trip_as_visible_escapes() {
        let name = "a\u{E0041}b\u{202E}c";
        let shown = escape_hidden(name);
        assert_eq!(shown, "a\\u{E0041}b\\u{202E}c");
        assert_eq!(unescape_hidden(&shown), name);
        // Any code point can be typed as an escape; a malformed one stays text.
        assert_eq!(unescape_hidden("x\\u{41}y\\u{zz"), "xAy\\u{zz");
        assert_eq!(
            unescape_hidden("\\u{+41}\\u{1234567}"),
            "\\u{+41}\\u{1234567}"
        );
        // Every character a reader cannot see or type is escaped, and comes back.
        let unseen = "\u{200B}\u{200C}\u{200D}\u{2060}\u{FEFF}\u{200E}\u{61C}\u{AD}\u{A0}\u{202F}\u{2028}\u{180E}\u{3164}\u{FE0F}\u{8F}";
        for c in unseen.chars() {
            let name = format!("a{c}b");
            let shown = escape_hidden(&name);
            assert_eq!(shown, format!("a\\u{{{:X}}}b", c as u32));
            assert_eq!(unescape_hidden(&shown), name);
        }
        // A name that contains an escape as text keeps it through the round trip.
        assert_eq!(escape_hidden("\\u{202E}"), "\\u{5C}u{202E}");
        assert_eq!(unescape_hidden("\\u{5C}u{202E}"), "\\u{202E}");
    }

    #[test]
    fn downloads_expire_after_an_hour_of_wall_clock_time() {
        let made = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        assert!(!expired(made, made + Duration::from_mins(59)));
        assert!(expired(made, made + Duration::from_mins(61)));
        // A night's sleep: the Mac slept from 23:00 to 07:00, and the folder
        // was made just before. Wall-clock age counts the night.
        assert!(expired(made, made + Duration::from_hours(8)));
        // The clock was set back past the folder's creation.
        assert!(!expired(made, made - Duration::from_mins(5)));
    }

    #[test]
    fn the_sweep_removes_only_expired_download_folders() {
        let (dir, outside) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let now = SystemTime::now();
        // A folder named as `download_folder` names one made `mins` ago.
        let made = |parent: &Path, kind: &str, mins: u64| {
            let then = now - Duration::from_mins(mins);
            let secs = then.duration_since(UNIX_EPOCH).unwrap().as_secs();
            let p = parent.join(format!("{kind}-{secs}-x1y2z3"));
            std::fs::create_dir(&p).unwrap();
            std::fs::write(p.join("file"), "content").unwrap();
            p
        };
        let old = made(dir.path(), "drive", 61);
        let new = made(dir.path(), "mail", 59);
        let other = made(dir.path(), "notes", 120);
        let unnamed = dir.path().join("drive-x1y2z3");
        std::fs::create_dir(&unnamed).unwrap();
        // An old folder elsewhere, such as an `--out` folder, reached by a
        // link whose name says it is old.
        let target = made(outside.path(), "drive", 120);
        let link = dir.path().join(target.file_name().unwrap());
        std::os::unix::fs::symlink(&target, &link).unwrap();
        sweep(dir.path(), now);
        assert!(!old.exists());
        assert!(new.join("file").exists());
        assert!(other.join("file").exists());
        assert!(unnamed.exists());
        assert!(link.symlink_metadata().is_ok() && target.join("file").exists());
    }

    #[test]
    fn a_download_folder_carries_the_time_it_was_made() {
        let before = SystemTime::now() - Duration::from_secs(1);
        let dir = download_folder("mail").unwrap();
        let name = dir.file_name().unwrap().to_str().unwrap();
        let made = made_at(name).unwrap();
        assert!(made >= before && made <= SystemTime::now(), "{name}");
        assert!(dir.starts_with(downloads().unwrap()), "{}", dir.display());
        std::fs::remove_dir(&dir).unwrap();
        assert_eq!(made_at("notes-1790000000-x"), None);
        assert_eq!(made_at("drive-soon-x"), None);
        assert_eq!(made_at(&format!("drive-{}-x", u64::MAX)), None);
    }

    /// R25: a panic on the blocking pool does not carry its message out.
    #[tokio::test]
    async fn a_blocking_panic_withholds_its_message() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let err = blocking(|| panic!("name Jane Doe")).await.unwrap_err();
        std::panic::set_hook(previous);
        let said = format!("{err:#}");
        assert!(
            said.contains("panicked") && !said.contains("Jane"),
            "{said}"
        );
    }

    #[tokio::test]
    async fn aliases_mode_gets_no_download_folder() {
        let refused = restricted(no_mentions(), async { download_folder("drive") })
            .await
            .unwrap_err();
        assert!(
            refused.downcast_ref::<DiskForbidden>().is_some(),
            "{refused:#}"
        );
        let dir = download_folder("drive").unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn usage_counts_files_and_bytes_in_every_download() {
        let dir = tempfile::tempdir().unwrap();
        for (folder, text) in [("drive-a", "hello"), ("mail-b", "world!")] {
            std::fs::create_dir(dir.path().join(folder)).unwrap();
            std::fs::write(dir.path().join(folder).join("f"), text).unwrap();
        }
        assert_eq!(usage(dir.path()), json!({ "files": 2, "bytes": 11 }));
        let missing = dir.path().join("none");
        assert_eq!(usage(&missing), json!({ "files": 0, "bytes": 0 }));
    }

    #[test]
    fn pages_follow_on_and_stop() {
        assert_eq!(page(None, 2, 5).unwrap(), (0, Some("2".into())));
        assert_eq!(page(Some("4"), 2, 5).unwrap(), (4, None));
        // A token from the model is untrusted: the largest offset must not overflow.
        assert_eq!(
            page(Some(&usize::MAX.to_string()), 2, 5).unwrap(),
            (usize::MAX, None)
        );
        assert!(page(Some("x"), 2, 5).is_err());
    }
}
