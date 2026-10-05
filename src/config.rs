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
//! # cli = "/Users/you/bin/proton-drive"
//! # cli_sha256 = "<pinned by setup on Linux>"
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
    /// On Linux, the CLI's SHA-256, pinned by `protonctl setup drive` (RFC
    /// Q33). macOS checks Proton's signature instead and ignores it.
    pub cli_sha256: Option<Sha256>,
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
/// automatically, the CLI's pin on Linux, and an empty `exclude` to show
/// where exclusions go.
pub fn add_drive(folder: Option<&Path>, cli: Option<&Path>, pin: Option<Sha256>) -> Result<()> {
    append(&path(), &drive_entry(folder, cli, pin)?)
}

fn drive_entry(folder: Option<&Path>, cli: Option<&Path>, pin: Option<Sha256>) -> Result<String> {
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
    if let Some(pin) = pin {
        writeln!(entry, "cli_sha256 = \"{pin}\"").expect("writing to a String cannot fail");
    }
    entry.push_str("exclude = []\n");
    Ok(entry)
}

/// Pin a new SHA-256 for the Drive CLI, the one value `setup drive` changes
/// once Drive is set up (Q33: each CLI update is pinned again).
pub fn set_cli_pin(pin: Sha256) -> Result<()> {
    let p = path();
    let text = read(&p)?.with_context(|| format!("there is no config at {}", p.display()))?;
    let new = with_cli_pin(&text, pin)
        .with_context(|| format!("edit cli_sha256 in [drive] of {} by hand", p.display()))?;
    replace(&p, &new)
}

/// `text` with `cli_sha256` in `[drive]` set to `pin`, edited as TOML, so
/// comments, layout and every other value stay as they were. The result
/// must read back as the same config but for the pin, or it is refused.
fn with_cli_pin(text: &str, pin: Sha256) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    doc.get_mut("drive")
        .and_then(toml_edit::Item::as_table_like_mut)
        .context("there is no [drive] table")?
        .insert("cli_sha256", toml_edit::value(pin.to_string()));
    let out = doc.to_string();
    let mut wanted: Config = toml::from_str(text)?;
    if let Some(drive) = wanted.drive.as_mut() {
        drive.cli_sha256 = Some(pin);
    }
    let after: Config = toml::from_str(&out).context("the edited config does not parse")?;
    anyhow::ensure!(after == wanted, "the edit changed more than cli_sha256");
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
        let bare: Config = toml::from_str(&drive_entry(None, None, None).unwrap()).unwrap();
        let drive = bare.drive.unwrap();
        assert!(drive.folder.is_none() && drive.cli.is_none() && drive.exclude.is_empty());
        let (folder, cli) = (
            Path::new("/Users/a \"b\"/Drive"),
            Path::new("/opt/it's/proton-drive"),
        );
        let pin = Sha256::of(b"cli");
        let text = drive_entry(Some(folder), Some(cli), Some(pin)).unwrap();
        let both: Config = toml::from_str(&text).unwrap();
        let drive = both.drive.unwrap();
        assert_eq!(drive.folder.as_deref(), Some(folder), "{text}");
        assert_eq!(drive.cli.as_deref(), Some(cli), "{text}");
        assert_eq!(drive.cli_sha256, Some(pin), "{text}");
    }

    /// Q33: a CLI update is pinned again in place, and nothing else changes.
    #[test]
    fn a_new_cli_pin_replaces_only_the_old_one() {
        let (old, new) = (Sha256::of(b"0.8.0"), Sha256::of(b"0.8.1"));
        let text = format!(
            "time_zone = \"UTC\"\n\n[drive] # set up\ncli = \"/opt/proton-drive\"\n  cli_sha256 = \"{old}\"\nexclude = [\"/Private\"]\n{}",
            calendar_entry("cli_sha256", "cli_sha256 = x")
        );
        let edited = with_cli_pin(&text, new).unwrap();
        assert!(!edited.contains(&old.to_string()), "{edited}");
        let cfg: Config = toml::from_str(&edited).unwrap();
        let drive = cfg.drive.unwrap();
        assert_eq!(drive.cli_sha256, Some(new));
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
        let cfg: Config = toml::from_str(&with_cli_pin(&text, new).unwrap()).unwrap();
        assert_eq!(cfg.calendar[0].name, tricky);
        assert_eq!(cfg.drive.unwrap().cli_sha256, Some(new));
        // A config without the pin gains one; one without [drive] is refused.
        let unpinned = "[drive]\nexclude = []\n";
        assert!(
            with_cli_pin(unpinned, new)
                .unwrap()
                .contains(&new.to_string())
        );
        assert!(with_cli_pin("time_zone = \"UTC\"\n", new).is_err());
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
