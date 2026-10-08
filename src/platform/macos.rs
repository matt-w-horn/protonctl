//! macOS: the login Keychain, `~/Library`, and File Provider placeholders.

use std::fs::Metadata;
use std::os::macos::fs::MetadataExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use core_foundation::data::CFData;
use security_framework::item::{
    ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, ItemUpdateOptions, ItemUpdateValue,
    Limit, update_item,
};
use security_framework::passwords;

use crate::config::home;

pub const SECRET_STORE: &str = "Keychain";
/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;
/// `st_flags` bit for a cloud-only (dataless) File Provider placeholder.
const SF_DATALESS: u32 = 0x4000_0000;

pub fn secret_get(service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    match passwords::get_generic_password(service, account) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(anyhow!("Keychain read of {service}/{account} failed: {e}")),
    }
}

pub fn secret_set(service: &str, account: &str, value: &[u8]) -> Result<()> {
    passwords::set_generic_password(service, account, value)
        .map_err(|e| anyhow!("Keychain write of {service}/{account} failed: {e}"))
}

/// An update when the item exists, so the value and comment change together;
/// otherwise an add.
pub fn secret_set_with_comment(
    service: &str,
    account: &str,
    value: &[u8],
    comment: &str,
) -> Result<()> {
    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::generic_password())
        .service(service)
        .account(account);
    let mut update = ItemUpdateOptions::new();
    update
        .set_value(ItemUpdateValue::Data(CFData::from_buffer(value)))
        .set_comment(comment);
    match update_item(&search, &update) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => {
            let mut add = ItemAddOptions::new(ItemAddValue::Data {
                class: ItemClass::generic_password(),
                data: CFData::from_buffer(value),
            });
            add.set_service(service)
                .set_account_name(account)
                .set_comment(comment);
            add.add()
                .map_err(|e| anyhow!("Keychain write of {service}/{account} failed: {e}"))
        }
        Err(e) => Err(anyhow!("Keychain write of {service}/{account} failed: {e}")),
    }
}

/// Reads attributes only, so no secret is loaded and macOS asks for no approval.
pub fn secret_comment(service: &str, account: &str) -> Result<Option<String>> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .account(account)
        .load_attributes(true)
        .limit(Limit::Max(1))
        .search();
    match found {
        // "icmt" is kSecAttrComment.
        Ok(items) => Ok(items
            .first()
            .and_then(security_framework::item::SearchResult::simplify_dict)
            .and_then(|mut d| d.remove("icmt"))),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(anyhow!("Keychain read of {service}/{account} failed: {e}")),
    }
}

pub fn secret_delete(service: &str, account: &str) -> Result<bool> {
    match passwords::delete_generic_password(service, account) {
        Ok(()) => Ok(true),
        Err(e) if e.code() == NOT_FOUND => Ok(false),
        Err(e) => Err(anyhow!(
            "Keychain delete of {service}/{account} failed: {e}"
        )),
    }
}

/// Reads attributes only, so no secret is loaded and macOS asks for no approval.
pub fn secret_accounts(service: &str) -> Result<Vec<String>> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .load_attributes(true)
        .limit(Limit::All)
        .search();
    match found {
        Ok(items) => {
            // "acct" is kSecAttrAccount.
            let mut accounts: Vec<String> = items
                .iter()
                .filter_map(|i| i.simplify_dict()?.remove("acct"))
                .collect();
            accounts.sort();
            Ok(accounts)
        }
        Err(e) if e.code() == NOT_FOUND => Ok(Vec::new()),
        Err(e) => Err(anyhow!("Keychain search of {service} failed: {e}")),
    }
}

pub fn cache_dir() -> PathBuf {
    home().join("Library/Caches/protonctl")
}

pub fn data_dir() -> PathBuf {
    home().join("Library/Application Support/protonctl")
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Linux has no such place, and both systems share this signature"
)]
pub fn cloud_storage() -> Option<PathBuf> {
    Some(home().join("Library/CloudStorage"))
}

pub fn cloud_only(meta: &Metadata) -> bool {
    meta.st_flags() & SF_DATALESS != 0
}

