//! `~/.config/protonctl/config.toml` (or `$XDG_CONFIG_HOME/protonctl/`, or
//! `$PROTONCTL_CONFIG`). It holds no secrets; those live in the Keychain.
//!
//! ```toml
//! time_zone = "America/Los_Angeles"   # top-level keys go before any [table]
//!
//! [mail]                               # written by `protonctl setup mail`
//! address = "you@proton.me"
//! port = 1143
//! cert_sha256 = "<pinned by setup>"
//!
//! [drive]
//! # folder = "/Users/you/Library/CloudStorage/ProtonDrive-you@proton.me-folder"
//! exclude = ["/Private"]
//!
//! [[calendar]]
//! id = "personal"
//! name = "Personal"
//!
//! [export]                             # optional: where export tools write
//! folder = "/Users/you/protonctl-export"
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// IANA zone for calendar output and floating times. Defaults to the Mac's zone.
    pub time_zone: Option<String>,
    pub mail: Option<MailConfig>,
    pub drive: Option<DriveConfig>,
    #[serde(default)]
    pub calendar: Vec<CalendarConfig>,
    pub export: Option<ExportConfig>,
}

/// The export folder (RFC R10's one exception): the only place outside the
/// download folder that protonctl writes Proton content to, and never cleans.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportConfig {
    /// An absolute path, outside the Proton Drive app's folder and the download cache.
    pub folder: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailConfig {
    /// The IMAP username Bridge shows for the account.
    pub address: String,
    /// Bridge's IMAP port.
    #[serde(default = "default_imap_port")]
    pub port: u16,
    /// SHA-256 of Bridge's TLS certificate, pinned by `protonctl setup mail`.
    pub cert_sha256: String,
}

/// Bridge's IMAP port unless set otherwise.
pub const DEFAULT_IMAP_PORT: u16 = 1143;

fn default_imap_port() -> u16 {
    DEFAULT_IMAP_PORT
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveConfig {
    /// The Proton Drive app's folder. Found automatically when omitted.
    pub folder: Option<PathBuf>,
    /// The official Proton Drive CLI. Default `~/bin/proton-drive`.
    pub cli: Option<PathBuf>,
    /// Drive paths that protonctl never shows or touches, e.g. "/Private".
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarConfig {
    pub id: String,
    pub name: String,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// protonctl's cache folder, which holds the download folders and the CLI's lock file.
pub fn cache_dir() -> PathBuf {
    home().join("Library/Caches/protonctl")
}

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

/// Append the `[mail]` table that `protonctl setup mail` writes.
pub fn add_mail(address: &str, port: u16, cert_sha256: &str) -> Result<()> {
    append(
        &path(),
        &format!(
            "\n[mail]\naddress = {}\nport = {port}\ncert_sha256 = {}\n",
            toml::Value::from(address),
            toml::Value::from(cert_sha256)
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
    std::fs::write(p, text).with_context(|| format!("cannot write {}", p.display()))
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
