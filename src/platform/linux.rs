//! Linux: the Secret Service over D-Bus (RFC Q31), XDG folders, and no
//! Drive app folder. Items carry the attributes `service` and `account`
//! (RFC lld-api, Keychain items), and `comment` where macOS has its item
//! comment. With no Secret Service running, every secret call fails, and
//! nothing is ever written to a file instead.

use std::collections::HashMap;
use std::fs::Metadata;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use secret_service::EncryptionType;
use secret_service::blocking::{Collection, Item, SecretService};

use crate::config::home;

pub const SECRET_STORE: &str = "Secret Service";

/// Every protonctl value is text.
const CONTENT_TYPE: &str = "text/plain";
/// The only service unit tests may reach, in the private D-Bus session that
/// `scripts/check.sh` starts; other tests never touch the user's keyring.
const TEST_SERVICE: &str = "protonctl-test";

/// What crosses the D-Bus session in a call.
#[derive(Clone, Copy, Debug)]
enum Carries {
    /// A secret: the session is encrypted, as libsecret's are by default.
    Secret,
    /// Attributes only. The per-call reads of the privacy setting are these,
    /// and an encrypted session's key exchange would cost each call more
    /// than the call itself.
    Attributes,
}

/// Run `f` against the Secret Service on a thread of its own: zbus's
/// blocking calls start their own runtime, which tokio forbids on a thread
/// that is running async code, and some callers are.
fn with_store<T: Send>(
    service: &str,
    what: &str,
    carries: Carries,
    f: impl FnOnce(&SecretService<'_>) -> Result<T> + Send,
) -> Result<T> {
    if cfg!(test) && service != TEST_SERVICE {
        bail!("unit tests never reach the user's secret store");
    }
    std::thread::scope(|s| {
        s.spawn(|| {
            let encryption = match carries {
                Carries::Secret => EncryptionType::Dh,
                Carries::Attributes => EncryptionType::Plain,
            };
            let store = SecretService::connect(encryption).map_err(|e| {
                anyhow!(
                    "cannot reach the Secret Service ({e}); protonctl keeps secrets only there, \
                     so start GNOME Keyring or KWallet in this session (RFC-0001 Q31)"
                )
            })?;
            f(&store)
        })
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
    .map_err(|e| e.context(format!("Secret Service {what} failed")))
}

fn attributes<'a>(service: &'a str, account: &'a str) -> HashMap<&'a str, &'a str> {
    HashMap::from([("service", service), ("account", account)])
}

/// The items whose attributes match, unlocked: unlocking a collection can
/// show the desktop's prompt for the keyring password.
fn search<'a>(store: &'a SecretService<'a>, attrs: HashMap<&str, &str>) -> Result<Vec<Item<'a>>> {
    let found = store.search_items(attrs)?;
    let mut items = found.unlocked;
    if !found.locked.is_empty() {
        store.unlock_all(&found.locked.iter().collect::<Vec<_>>())?;
        items.extend(found.locked);
    }
    Ok(items)
}

/// The default collection, unlocked as `search` unlocks items: a locked
/// collection refuses a new item without showing the unlock prompt.
fn unlocked_default<'a>(store: &'a SecretService<'a>) -> Result<Collection<'a>> {
    let collection = store.get_default_collection()?;
    if collection.is_locked()? {
        collection.unlock()?;
    }
    Ok(collection)
}

/// The one item for `account`, or `None`. Two would leave a reader to pick
/// one, so that is an error.
fn one<'a>(store: &'a SecretService<'a>, service: &str, account: &str) -> Result<Option<Item<'a>>> {
    let mut items = search(store, attributes(service, account))?;
    if items.len() > 1 {
        bail!(
            "{} items are stored for {service}/{account}; delete them with `protonctl logout` and set it up again",
            items.len()
        );
    }
    Ok(items.pop())
}

pub fn secret_get(service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    with_store(service, "read", Carries::Secret, |store| {
        one(store, service, account)?
            .map(|item| item.get_secret().map_err(anyhow::Error::from))
            .transpose()
    })
}

pub fn secret_set(service: &str, account: &str, value: &[u8]) -> Result<()> {
    with_store(service, "write", Carries::Secret, |store| {
        match one(store, service, account)? {
            Some(item) => item.set_secret(value, CONTENT_TYPE)?,
            None => {
                unlocked_default(store)?.create_item(
                    &format!("{service}/{account}"),
                    attributes(service, account),
                    value,
                    false,
                    CONTENT_TYPE,
                )?;
            }
        }
        Ok(())
    })
}

/// A new item is made with its comment in one call. The Secret Service has
/// no single update of a secret and its attributes, so an existing item
/// takes the value first and the comment second: a reader between the two
/// sees the new value under the old comment, never the reverse, and
/// `KeyWatch` reads the comment again before calling that a mismatch.
pub fn secret_set_with_comment(
    service: &str,
    account: &str,
    value: &[u8],
    comment: &str,
) -> Result<()> {
    with_store(service, "write", Carries::Secret, |store| {
        let mut attrs = attributes(service, account);
        attrs.insert("comment", comment);
        match one(store, service, account)? {
            Some(item) => {
                item.set_secret(value, CONTENT_TYPE)?;
                item.set_attributes(attrs)?;
            }
            None => {
                unlocked_default(store)?.create_item(
                    &format!("{service}/{account}"),
                    attrs,
                    value,
                    false,
                    CONTENT_TYPE,
                )?;
            }
        }
        Ok(())
    })
}

/// Reads attributes only, never the secret.
pub fn secret_comment(service: &str, account: &str) -> Result<Option<String>> {
    with_store(service, "read", Carries::Attributes, |store| {
        Ok(match one(store, service, account)? {
            Some(item) => item.get_attributes()?.remove("comment"),
            None => None,
        })
    })
}

pub fn secret_delete(service: &str, account: &str) -> Result<bool> {
    with_store(service, "delete", Carries::Attributes, |store| {
        let items = search(store, attributes(service, account))?;
        for item in &items {
            item.delete()?;
        }
        Ok(!items.is_empty())
    })
}

/// Reads attributes only, never a secret.
pub fn secret_accounts(service: &str) -> Result<Vec<String>> {
    with_store(service, "search", Carries::Attributes, |store| {
        let mut accounts = Vec::new();
        for item in search(store, HashMap::from([("service", service)]))? {
            accounts.extend(item.get_attributes()?.remove("account"));
        }
        accounts.sort();
        accounts.dedup();
        Ok(accounts)
    })
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

/// What a document reader may read: programs, libraries, the loader's cache
/// and font configuration, and nothing of the user's or the machine's own.
const READER_READS: [&str; 7] = [
    "/usr",
    "/lib",
    "/lib64",
    "/bin",
    "/etc/ld.so.cache",
    "/etc/fonts",
    "/var/cache/fontconfig",
];

/// System calls a reader never needs, refused by number: a new socket;
/// `io_uring`, whose requests seccomp cannot see; and every change to a
/// file's metadata (mode, owner, times, extended attributes) or length by
/// path, which Landlock does not cover (truncation only from Linux 6.2),
/// so a reader cannot loosen the mode of a file it cannot open.
fn refused_calls() -> Vec<i64> {
    /// The same on every architecture: calls added since Linux 5.1 share
    /// their numbers, and libc does not name these everywhere yet.
    const FCHMODAT2: i64 = 452;
    const SETXATTRAT: i64 = 463;
    const REMOVEXATTRAT: i64 = 466;
    let mut calls = vec![
        libc::SYS_socket,
        libc::SYS_io_uring_setup,
        libc::SYS_fchmod,
        libc::SYS_fchmodat,
        FCHMODAT2,
        libc::SYS_fchown,
        libc::SYS_fchownat,
        libc::SYS_utimensat,
        libc::SYS_setxattr,
        libc::SYS_lsetxattr,
        libc::SYS_fsetxattr,
        SETXATTRAT,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
        REMOVEXATTRAT,
        libc::SYS_truncate,
    ];
    // x86_64 also keeps the older calls that aarch64 replaced with the
    // `*at` forms above.
    #[cfg(target_arch = "x86_64")]
    calls.extend([
        libc::SYS_chmod,
        libc::SYS_chown,
        libc::SYS_lchown,
        libc::SYS_utime,
        libc::SYS_utimes,
        libc::SYS_futimesat,
    ]);
    calls
}

/// The x32 ABI's mark on an `x86_64` system call number.
const X32_SYSCALL_BIT: i64 = 0x4000_0000;

/// Landlock first, which also sets no-new-privileges, which seccomp needs.
/// Landlock confines files, TCP, abstract Unix sockets and signals as far
/// as this kernel supports, and must hold at least for files; it also keeps
/// the reader from tracing any process outside the sandbox. seccomp refuses
/// every new socket and `io_uring`, whose requests seccomp cannot see, so no
/// network and no Unix socket (the session bus, say) is reached, whatever
/// the kernel's Landlock covers; a 32-bit call ends the process.
pub fn sandbox() -> Result<()> {
    use landlock::{
        ABI, Access as _, AccessFs, AccessNet, Ruleset, RulesetAttr as _, RulesetCreatedAttr as _,
        RulesetStatus, Scope, path_beneath_rules,
    };
    use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};

    let abi = ABI::V9;
    let reads = READER_READS.iter().filter(|p| Path::new(p).exists());
    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))?
        .handle_access(AccessNet::from_all(abi))?
        .scope(Scope::from_all(abi))?
        .create()?
        .add_rules(path_beneath_rules(reads, AccessFs::from_read(abi)))?
        // Readers open it for their own output.
        .add_rules(path_beneath_rules(["/dev/null"], AccessFs::from_all(abi)))?
        .restrict_self()?;
    ensure!(
        status.ruleset != RulesetStatus::NotEnforced,
        "this kernel does not enforce Landlock, which the document readers need (RFC-0001 R21)"
    );
    let mut calls = refused_calls();
    // The filter matches call numbers exactly, and on x86_64 a kernel that
    // enables the x32 ABI also takes each call with this bit set, which the
    // check of the architecture does not catch.
    if cfg!(target_arch = "x86_64") {
        let x32: Vec<i64> = calls.iter().map(|call| call | X32_SYSCALL_BIT).collect();
        calls.extend(x32);
    }
    let refused = calls.into_iter().map(|call| (call, Vec::new())).collect();
    let filter: BpfProgram = SeccompFilter::new(
        refused,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EACCES.unsigned_abs()),
        std::env::consts::ARCH.try_into()?,
    )?
    .try_into()?;
    seccompiler::apply_filter(&filter)?;
    Ok(())
}

