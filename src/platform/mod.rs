//! What differs between macOS and Linux (RFC section 11): one file per
//! system, chosen at compile time, each exporting the items below. The rest
//! of protonctl is the same on both.

use std::fs::Metadata;
use std::path::{Path, PathBuf};

use anyhow::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("protonctl builds on macOS and Linux only (RFC section 11)");

/// The secret store's name, for messages.
pub const SECRET_STORE: &str = imp::SECRET_STORE;

/// The secret stored for `account` under `service`, or `None` when there is none.
pub fn secret_get(service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    imp::secret_get(service, account)
}

pub fn secret_set(service: &str, account: &str, value: &[u8]) -> Result<()> {
    imp::secret_set(service, account, value)
}

/// Store `value` with `comment` as a non-secret attribute, in one update, so
/// a reader never sees the new value with the old comment.
pub fn secret_set_with_comment(
    service: &str,
    account: &str,
    value: &[u8],
    comment: &str,
) -> Result<()> {
    imp::secret_set_with_comment(service, account, value, comment)
}

/// An item's comment, read without its secret, so no approval is asked;
/// `None` when there is no such item or it has no comment.
pub fn secret_comment(service: &str, account: &str) -> Result<Option<String>> {
    imp::secret_comment(service, account)
}

/// Returns false when there was nothing to delete.
pub fn secret_delete(service: &str, account: &str) -> Result<bool> {
    imp::secret_delete(service, account)
}

/// Every account stored under `service`, sorted, without reading any secret.
pub fn secret_accounts(service: &str) -> Result<Vec<String>> {
    imp::secret_accounts(service)
}

/// protonctl's cache folder, which holds the download folders and the CLI's lock file.
pub fn cache_dir() -> PathBuf {
    imp::cache_dir()
}

/// Where the Proton Drive app keeps its folder, if this system has such a place.
pub fn cloud_storage() -> Option<PathBuf> {
    imp::cloud_storage()
}

/// Whether a file is a cloud-only placeholder whose content is not on this machine.
pub fn cloud_only(meta: &Metadata) -> bool {
    imp::cloud_only(meta)
}

/// This process's memory disk, mounted at `mount` on first use and readable
/// by this user only: where aliases mode lets the Drive CLI write a file it
/// reads (RFC R10, Q14), since the CLI writes only into a folder. Fails
/// rather than fall back to the disk. On Linux it is a folder under
/// `$XDG_RUNTIME_DIR`, which is in memory already, so `mount` is not used.
pub fn memory_disk(mount: &Path) -> Result<PathBuf> {
    imp::memory_disk(mount)
}

/// Detach the memory disk, if this process made one, with everything on it.
pub fn remove_memory_disk() {
    imp::remove_memory_disk();
}

/// The command that becomes the document reader at `reader`, confined
/// (R21): no file writes, no network, no other process, reads of the
/// system's own programs and libraries only. On Linux this process enters
/// the sandbox itself, which cannot be undone, and the command is the
/// reader; on macOS the command is `sandbox-exec` with the readers' profile,
/// in front of the reader (Q13).
pub fn confine(reader: &str) -> Result<std::process::Command> {
    imp::confine(reader)
}
