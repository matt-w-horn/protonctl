//! Linux: XDG folders, and no secret store or Drive app folder yet. Until
//! Phase P2 picks a secret store (RFC Q31), every service that needs a secret
//! reports that it is not available on Linux, so protonctl builds and its
//! tests run here, but it serves nothing.

use std::fs::Metadata;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::config::home;

pub const SECRET_STORE: &str = "secret store";
/// Linux readers wait for Phase P4 (RFC Q34).
pub const DOCUMENT_READERS: bool = false;

fn no_store() -> anyhow::Error {
    anyhow!("protonctl has no secret store on Linux yet (RFC-0001 section 11, Q31)")
}

pub fn secret_get(_service: &str, _account: &str) -> Result<Option<Vec<u8>>> {
    Err(no_store())
}

pub fn secret_set(_service: &str, _account: &str, _value: &[u8]) -> Result<()> {
    Err(no_store())
}

pub fn secret_set_with_comment(
    _service: &str,
    _account: &str,
    _value: &[u8],
    _comment: &str,
) -> Result<()> {
    Err(no_store())
}

pub fn secret_comment(_service: &str, _account: &str) -> Result<Option<String>> {
    Err(no_store())
}

pub fn secret_delete(_service: &str, _account: &str) -> Result<bool> {
    Err(no_store())
}

pub fn secret_accounts(_service: &str) -> Result<Vec<String>> {
    Err(no_store())
}

/// `$XDG_CACHE_HOME/protonctl`, or `~/.cache/protonctl`. The XDG Base
/// Directory specification says to ignore a relative value.
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".cache"))
        .join("protonctl")
}

/// No Proton Drive app is known for Linux, so Drive lists through the CLI.
pub fn cloud_storage() -> Option<PathBuf> {
    None
}

/// Without the app there are no File Provider placeholders.
pub fn cloud_only(_meta: &Metadata) -> bool {
    false
}