/// `statfs`'s type for tmpfs, a file system in memory and swap.
const TMPFS_MAGIC: u64 = 0x0102_1994;

/// This process's folder under `$XDG_RUNTIME_DIR`, once made.
static MEMORY: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Linux needs no disk of its own: `$XDG_RUNTIME_DIR` is in memory already,
/// once `memory_base` has checked it, so this makes a 0700 folder there for
/// the process and `mount`, where macOS mounts its RAM disk, is not used.
pub fn memory_disk(_mount: &Path) -> Result<PathBuf> {
    let base = memory_base()?;
    let dir = base.join(format!("protonctl-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    sweep_memory(&base, std::time::SystemTime::now());
    *MEMORY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(dir.clone());
    Ok(dir)
}

/// Delete this process's folder in memory, with everything in it. A failure
/// is reported, since the folder can hold file content until logout.
pub fn remove_memory_disk() {
    let taken = MEMORY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(dir) = taken
        && let Err(e) = std::fs::remove_dir_all(&dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("protonctl: cannot delete {}: {e}", dir.display());
    }
}

/// `$XDG_RUNTIME_DIR`, once it is on tmpfs, only this user can open it, and
/// every active swap is encrypted or in memory (R10, Q14).
fn memory_base() -> Result<PathBuf> {
    let dir = if cfg!(test) {
        // /dev/shm is tmpfs too, and the tests' environment has no session.
        let dir = PathBuf::from(format!("/dev/shm/protonctl-test-{}", own_uid()?));
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        dir
    } else {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .context("$XDG_RUNTIME_DIR is not set, so there is no folder in memory for the Drive CLI to write to")?
    };
    private_memory(&dir)?;
    // The test machine's own swap is not the tests' to judge; the rules are
    // tested on a synthetic sysfs instead.
    let swaps = if cfg!(test) {
        String::new()
    } else {
        std::fs::read_to_string("/proc/swaps").context("cannot read /proc/swaps")?
    };
    private_swap(&swaps, Path::new("/sys"), device_of)?;
    Ok(dir)
}

/// How long a dead process's folder is left, in case its process ID is not
/// visible here (another PID namespace sharing the folder).
const MEMORY_LEFTOVER_AGE: std::time::Duration = std::time::Duration::from_mins(10);

/// Delete the `protonctl-<pid>` folders in `base` whose process is gone and
/// that no one has changed for `MEMORY_LEFTOVER_AGE`: what a server killed
/// before it could clean up left, which can hold a file it was reading.
fn sweep_memory(base: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("protonctl-"))
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        let alive = Path::new("/proc").join(pid.to_string()).exists();
        let old = std::fs::symlink_metadata(entry.path())
            .ok()
            .filter(std::fs::Metadata::is_dir)
            .and_then(|m| m.modified().ok())
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > MEMORY_LEFTOVER_AGE);
        if !alive && old {
            std::fs::remove_dir_all(entry.path()).ok();
        }
    }
}

