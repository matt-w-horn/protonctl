//! Shaping results for the model. Text that other people wrote (invitation
//! descriptions, file contents, and later email bodies) is returned as data
//! with invisible formatting removed (RFC R6); results name these fields in a
//! `provenance` member. Long text is truncated and long lists are paged.
//! Downloaded content rests in one private folder per process for at most an
//! hour (RFC R10).

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::config::home;

/// Unicode tag characters, which can hide text from a human reader, and bidi
/// override/isolate controls, which can make text display differently than it
/// reads. Everything else passes through untouched.
fn is_hidden(c: char) -> bool {
    matches!(c as u32, 0xE0000..=0xE007F | 0x202A..=0x202E | 0x2066..=0x2069)
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
/// round trip, so hidden characters in them are shown as visible `\u{E0041}`
/// escapes, which `unescape_hidden` turns back into the real name. A backslash
/// that would begin such an escape is itself shown as `\u{5C}`, so a name that
/// really contains the text `\u{202E}` comes back as that text.
pub fn escape_hidden(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        if is_hidden(c) || (c == '\\' && s[i + 1..].starts_with("u{")) {
            write!(out, "\\u{{{:X}}}", c as u32).expect("writing to a String cannot fail");
        } else {
            out.push(c);
        }
    }
    out
}

/// Undo `escape_hidden`. Only escapes of hidden characters and of a backslash
/// are decoded, so a name that really contains `\u{41}` keeps it.
pub fn unescape_hidden(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\u{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 3..];
        let hidden = after.find('}').and_then(|j| {
            let c = char::from_u32(u32::from_str_radix(&after[..j], 16).ok()?)?;
            (is_hidden(c) || c == '\\').then_some((c, j))
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
        .context("invalid pageToken")?
        .unwrap_or(0);
    let end = offset.saturating_add(size);
    Ok((offset, (end < total).then(|| end.to_string())))
}

fn downloads_path() -> PathBuf {
    // Unit tests share one process and the real HOME; keep them out of it.
    let base = if cfg!(test) {
        std::env::temp_dir().join("protonctl-test")
    } else {
        home().join("Library/Caches/protonctl")
    };
    base.join(format!("downloads/{}", std::process::id()))
}

/// This process's folder for downloaded mail attachments and Drive files, made
/// 0700 on first use. Nothing else on disk holds their content (RFC R10).
pub fn downloads() -> Result<PathBuf> {
    let dir = downloads_path();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// Delete this process's download folder; every command ends with this. A
/// failure is reported, since the folder can hold message and file content.
pub fn remove_downloads() {
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

/// Delete the `drive-*` and `mail-*` folders directly in `dir` that are
/// expired at `now`, by the creation time on disk. Nothing else is touched:
/// not other names, not symlinks, and not `--out` folders, which lie outside.
fn sweep(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // no download yet
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name
            .to_str()
            .is_some_and(|n| n.starts_with("drive-") || n.starts_with("mail-"));
        // symlink_metadata: a link is not a folder of ours, whatever it points to.
        let created = std::fs::symlink_metadata(entry.path())
            .ok()
            .filter(std::fs::Metadata::is_dir)
            .and_then(|m| m.created().ok());
        if ours && created.is_some_and(|c| expired(c, now)) {
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
            Just("\\".to_string()),
        ];
        prop::collection::vec(token, 0..12).prop_map(|t| t.concat())
    }

    proptest! {
        #[test]
        fn escaping_hidden_characters_round_trips(s in tricky_text()) {
            let shown = escape_hidden(&s);
            prop_assert!(!shown.chars().any(is_hidden), "{:?}", shown);
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
    fn removes_tag_and_bidi_characters_only() {
        let mut n = 0;
        // "hi" + tag-encoded "x" + RLO + "ok" + PDI; the ZWJ emoji must survive.
        let s = "hi\u{E0078}\u{202E}ok\u{2069} 👨\u{200D}👩";
        assert_eq!(clean(s, &mut n), "hiok 👨\u{200D}👩");
        assert_eq!(n, 3);
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
        // Only hidden characters are decoded; a literal escape of anything else stays.
        assert_eq!(unescape_hidden("x\\u{41}y\\u{zz"), "x\\u{41}y\\u{zz");
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
        use std::fs::{File, FileTimes};
        use std::os::darwin::fs::FileTimesExt as _;
        let (dir, outside) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let now = SystemTime::now();
        let made = |parent: &Path, name: &str, mins: u64| {
            let p = parent.join(name);
            std::fs::create_dir(&p).unwrap();
            std::fs::write(p.join("file"), "content").unwrap();
            let created = now - Duration::from_mins(mins);
            let times = FileTimes::new().set_created(created);
            File::open(&p).unwrap().set_times(times).unwrap();
            p
        };
        let old = made(dir.path(), "drive-old", 61);
        let new = made(dir.path(), "mail-new", 59);
        let other = made(dir.path(), "notes", 120);
        // An old folder elsewhere, such as an `--out` folder, reached by a link.
        let target = made(outside.path(), "drive-out", 120);
        let link = dir.path().join("drive-link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        sweep(dir.path(), now);
        assert!(!old.exists());
        assert!(new.join("file").exists());
        assert!(other.join("file").exists());
        assert!(link.symlink_metadata().is_ok() && target.join("file").exists());
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
