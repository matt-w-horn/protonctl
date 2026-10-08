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

/// The document readers' sandbox profile (RFC R21, Q13), for
/// `/usr/bin/sandbox-exec`, with the reader's path in place of `{reader}`.
/// `sandbox-exec` is deprecated in its manual and still enforces: measured
/// on macOS 27.0 (26A428) on 2026-10-07. Each allow is one a reader failed
/// without, starting from `(deny default)`, and nothing else is allowed: no
/// file write anywhere, no network, no other process (no fork, no exec but
/// the reader's own, no signal), and no Mach service, so no Keychain
/// (`securityd`), no Apple Events (osascript finds no application), no
/// shell, no `WindowServer`, no font or preferences daemon. `file-read*` on
/// `/` and `/System/Library`: both readers, for dyld, the frameworks, the
/// fonts and the OSA components. `/usr/bin` as a literal, and the metadata
/// of `/System` and `/System/Volumes/Data`: osascript, which otherwise
/// finds no JavaScript component. `/usr/share`: ICU's data, which
/// `JavaScriptCore` reads for text in other scripts. `/usr/lib`: no fixture
/// needs it; kept because dyld loads the libraries outside the shared cache
/// from there on demand. The `hw.` sysctls: osascript. Denied, and measured
/// harmless: `/Library/Fonts` and the user's fonts, so a PDF whose font is
/// not embedded renders with a substitute; and the display's colour
/// profile, so page images carry sRGB.
const READER_PROFILE: &str = r#"(version 1)
(deny default)
(allow process-exec (literal "{reader}"))
(allow file-read* (literal "{reader}") (literal "/") (literal "/usr/bin") (subpath "/usr/lib") (subpath "/usr/share") (subpath "/System/Library"))
(allow file-read-metadata (literal "/System") (literal "/System/Volumes/Data"))
(allow sysctl-read (sysctl-name-prefix "hw."))
"#;