/// The user this process runs as: the owner of its own `/proc` entry.
fn own_uid() -> Result<u32> {
    Ok(std::fs::metadata("/proc/self")?.uid())
}

fn private_memory(dir: &Path) -> Result<()> {
    let meta = std::fs::metadata(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    ensure!(meta.is_dir(), "{} is not a folder", dir.display());
    ensure!(
        meta.uid() == own_uid()?,
        "{} belongs to another user",
        dir.display()
    );
    // The group's and others' permission bits.
    let shared = meta.mode() & 0o077;
    ensure!(
        shared == 0,
        "other users can open {} (mode {:o})",
        dir.display(),
        meta.mode() & 0o777
    );
    let fs = rustix::fs::statfs(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    ensure!(
        u64::try_from(fs.f_type) == Ok(TMPFS_MAGIC),
        "{} is not on tmpfs, a file system in memory",
        dir.display()
    );
    Ok(())
}

/// The device, as major and minor numbers, that a swap entry lives on.
fn device_of(name: &Path, kind: &str) -> Result<(u32, u32)> {
    let meta =
        std::fs::metadata(name).with_context(|| format!("cannot read {}", name.display()))?;
    let dev = match kind {
        "partition" => meta.rdev(),
        "file" => meta.dev(),
        other => bail!("swap {} is of the unknown type {other}", name.display()),
    };
    let dev = (rustix::fs::major(dev), rustix::fs::minor(dev));
    if kind == "partition" || Path::new(&format!("/sys/dev/block/{}:{}", dev.0, dev.1)).exists() {
        return Ok(dev);
    }
    // A file system with no block device of its own, as btrfs, reports an
    // anonymous one; its mount names the real device. btrfs keeps a swap
    // file only on a single-device file system, so that is the one.
    let info = std::fs::read_to_string("/proc/self/mountinfo")?;
    let source = mount_source(&info, dev)
        .with_context(|| format!("no mount names the device under swap {}", name.display()))?;
    let rdev = std::fs::metadata(&source)
        .with_context(|| format!("cannot read {}", source.display()))?
        .rdev();
    Ok((rustix::fs::major(rdev), rustix::fs::minor(rdev)))
}

/// The source of the mount whose device is `dev`, from `/proc/self/mountinfo`
/// ("id parent major:minor root point options ... - type source options").
fn mount_source(mountinfo: &str, dev: (u32, u32)) -> Option<PathBuf> {
    let wanted = format!("{}:{}", dev.0, dev.1);
    mountinfo.lines().find_map(|line| {
        let (head, tail) = line.split_once(" - ")?;
        (head.split(' ').nth(2)? == wanted).then_some(())?;
        let source = tail.split(' ').nth(1)?;
        source
            .starts_with('/')
            .then(|| PathBuf::from(unescape(source)))
    })
}

/// Every swap in `/proc/swaps` must be in memory (zram) or on dm-crypt at
/// some depth below it (LVM on LUKS, say), so the pages of a file read into
/// tmpfs never reach a disk in clear (R10). `sys` is where sysfs is.
fn private_swap(
    swaps: &str,
    sys: &Path,
    device_of: impl Fn(&Path, &str) -> Result<(u32, u32)>,
) -> Result<()> {
    for line in swaps.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(kind)) = (fields.next(), fields.next()) else {
            continue;
        };
        let name = PathBuf::from(unescape(name));
        let (major, minor) = device_of(&name, kind)?;
        ensure!(
            private_device(sys, &format!("{major}:{minor}")),
            "swap {} is not encrypted, so a file read into memory could reach the disk in clear; \
             encrypt it or use zram (RFC-0001 R10)",
            name.display()
        );
    }
    Ok(())
}

