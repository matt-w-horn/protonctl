//! `~/.config/protonctl/config.toml` (or `$XDG_CONFIG_HOME/protonctl/`, or
//! `$PROTONCTL_CONFIG`). It holds no secrets; those live in the secret store.
//!
//! ```toml
//! time_zone = "America/Los_Angeles"   # top-level keys go before any [table]
//!
//! [mail]                               # written by `protonctl setup mail`
//! address = "you@proton.me"
//! port = 1143
//! cert_sha256 = "<pinned by setup>"
//!
//! [drive]                              # written by `protonctl setup drive`; Drive is off without it
//! # folder = "/Users/you/Library/CloudStorage/ProtonDrive-you@proton.me-folder"
//! # cli = "/Users/you/bin/proton-drive"  # on Linux its SHA-256 is pinned in the secret store
//! exclude = ["/Private"]
//!
//! [[calendar]]
//! id = "personal"
//! name = "Personal"
//!
//! [export]                             # optional: where export tools write
//! folder = "/Users/you/protonctl-export"
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::digest::Sha256;

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// IANA zone for calendar output and floating times. Defaults to this computer's zone.
    pub time_zone: Option<String>,
    pub mail: Option<MailConfig>,
    pub drive: Option<DriveConfig>,
    #[serde(default)]
    pub calendar: Vec<CalendarConfig>,
    pub export: Option<ExportConfig>,
}

/// The export folder (RFC R10's one exception): the only place outside the
/// download folder that protonctl writes Proton content to, and never cleans.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportConfig {
    /// An absolute path, outside the Proton Drive app's folder and the download cache.
    pub folder: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailConfig {
    /// The IMAP username Bridge shows for the account.
    pub address: String,
    /// Bridge's IMAP port.
    #[serde(default = "default_imap_port")]
    pub port: u16,
    /// SHA-256 of Bridge's TLS certificate, pinned by `protonctl setup mail`.
    pub cert_sha256: Sha256,
}

/// Bridge's IMAP port unless set otherwise.
pub const DEFAULT_IMAP_PORT: u16 = 1143;

fn default_imap_port() -> u16 {
    DEFAULT_IMAP_PORT
}

#[derive(Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveConfig {
    /// The Proton Drive app's folder. Found automatically when omitted.
    pub folder: Option<PathBuf>,
    /// The official Proton Drive CLI. Default `~/bin/proton-drive`.
    pub cli: Option<PathBuf>,
    /// Where `setup drive` once pinned the CLI on Linux. The pin now lives
    /// in the secret store (Q33, #72), since the model can edit this file
    /// in Claude Code; a value left here is read only to say so.
    #[serde(rename = "cli_sha256")]
    pub old_cli_sha256: Option<serde::de::IgnoredAny>,
    /// Drive paths that protonctl never shows or touches, e.g. "/Private".
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarConfig {
    pub id: String,
    pub name: String,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

pub use crate::platform::cache_dir;

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("PROTONCTL_CONFIG") {
        return p.into();
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map_or_else(|| home().join(".config"), PathBuf::from)
        .join("protonctl/config.toml")
}

pub fn load() -> Result<Config> {
    let p = path();
    match read(&p)? {
        Some(text) => {
            toml::from_str(&text).with_context(|| format!("invalid config {}", p.display()))
        }
        None => Ok(Config::default()),
    }
}

/// The config file's text, or `None` when there is none yet.
fn read(p: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(p) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", p.display())),
    }
}

/// Calendar ids name Keychain items, so they stay short and plain.
pub fn check_calendar_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        bail!("calendar id must be 1 to 32 characters of a-z, 0-9 and '-'");
    }
    Ok(())
}

/// The Bridge username names a Keychain item, so it must look like an address.
pub fn check_address(address: &str) -> Result<()> {
    let ok = address.len() <= 254
        && address.contains('@')
        && !address
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '/');
    if !ok {
        bail!("--address must be the username Bridge shows for the account, like you@proton.me");
    }
    Ok(())
}

/// Append a `[[calendar]]` entry unless one with this id exists, and return
/// the name the config holds for it.
pub fn add_calendar(id: &str, name: &str) -> Result<String> {
    if let Some(c) = load()?.calendar.into_iter().find(|c| c.id == id) {
        return Ok(c.name);
    }
    append(&path(), &calendar_entry(id, name))?;
    Ok(name.to_string())
}

/// Append the `[drive]` table that `protonctl setup drive` writes: the
/// folder and CLI only when given, so the app's folder is still found
/// automatically, and an empty `exclude` to show where exclusions go.
pub fn add_drive(folder: Option<&Path>, cli: Option<&Path>) -> Result<()> {
    append(&path(), &drive_entry(folder, cli)?)
}

