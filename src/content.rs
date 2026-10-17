//! Shaping results for the model. Text that other people wrote (invitation
//! descriptions, file contents, and later email bodies) is returned as data
//! with invisible formatting removed (RFC R6); results name these fields in a
//! `provenance` member. Long text is truncated and long lists are paged.
//! Downloaded content rests in one private folder per process for at most an
//! hour (RFC R10).

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::config::cache_dir;

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
/// fillers that emoji and some scripts are written with, the selectors and
/// fillers only where `hidden_here` finds them in use.
fn is_hidden(c: char) -> bool {
    let shaping = matches!(c as u32,
        0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF | 0x034F | 0x115F | 0x1160);
    (ignorable(c) && !shaping) || (c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

/// Characters in an identifier (a Drive path) that a reader cannot see or
/// type, so they are shown as escapes: every ignorable one, controls, and
/// whitespace other than the plain space, and the braille blank, which
/// shows as nothing.
fn invisible(c: char) -> bool {
    ignorable(c) || c.is_control() || (c.is_whitespace() && c != ' ') || c == '\u{2800}'
}

fn is_selector(c: char) -> bool {
    matches!(c as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

/// A Hangul leading consonant (choseong), the choseong filler among them.
fn is_choseong(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x115F | 0xA960..=0xA97C)
}

/// A Hangul vowel (jungseong), the jungseong filler among them.
fn is_jungseong(c: char) -> bool {
    matches!(c as u32, 0x1160..=0x11A7 | 0xD7B0..=0xD7C6)
}

/// Whether `c`, which `is_hidden` keeps, is hidden where it stands, after
/// `prev` and before `next` in the text as written. A variation selector is
/// kept only directly after a character it can vary (not another selector,
/// a joiner, a space or anything removed), so a run of them after one
/// letter, which can carry hidden bytes, keeps only its first. A Hangul
/// filler is kept only where it fills in for a jamo: the choseong filler
/// before a vowel, the jungseong filler after a leading consonant.
fn hidden_here(c: char, prev: Option<char>, next: Option<char>) -> bool {
    if is_selector(c) {
        return !prev.is_some_and(|p| {
            !is_selector(p)
                && !matches!(p, '\u{200C}' | '\u{200D}' | '\u{034F}')
                && !p.is_whitespace()
                && !is_hidden(p)
        });
    }
    match c {
        '\u{115F}' => !next.is_some_and(is_jungseong),
        '\u{1160}' => !prev.is_some_and(is_choseong),
        _ => false,
    }
}

/// `s` without hidden characters; `removed` counts what was dropped.
pub fn clean(s: &str, removed: &mut usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        let hidden = is_hidden(c) || hidden_here(c, prev, chars.peek().copied());
        *removed += usize::from(hidden);
        if !hidden {
            out.push(c);
        }
        prev = Some(c);
    }
    out
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

/// Cut `s` to at most `max` characters. Returns true if anything was cut.
pub fn truncate(s: &mut String, max: usize) -> bool {
    match s.char_indices().nth(max) {
        Some((i, _)) => {
            s.truncate(i);
            true
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
        bail!("unbalanced double quote in the query");
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
        .map_err(|e| anyhow::anyhow!("invalid pageToken: {e}"))?
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

/// This process's folder for downloaded mail attachments and Drive files, made
/// 0700 on first use. Nothing else on disk holds their content (RFC R10).
pub fn downloads() -> Result<PathBuf> {
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

/// Delete this process's download folder; every command ends with this. A
/// failure is reported, since it can hold message and file content.
pub fn remove_downloads() {
    remove(&downloads_path());
}

fn remove(dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        // No path: a server's stderr goes to its host's logs (R25).
        eprintln!("protonctl: cannot delete a download folder: {e}");
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

    /// R6: a variation selector stays only alone, right after a character it
    /// can vary, and a Hangul filler only inside a jamo sequence. Runs of
    /// selectors after one letter carry hidden bytes (one per selector from
    /// the 240 of the supplement), which nothing shows.
    #[test]
    fn selectors_and_fillers_stay_only_where_text_is_written_with_them() {
        for kept in [
            "❤\u{FE0F}",
            "葛\u{E0100}",
            "1\u{FE0F}\u{20E3}",
            "👁\u{FE0F}\u{200D}🗨\u{FE0F}",
            "👨\u{200D}👩\u{200D}👧",
            "\u{1100}\u{1161}",
            "\u{115F}\u{1161}",
            "\u{1100}\u{1160}\u{11A8}",
        ] {
            let mut n = 0;
            assert_eq!(clean(kept, &mut n), kept, "{kept:?}");
            assert_eq!(n, 0, "{kept:?}");
        }
        for (s, out, gone) in [
            // Bytes hidden after one letter: only the first selector stays.
            ("a\u{E0141}\u{E0142}\u{E0143}b", "a\u{E0141}b", 2),
            ("❤\u{FE0F}\u{FE0E}", "❤\u{FE0F}", 1),
            // A selector with nothing to vary: at the start, after a space
            // or a line break, after a joiner, after a removed character.
            ("\u{FE0F}x", "x", 1),
            (" \u{E0100}", " ", 1),
            ("a\n\u{FE0F}", "a\n", 1),
            ("👨\u{200D}\u{FE0F}", "👨\u{200D}", 1),
            ("a\u{200B}\u{FE0F}", "a", 2),
            // Hangul fillers with no jamo to fill in for.
            ("a\u{115F}b", "ab", 1),
            ("a\u{1160}b", "ab", 1),
            ("\u{1160}x\u{115F}", "x", 2),
        ] {
            let mut n = 0;
            assert_eq!(clean(s, &mut n), out, "{s:?}");
            assert_eq!(n, gone, "{s:?}");
        }
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