/// Whether the block device `dev` ("major:minor") is zram, dm-crypt, or
/// built only on devices that are.
fn private_device(sys: &Path, dev: &str) -> bool {
    let block = sys.join("dev/block").join(dev);
    let name = std::fs::read_link(&block)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    // zram compresses into memory, unless it writes pages back to a disk.
    let zram = name.is_some_and(|n| n.starts_with("zram"))
        && std::fs::read_to_string(block.join("backing_dev")).map_or(true, |d| d.trim() == "none");
    if zram || std::fs::read_to_string(block.join("dm/uuid")).is_ok_and(|u| encrypting(&u)) {
        return true;
    }
    let Ok(below) = std::fs::read_dir(block.join("slaves")) else {
        return false;
    };
    let mut any = false;
    for entry in below.flatten() {
        let dev =
            std::fs::read_to_string(sys.join("class/block").join(entry.file_name()).join("dev"));
        if !dev.is_ok_and(|d| private_device(sys, d.trim())) {
            return false;
        }
        any = true;
    }
    any
}

/// Whether a device-mapper UUID is cryptsetup's for a device that encrypts:
/// "CRYPT-<type>-...". Its integrity-only and verity types also start with
/// "CRYPT-" and encrypt nothing.
fn encrypting(uuid: &str) -> bool {
    const ENCRYPTING: [&str; 7] = [
        "LUKS1", "LUKS2", "PLAIN", "LOOPAES", "TCRYPT", "BITLK", "FVAULT2",
    ];
    uuid.strip_prefix("CRYPT-")
        .and_then(|rest| rest.split('-').next())
        .is_some_and(|kind| ENCRYPTING.contains(&kind))
}