fn drive_entry(folder: Option<&Path>, cli: Option<&Path>) -> Result<String> {
    let mut entry = String::from("\n[drive]\n");
    for (key, value) in [("folder", folder), ("cli", cli)] {
        if let Some(v) = value {
            let v = v
                .to_str()
                .with_context(|| format!("{key} is not UTF-8: {}", v.display()))?;
            writeln!(entry, "{key} = {}", toml::Value::from(v))
                .expect("writing to a String cannot fail");
        }
    }
    entry.push_str("exclude = []\n");
    Ok(entry)
}

/// What `setup drive` changes once Drive is set up: the CLI's path when
/// `--cli` gives one, and a `cli_sha256` left from before the pin moved to
/// the secret store, which goes (Q33, #72). Nothing is written when
/// neither applies.
pub fn set_cli(cli: Option<&Path>) -> Result<()> {
    let p = path();
    let text = read(&p)?.with_context(|| format!("there is no config at {}", p.display()))?;
    let new =
        with_cli(&text, cli).with_context(|| format!("edit [drive] in {} by hand", p.display()))?;
    if new != text {
        replace(&p, &new)?;
    }
    Ok(())
}

/// `text` with `cli` in `[drive]` set to `cli` when given, and without
/// `cli_sha256`, edited as TOML, so comments, layout and every other value
/// stay as they were. The result must read back as the same config but for
/// those two, or it is refused.
fn with_cli(text: &str, cli: Option<&Path>) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    let drive = doc
        .get_mut("drive")
        .and_then(toml_edit::Item::as_table_like_mut)
        .context("there is no [drive] table")?;
    drive.remove("cli_sha256");
    if let Some(cli) = cli {
        let cli = cli
            .to_str()
            .with_context(|| format!("cli is not UTF-8: {}", cli.display()))?;
        drive.insert("cli", toml_edit::value(cli));
    }
    let out = doc.to_string();
    let mut wanted: Config = toml::from_str(text)?;
    if let Some(drive) = wanted.drive.as_mut() {
        drive.old_cli_sha256 = None;
        if let Some(cli) = cli {
            drive.cli = Some(cli.to_path_buf());
        }
    }
    let after: Config = toml::from_str(&out).context("the edited config does not parse")?;
    anyhow::ensure!(
        after == wanted,
        "the edit changed more than cli and cli_sha256"
    );
    Ok(out)
}

/// Append the `[mail]` table that `protonctl setup mail` writes.
pub fn add_mail(address: &str, port: u16, cert_sha256: Sha256) -> Result<()> {
    append(
        &path(),
        &format!(
            "\n[mail]\naddress = {}\nport = {port}\ncert_sha256 = \"{cert_sha256}\"\n",
            toml::Value::from(address),
        ),
    )
}

fn append(p: &Path, entry: &str) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = read(p)?.unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(entry);
    replace(p, &text)
}

/// Write `text` as the file at `p` in one step: into a new file beside the
/// one it replaces, then renamed over it, so a failed write (a full disk,
/// say) leaves the old config whole. A symlink stays one: the file it
/// points to is replaced, keeping its permissions.
fn replace(p: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    let target = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = target.parent().context("the config path has no folder")?;
    let mut new = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("cannot write in {}", dir.display()))?;
    new.write_all(text.as_bytes())?;
    new.as_file().sync_all()?;
    if let Ok(old) = std::fs::metadata(&target) {
        new.as_file().set_permissions(old.permissions())?;
    }
    new.persist(&target)
        .with_context(|| format!("cannot write {}", target.display()))?;
    Ok(())
}