/// `sandbox-exec` with the readers' profile in front of `reader`. Without
/// `unsafe`, this process cannot call `sandbox_init` itself. `sandbox-exec`
/// exits 65 when the profile does not compile and 71 when it cannot run the
/// reader (`convert::sandbox_failed`), and otherwise becomes the reader, in
/// the same process, so the reader's exit is its own.
pub fn confine(reader: &str) -> Result<Command> {
    // The path is quoted into the profile as it stands.
    ensure!(
        !reader.contains(['"', '\\']),
        "a reader's path cannot hold a quote or a backslash: {reader}"
    );
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .arg("-p")
        .arg(READER_PROFILE.replace("{reader}", reader))
        .arg(reader);
    Ok(command)
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

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    /// Set in the child that `the_sandbox_confines_a_reader` starts, with
    /// where the parent put the files the probe tries, the port it listens
    /// on, and the account of the Keychain item it made.
    const PROBE: &str = "PROTONCTL_SANDBOX_PROBE";
    const PROBE_DIR: &str = "PROTONCTL_PROBE_DIR";
    const PROBE_PORT: &str = "PROTONCTL_PROBE_PORT";
    const PROBE_ACCOUNT: &str = "PROTONCTL_PROBE_ACCOUNT";
    /// The Keychain service of the throwaway item: never protonctl's own.
    const PROBE_SERVICE: &str = "protonctl-test-sandbox-probe";

    /// Deletes the throwaway item when dropped, after a panic too.
    struct TestItem(String);

    impl Drop for TestItem {
        fn drop(&mut self) {
            secret_delete(PROBE_SERVICE, &self.0).ok();
        }
    }

    /// M4.1's exit (R21, Q13): a process under the readers' profile reaches
    /// no network, no other process and no Keychain item, writes nothing,
    /// and reads only the system's own files, yet can still run. The
    /// profile applies to a program `sandbox-exec` starts, so the probe is
    /// this test binary again, with only `sandbox_probe` selected, under
    /// `confine`. The item it tries is made here under a test service,
    /// read back here to show it is readable, and deleted after.
    #[test]
    fn the_sandbox_confines_a_reader() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("before"), "x").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _socket = std::os::unix::net::UnixListener::bind(dir.path().join("sock")).unwrap();
        let account = format!("probe-{}", std::process::id());
        // A stale one, from a run that crashed, would make the write an
        // update, which asks the user; deleting asks nothing.
        secret_delete(PROBE_SERVICE, &account).unwrap();
        secret_set(PROBE_SERVICE, &account, b"test").unwrap();
        let item = TestItem(account.clone());
        assert_eq!(
            secret_get(PROBE_SERVICE, &account).unwrap().as_deref(),
            Some(&b"test"[..])
        );
        let exe = std::env::current_exe().unwrap();
        let out = confine(exe.to_str().unwrap())
            .unwrap()
            .args(["platform::macos::tests::sandbox_probe", "--exact"])
            .args(["--include-ignored", "--nocapture", "--test-threads=1"])
            .env(PROBE, "1")
            .env(PROBE_DIR, dir.path())
            .env(PROBE_PORT, port.to_string())
            .env(PROBE_ACCOUNT, &account)
            .output()
            .unwrap();
        drop(item);
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{said}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(said.contains("1 passed"), "{said}");
    }

    #[test]
    #[ignore = "runs only as the child of the_sandbox_confines_a_reader"]
    fn sandbox_probe() {
        if std::env::var_os(PROBE).is_none() {
            return;
        }
        let dir = PathBuf::from(std::env::var_os(PROBE_DIR).unwrap());
        let port: u16 = std::env::var(PROBE_PORT).unwrap().parse().unwrap();
        let account = std::env::var(PROBE_ACCOUNT).unwrap();
        let denied = |what: &str, r: std::io::Result<()>| {
            let e = r.expect_err(what);
            assert_eq!(e.kind(), ErrorKind::PermissionDenied, "{what}: {e}");
        };
        denied(
            "TCP, to a port the parent listens on",
            std::net::TcpStream::connect(("127.0.0.1", port)).map(drop),
        );
        denied("UDP", std::net::UdpSocket::bind("127.0.0.1:0").map(drop));
        denied(
            "a Unix socket the parent listens on",
            std::os::unix::net::UnixStream::connect(dir.join("sock")).map(drop),
        );
        denied(
            "a write to the temporary folder",
            std::fs::write(dir.join("probe"), "x"),
        );
        denied(
            "a read of the temporary folder",
            std::fs::read(dir.join("before")).map(drop),
        );
        denied(
            "a read of the home folder",
            std::fs::read_dir(home()).map(drop),
        );
        // Where reads are allowed, the sandbox's refusal (EPERM) is told
        // from the sealed system volume's (EROFS), which a profile that
        // allowed writes would give.
        denied("a write to /usr/bin", std::fs::write("/usr/bin/probe", "x"));
        denied(
            "a write to /System/Library",
            std::fs::write("/System/Library/probe", "x"),
        );
        denied(
            "a chmod of the user's file",
            std::fs::set_permissions(dir.join("before"), std::fs::Permissions::from_mode(0o644)),
        );
        denied(
            "a new process",
            Command::new("/usr/bin/true").status().map(drop),
        );
        // securityd is a Mach service, and the profile names none.
        let read = secret_get(PROBE_SERVICE, &account);
        assert!(
            read.is_err(),
            "the Keychain item was read in the sandbox: {read:?}"
        );
        // What a reader needs still works: the system's fonts, frameworks
        // and libraries, its own folder, and a thread. Only the reader
        // itself is readable in `/usr/bin`, and here that is this binary.
        std::fs::read_dir("/System/Library/Fonts").unwrap();
        std::fs::metadata("/System/Library/Frameworks/PDFKit.framework").unwrap();
        std::fs::metadata("/usr/lib/dyld").unwrap();
        std::fs::metadata("/usr/bin").unwrap();
        denied(
            "a read of a program that is not the reader",
            std::fs::metadata("/usr/bin/textutil").map(drop),
        );
        assert_eq!(std::thread::spawn(|| 7).join().unwrap(), 7);
    }
}