/// A name from `/proc/swaps`, where a space, tab, newline or backslash is
/// written as a backslash and three octal digits.
fn unescape(name: &str) -> String {
    let mut out = Vec::with_capacity(name.len());
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let code = name
            .get(i + 1..i + 4)
            .filter(|_| bytes[i] == b'\\')
            .and_then(|o| u8::from_str_radix(o, 8).ok());
        if let Some(c) = code {
            out.push(c);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Run by `scripts/check.sh` in a private D-Bus session with a throwaway
/// GNOME Keyring, under their own service name.
#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = TEST_SERVICE;

    #[test]
    #[ignore = "needs a Secret Service; scripts/check.sh runs it in a private D-Bus session"]
    fn the_secret_service_keeps_items_values_and_comments() {
        for account in secret_accounts(S).unwrap() {
            secret_delete(S, &account).unwrap();
        }
        assert_eq!(secret_get(S, "a").unwrap(), None);
        assert_eq!(secret_comment(S, "a").unwrap(), None);
        assert!(!secret_delete(S, "a").unwrap());

        secret_set(S, "a", b"one").unwrap();
        secret_set(S, "a", b"two").unwrap();
        assert_eq!(secret_get(S, "a").unwrap().as_deref(), Some(&b"two"[..]));

        secret_set_with_comment(S, "k", b"key 1", "id 1").unwrap();
        assert_eq!(secret_comment(S, "k").unwrap().as_deref(), Some("id 1"));
        secret_set_with_comment(S, "k", b"key 2", "id 2").unwrap();
        // One item, updated in place: a second would make these refuse.
        assert_eq!(secret_comment(S, "k").unwrap().as_deref(), Some("id 2"));
        assert_eq!(secret_get(S, "k").unwrap().as_deref(), Some(&b"key 2"[..]));
        assert_eq!(secret_accounts(S).unwrap(), ["a", "k"]);

        assert!(secret_delete(S, "a").unwrap());
        assert_eq!(secret_get(S, "a").unwrap(), None);
        assert_eq!(secret_accounts(S).unwrap(), ["k"]);
        secret_delete(S, "k").unwrap();
    }

    /// Some callers read secrets on tokio's async threads, where zbus's own
    /// runtime would panic without a thread of its own.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a Secret Service; scripts/check.sh runs it in a private D-Bus session"]
    async fn the_secret_service_answers_on_an_async_thread() {
        secret_set(S, "async", b"x").unwrap();
        assert_eq!(secret_get(S, "async").unwrap().as_deref(), Some(&b"x"[..]));
        assert!(secret_delete(S, "async").unwrap());
    }

    /// Set in the child that `a_locked_keyring_fails_at_once_without_a_prompt`
    /// starts.
    const LOCKED: &str = "PROTONCTL_LOCKED_PROBE";

    /// With the collection locked and no unlock prompt able to show, as in a
    /// session with no display, every read and write fails at once with the
    /// prompt dismissed, and none waits; the privacy setting is read this way
    /// on every call (R8, R26). A first write unlocks as a read does, rather
    /// than failing on the locked collection without a prompt. A collection
    /// cannot be unlocked again without the prompt, so the probe runs in a
    /// keyring of its own: this test binary again, with only `locked_probe`
    /// selected, in a private D-Bus session with no display and its own
    /// home and runtime folders. Run with `--nocapture` to see each call's
    /// time.
    #[test]
    #[ignore = "needs dbus-run-session and gnome-keyring-daemon; scripts/check.sh runs it"]
    fn a_locked_keyring_fails_at_once_without_a_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("dbus-run-session");
        child
            .arg("--")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/with-keyring.sh"
            ))
            .arg(std::env::current_exe().unwrap())
            .args(["platform::linux::tests::locked_probe", "--exact"])
            .args(["--include-ignored", "--nocapture", "--test-threads=1"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .envs(["HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR"].map(|k| (k, dir.path())))
            .env(LOCKED, "1");
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            child.env("LLVM_PROFILE_FILE", profile);
        }
        let out = child.output().unwrap();
        let (said, logged) = (
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        eprint!("{logged}");
        assert!(out.status.success(), "{said}{logged}");
        assert!(said.contains("1 passed"), "{said}");
    }

    #[test]
    #[ignore = "runs only as the child of a_locked_keyring_fails_at_once_without_a_prompt"]
    fn locked_probe() {
        use std::time::{Duration, Instant};
        if std::env::var_os(LOCKED).is_none() {
            return;
        }
        secret_set(S, "kept", b"x").unwrap();
        with_store(S, "lock", Carries::Attributes, |store| {
            Ok(store.get_default_collection()?.lock()?)
        })
        .unwrap();
        let calls: [(&str, &dyn Fn() -> Result<()>); 7] = [
            ("a read", &|| secret_get(S, "kept").map(drop)),
            ("a comment's read", &|| secret_comment(S, "kept").map(drop)),
            ("a search", &|| secret_accounts(S).map(drop)),
            ("a delete", &|| secret_delete(S, "kept").map(drop)),
            ("a change", &|| secret_set(S, "kept", b"y")),
            ("a first write", &|| secret_set(S, "new", b"y")),
            ("a first write with a comment", &|| {
                secret_set_with_comment(S, "new", b"y", "c")
            }),
        ];
        for (what, call) in calls {
            let started = Instant::now();
            let err = call().expect_err(what);
            let took = started.elapsed();
            eprintln!("{what} failed after {} ms: {err:#}", took.as_millis());
            assert!(
                took < Duration::from_secs(10),
                "{what} took {} ms",
                took.as_millis()
            );
            let said = format!("{err:#}");
            assert!(said.contains("prompt dismissed"), "{what}: {said}");
        }
    }

    /// A sysfs tree: zram0 (252:0); zram1 (252:1), with a backing disk;
    /// sda2 (8:2), a plain partition; dm-0 (253:0), LUKS on sda3; dm-1
    /// (253:1), LVM on dm-0; dm-2 (253:2), LVM on dm-0 and sda2; dm-3 and
    /// dm-4 (253:3, 253:4), integrity-only and verity on sda3.
    fn sysfs() -> tempfile::TempDir {
        let sys = tempfile::tempdir().unwrap();
        let block = sys.path().join("dev/block");
        let class = sys.path().join("class/block");
        std::fs::create_dir_all(&block).unwrap();
        // zram0 keeps its pages in memory; zram1 writes them back to sdb1.
        for (zram, backing) in [("0", "none\n"), ("1", "/dev/sdb1\n")] {
            let dir = sys.path().join(format!("devices/virtual/block/zram{zram}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("backing_dev"), backing).unwrap();
            let target = format!("../../devices/virtual/block/zram{zram}");
            std::os::unix::fs::symlink(target, block.join(format!("252:{zram}"))).unwrap();
        }
        for (dev, name, uuid, below) in [
            ("8:2", "sda2", None, &[][..]),
            ("253:0", "dm-0", Some("CRYPT-LUKS2-abc-luks"), &["sda3"][..]),
            ("253:1", "dm-1", Some("LVM-xyz"), &["dm-0"][..]),
            ("253:2", "dm-2", Some("LVM-xyz2"), &["dm-0", "sda2"][..]),
            // cryptsetup's integrity-only and verity devices encrypt nothing.
            (
                "253:3",
                "dm-3",
                Some("CRYPT-INTEGRITY-abc-int"),
                &["sda3"][..],
            ),
            ("253:4", "dm-4", Some("CRYPT-VERITY-abc-ver"), &["sda3"][..]),
        ] {
            let d = block.join(dev);
            std::fs::create_dir_all(d.join("slaves")).unwrap();
            for b in below {
                std::fs::create_dir_all(d.join("slaves").join(b)).unwrap();
            }
            if let Some(uuid) = uuid {
                std::fs::create_dir_all(d.join("dm")).unwrap();
                std::fs::write(d.join("dm/uuid"), uuid).unwrap();
            }
            std::fs::create_dir_all(class.join(name)).unwrap();
            std::fs::write(class.join(name).join("dev"), format!("{dev}\n")).unwrap();
        }
        std::fs::create_dir_all(class.join("sda3")).unwrap();
        std::fs::write(class.join("sda3/dev"), "8:3\n").unwrap();
        sys
    }

    /// R10: a file read into memory may reach only swap that is encrypted
    /// or in memory, at every level below it.
    #[test]
    fn only_encrypted_or_memory_swap_passes() {
        let sys = sysfs();
        let header = "Filename\tType\tSize\tUsed\tPriority\n";
        let swaps = |rows: &[&str]| format!("{header}{}", rows.join("\n"));
        let devices = |name: &Path, _: &str| -> Result<(u32, u32)> {
            Ok(match name.to_str().unwrap() {
                "/dev/zram0" => (252, 0),
                "/dev/zram1" => (252, 1),
                "/dev/dm-3" => (253, 3),
                "/dev/dm-4" => (253, 4),
                "/dev/sda2" => (8, 2),
                "/dev/dm-0" => (253, 0),
                "/swap file" => (253, 1),
                "/dev/dm-2" => (253, 2),
                other => panic!("{other}"),
            })
        };
        let ok = |rows: &[&str]| private_swap(&swaps(rows), sys.path(), devices).is_ok();
        assert!(ok(&[]));
        assert!(ok(&["/dev/zram0 partition 8G 0 100"]));
        assert!(ok(&["/dev/dm-0 partition 8G 0 -2"]));
        // A file on LVM on LUKS; its name holds an escaped space.
        assert!(ok(&[
            "/swap\\040file file 8G 0 -2",
            "/dev/zram0 partition 8G 0 100"
        ]));
        assert!(!ok(&["/dev/sda2 partition 8G 0 -2"]));
        assert!(!ok(&[
            "/dev/zram0 partition 8G 0 100",
            "/dev/sda2 partition 8G 0 -2"
        ]));
        // LVM across an encrypted and a plain device.
        assert!(!ok(&["/dev/dm-2 partition 8G 0 -2"]));
        // zram that writes pages back to a disk, and dm devices that only
        // check what they hold.
        assert!(!ok(&["/dev/zram1 partition 8G 0 100"]));
        assert!(!ok(&["/dev/dm-3 partition 8G 0 -2"]));
        assert!(!ok(&["/dev/dm-4 partition 8G 0 -2"]));
    }

    /// A swap file on btrfs reports an anonymous device; its mount names the
    /// real one.
    #[test]
    fn a_mount_names_the_device_under_an_anonymous_one() {
        let info = "29 1 0:26 / / rw,relatime shared:1 - btrfs /dev/mapper/root rw,ssd\n\
                    30 29 0:27 / /home rw shared:2 - btrfs /dev/mapper/a\\040b rw\n\
                    31 29 0:28 / /run rw shared:3 - tmpfs tmpfs rw\n";
        assert_eq!(
            mount_source(info, (0, 26)),
            Some(PathBuf::from("/dev/mapper/root"))
        );
        assert_eq!(
            mount_source(info, (0, 27)),
            Some(PathBuf::from("/dev/mapper/a b"))
        );
        // A source that is not a path, and a device no mount has.
        assert_eq!(mount_source(info, (0, 28)), None);
        assert_eq!(mount_source(info, (0, 99)), None);
    }

    /// A folder whose process is gone is deleted once no one has changed it
    /// for a while; a live process's folder, a fresh one and other names stay.
    #[test]
    fn a_dead_process_s_memory_folder_is_swept() {
        let base = tempfile::tempdir().unwrap();
        // Above the kernel's largest process ID, so never alive.
        let dead = base.path().join(format!("protonctl-{}", u32::MAX));
        let alive = base
            .path()
            .join(format!("protonctl-{}", std::process::id()));
        let other = base.path().join("protonctl-notes");
        for dir in [&dead, &alive, &other] {
            std::fs::create_dir_all(dir.join("drive-x")).unwrap();
        }
        let now = std::time::SystemTime::now();
        sweep_memory(base.path(), now);
        assert!(dead.exists(), "a fresh folder was swept");
        sweep_memory(base.path(), now + std::time::Duration::from_hours(1));
        assert!(!dead.exists(), "the dead process's folder stayed");
        assert!(alive.exists() && other.exists());
    }

    #[test]
    fn swap_names_are_unescaped() {
        assert_eq!(unescape("/a\\040b\\134c"), "/a b\\c");
        assert_eq!(unescape("/plain"), "/plain");
        assert_eq!(unescape("/end\\04"), "/end\\04");
    }

    /// The folder must be on tmpfs and private to this user.
    #[test]
    fn the_memory_folder_must_be_tmpfs_and_private() {
        let dir = memory_base().unwrap();
        private_memory(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = private_memory(&dir).unwrap_err().to_string();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(err.contains("other users can open"), "{err}");
        let disk = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        std::fs::set_permissions(disk.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let err = private_memory(disk.path()).unwrap_err().to_string();
        assert!(err.contains("not on tmpfs"), "{err}");
    }

    /// Set in the child that `the_sandbox_confines_a_reader` starts.
    const PROBE: &str = "PROTONCTL_SANDBOX_PROBE";

    /// MP4's exit (R21): a process in the readers' sandbox reaches no network
    /// and no other process, writes nothing, and reads only the system's
    /// programs and libraries, yet can still run a reader. The sandbox cannot
    /// be undone, so the probe runs in a child: this test binary again, with
    /// only `sandbox_probe` selected.
    #[test]
    fn the_sandbox_confines_a_reader() {
        let out = std::process::Command::new("/proc/self/exe")
            .args(["platform::linux::tests::sandbox_probe", "--exact"])
            .args(["--include-ignored", "--nocapture", "--test-threads=1"])
            .env(PROBE, "1")
            .output()
            .unwrap();
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
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let tmp = std::env::temp_dir();
        let before = tempfile::NamedTempFile::new_in(&tmp).unwrap();
        sandbox().unwrap();
        let denied = |what: &str, r: std::io::Result<()>| {
            let e = r.expect_err(what);
            assert_eq!(
                e.kind(),
                std::io::ErrorKind::PermissionDenied,
                "{what}: {e}"
            );
        };
        denied("TCP", std::net::TcpStream::connect("127.0.0.1:9").map(drop));
        denied("UDP", std::net::UdpSocket::bind("127.0.0.1:0").map(drop));
        denied(
            "a Unix socket",
            std::os::unix::net::UnixDatagram::unbound().map(drop),
        );
        denied("a write to /tmp", std::fs::write(tmp.join("probe"), "x"));
        denied("a read of /tmp", std::fs::read(before.path()).map(drop));
        denied(
            "a read of the home folder",
            std::fs::read_dir(&home).map(drop),
        );
        denied("a read of /etc", std::fs::read("/etc/hostname").map(drop));
        denied("a read of /proc", std::fs::read("/proc/1/status").map(drop));
        denied("a write to /usr", std::fs::write("/usr/probe", "x"));
        // Landlock leaves a file's metadata alone; seccomp refuses it.
        denied(
            "a chmod of the user's file",
            std::fs::set_permissions(before.path(), std::fs::Permissions::from_mode(0o644)),
        );
        let (uid, gid) = {
            let m = before.as_file().metadata().unwrap();
            (m.uid(), m.gid())
        };
        denied(
            "a chown of the user's file",
            std::os::unix::fs::chown(before.path(), Some(uid), Some(gid)),
        );
        // What a reader needs still works.
        std::fs::read("/etc/ld.so.cache").unwrap();
        assert!(
            std::process::Command::new("/usr/bin/true")
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn unit_tests_never_reach_the_user_s_items() {
        let err = secret_get("protonctl", "privacy-mode").unwrap_err();
        assert!(err.to_string().contains("never reach"), "{err}");
    }
}