/// The `[[calendar]]` table `add_calendar` appends, with both values TOML-escaped.
fn calendar_entry(id: &str, name: &str) -> String {
    format!(
        "\n[[calendar]]\nid = {}\nname = {}\n",
        toml::Value::from(id),
        toml::Value::from(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_names_read_back_exactly() {
        for name in [
            "Personal",
            "a \"b\"",
            "it's",
            "back\\slash",
            "two\nlines",
            "[[calendar]]",
        ] {
            let text = format!("time_zone = \"UTC\"\n{}", calendar_entry("personal", name));
            let back: Config = toml::from_str(&text).unwrap();
            assert_eq!(back.calendar.len(), 1, "{text}");
            assert_eq!(back.calendar[0].name, name, "{text}");
        }
    }

    #[test]
    fn the_drive_table_reads_back_exactly() {
        let bare: Config = toml::from_str(&drive_entry(None, None).unwrap()).unwrap();
        let drive = bare.drive.unwrap();
        assert!(drive.folder.is_none() && drive.cli.is_none() && drive.exclude.is_empty());
        let (folder, cli) = (
            Path::new("/Users/a \"b\"/Drive"),
            Path::new("/opt/it's/proton-drive"),
        );
        let text = drive_entry(Some(folder), Some(cli)).unwrap();
        assert!(!text.contains("cli_sha256"), "{text}");
        let both: Config = toml::from_str(&text).unwrap();
        let drive = both.drive.unwrap();
        assert_eq!(drive.folder.as_deref(), Some(folder), "{text}");
        assert_eq!(drive.cli.as_deref(), Some(cli), "{text}");
    }

    /// Q33, #72: `setup drive --cli` changes the path in place, a pin left
    /// from before goes, and nothing else changes.
    #[test]
    fn a_cli_change_edits_only_cli_and_the_old_pin() {
        let old = Sha256::of(b"0.8.0");
        let text = format!(
            "time_zone = \"UTC\"\n\n[drive] # set up\ncli = \"/opt/proton-drive\"\n  cli_sha256 = \"{old}\"\nexclude = [\"/Private\"]\n{}",
            calendar_entry("cli_sha256", "cli_sha256 = x")
        );
        // An old pin is still read, and only so `setup drive` can say so.
        let before: Config = toml::from_str(&text).unwrap();
        assert!(before.drive.unwrap().old_cli_sha256.is_some());
        let edited = with_cli(&text, Some(Path::new("/opt/it's/proton-drive"))).unwrap();
        assert!(!edited.contains(&old.to_string()), "{edited}");
        assert!(edited.contains("[drive] # set up"), "{edited}");
        let cfg: Config = toml::from_str(&edited).unwrap();
        let drive = cfg.drive.unwrap();
        assert_eq!(drive.old_cli_sha256, None);
        assert_eq!(
            drive.cli.as_deref(),
            Some(Path::new("/opt/it's/proton-drive"))
        );
        assert_eq!(drive.exclude, ["/Private"]);
        assert_eq!(cfg.calendar[0].id, "cli_sha256");
        assert_eq!(cfg.time_zone.as_deref(), Some("UTC"));
        // A name that holds a `[drive]` line is a multi-line string, and
        // stays one; the line-by-line edit once wrote the pin into it.
        let tricky = "Team\n[drive]\ncli_sha256 = \"x\"\nnotes";
        let text = format!(
            "[drive]\ncli_sha256 = \"{old}\"\nexclude = []\n{}",
            calendar_entry("team", tricky)
        );
        let cfg: Config = toml::from_str(&with_cli(&text, None).unwrap()).unwrap();
        assert_eq!(cfg.calendar[0].name, tricky);
        let drive = cfg.drive.unwrap();
        assert_eq!((drive.old_cli_sha256, drive.cli), (None, None));
        // Nothing to change leaves the text as it was; no [drive] is refused.
        let unpinned = "[drive] # kept\nexclude = []\n";
        assert_eq!(with_cli(unpinned, None).unwrap(), unpinned);
        assert!(with_cli("time_zone = \"UTC\"\n", None).is_err());
    }

    #[test]
    fn a_config_that_cannot_be_read_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, b"time_zone = \"UTC\"\n\xff\n").unwrap();
        assert!(append(&p, &calendar_entry("personal", "Personal")).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"time_zone = \"UTC\"\n\xff\n");
        // A missing file is a new one.
        let new = dir.path().join("new/config.toml");
        append(&new, &calendar_entry("personal", "Personal")).unwrap();
        assert!(
            std::fs::read_to_string(&new)
                .unwrap()
                .contains("[[calendar]]")
        );
    }

    /// A config kept elsewhere behind a symlink, as dotfiles often are,
    /// stays behind it, with its permissions.
    #[test]
    fn a_rewrite_keeps_a_symlinked_config_and_its_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let (real, link) = (dir.path().join("real.toml"), dir.path().join("config.toml"));
        std::fs::write(&real, "old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        replace(&link, "new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn bridge_usernames_must_look_like_addresses() {
        assert!(check_address("you@proton.me").is_ok());
        assert!(check_address("you").is_err());
        assert!(check_address("a/b@proton.me").is_err());
        assert!(check_address("a b@proton.me").is_err());
    }

    #[test]
    fn calendar_ids_are_restricted() {
        assert!(check_calendar_id("personal-2").is_ok());
        assert!(check_calendar_id("../x").is_err());
        assert!(check_calendar_id("").is_err());
    }
}