/// macOS runs PDFKit and `textutil` directly until Phase 4 picks their
/// sandbox (Q13), so `protonctl convert` is not used here yet.
pub fn sandbox() -> Result<()> {
    Err(anyhow!(
        "the sandbox for macOS's document readers is Phase 4's work (RFC-0001 Q13)"
    ))
}

/// The memory disk's size: two of the largest files `drive` reads inline
/// (64 MiB), since the next download can start before the last copy is
/// deleted, and room for HFS+'s own structures. Memory is taken only as
/// blocks are written.
const MEMORY_DISK_BYTES: u64 = 144 << 20;

/// The device node and mount point of this process's memory disk.
static MEMORY_DISK: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);

/// A system tool, run with no environment and no inherited stdio; its
/// stdout, or an error quoting its stderr.
fn run(tool: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(tool)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("cannot run {tool}"))?;
    ensure!(
        out.status.success(),
        "{tool} {} failed: {}",
        args.first().unwrap_or(&""),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A RAM disk, attached and mounted by the user without admin rights (RFC
/// Q14, checked 2026-10-04 on macOS 27): `nobrowse` keeps it out of Finder,
/// and Spotlight does not index it; `owners` makes the volume honour its 0700
/// root. The device nodes come up readable by group `staff`, which holds
/// every user, so they are made 0600 before anything is written.
pub fn memory_disk(mount: &Path) -> Result<PathBuf> {
    let mut disk = MEMORY_DISK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, at)) = disk.as_ref() {
        return Ok(at.clone());
    }
    let parent = mount
        .parent()
        .context("the memory disk's mount point has no parent")?;
    std::fs::create_dir_all(mount)?;
    std::fs::set_permissions(mount, std::fs::Permissions::from_mode(0o700))?;
    let sectors = MEMORY_DISK_BYTES / 512;
    let attached = run(
        "/usr/sbin/diskutil",
        &["image", "attach", "--noMount", &format!("ram://{sectors}")],
    )?;
    let device = attached
        .split_whitespace()
        .find(|w| w.starts_with("/dev/disk"))
        .with_context(|| format!("diskutil attached no disk: {}", attached.trim()))?
        .to_owned();
    let made = (|| -> Result<()> {
        let raw = device.replacen("/dev/disk", "/dev/rdisk", 1);
        for node in [&device, &raw] {
            std::fs::set_permissions(node, std::fs::Permissions::from_mode(0o600))?;
        }
        run("/sbin/newfs_hfs", &["-v", "protonctl", &device])?;
        let at = mount
            .to_str()
            .context("the memory disk's mount point is not UTF-8")?;
        run(
            "/usr/sbin/diskutil",
            &[
                "mount",
                "-mountOptions",
                "nobrowse,owners,noexec",
                "-mountPoint",
                at,
                &device,
            ],
        )?;
        // A mount that silently failed would leave the folder on the disk.
        if std::fs::metadata(mount)?.dev() == std::fs::metadata(parent)?.dev() {
            bail!("the memory disk is not mounted at {}", mount.display());
        }
        std::fs::set_permissions(mount, std::fs::Permissions::from_mode(0o700))?;
        Ok(())
    })();
    if let Err(e) = made {
        eject(&device);
        return Err(e);
    }
    *disk = Some((device, mount.to_path_buf()));
    Ok(mount.to_path_buf())
}

pub fn remove_memory_disk() {
    let taken = MEMORY_DISK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some((device, mount)) = taken {
        eject(&device);
        // Empty once the disk is gone; a failure here holds no content.
        std::fs::remove_dir(mount).ok();
    }
}

/// Eject `device`, forcing the unmount if a file on it is still open. A
/// failure is reported, since the disk can hold file content until restart.
fn eject(device: &str) {
    let ejected = run("/usr/sbin/diskutil", &["eject", device]).or_else(|_| {
        run("/usr/sbin/diskutil", &["unmountDisk", "force", device])?;
        run("/usr/sbin/diskutil", &["eject", device])
    });
    if let Err(e) = ejected {
        eprintln!("protonctl: cannot eject the memory disk {device}: {e:#}");
    }
}
