//! Linux: the Secret Service over D-Bus (RFC Q31), XDG folders, and no
//! Drive app folder. Items carry the attributes `service` and `account`
//! (RFC lld-api, Keychain items), and `comment` where macOS has its item
//! comment. With no Secret Service running, every secret call fails, and
//! nothing is ever written to a file instead.

use std::collections::HashMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail, ensure};
use seccompiler::SeccompRule;
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
    /// Attributes only: an encrypted session's key exchange would cost a
    /// read of an item's comment more than the read itself.
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

pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
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

/// A reader's address space, in bytes (R21): pandoc's heap of 1 GiB
/// (`convert.rs`) and its program, and far more than poppler needs, so a
/// file built to exhaust memory ends the reader instead of the machine.
/// GHC reserves a terabyte only when it may; under this limit it reserves
/// less.
const READER_ADDRESS_SPACE: u64 = 2 << 30;

/// `RLIMIT_CORE` of 1, not 0: the kernel writes no core file below a page,
/// and drops a core bound for a `|` handler (systemd-coredump, apport) only
/// at exactly 1, which it keeps to stop a handler's own crash recursing. A
/// crashed reader's core holds the document. An `@` socket handler (Linux
/// 6.17 and later) is given the core whatever the limit.
const READER_CORE: u64 = 1;

/// Requests that set a file's owner, besides `F_SETOWN`, which libc names:
/// an `fcntl` one and two `ioctl` ones, the same on `x86_64` and aarch64
/// (`asm-generic/fcntl.h`, `asm-generic/sockios.h`).
const F_SETOWN_EX: u64 = 15;
const FIOSETOWN: u64 = 0x8901;
const SIOCSPGRP: u64 = 0x8902;

/// The x32 ABI's mark on an `x86_64` system call number.
const X32_SYSCALL_BIT: i64 = 0x4000_0000;

/// System calls a reader never needs, each refused outright or when its
/// rules hold: a new socket; `io_uring`, whose requests seccomp cannot see;
/// every change to a file's metadata (mode, owner, times, extended
/// attributes) or length by path, which Landlock does not cover
/// (truncation only from Linux 6.2), so a reader cannot loosen the mode of
/// a file it cannot open; a signal or a pidfd for any process but its own,
/// and a file owner, which would carry `SIGIO` to one, so before Linux 6.12,
/// where Landlock scopes signals, a reader still signals no other process;
/// a new process, which would outlive the reader's time limit; and a
/// change to any resource limit, so the reader keeps its own and sets none
/// on the user's other processes.
fn refused_calls() -> Result<Vec<(i64, Vec<SeccompRule>)>> {
    use seccompiler::SeccompCmpArgLen::{Dword, Qword};
    use seccompiler::SeccompCmpOp::{Eq, MaskedEq, Ne};
    use seccompiler::{SeccompCmpArgLen, SeccompCmpOp, SeccompCondition};

    /// The same on every architecture: calls added since Linux 5.1 share
    /// their numbers, and libc does not name these everywhere yet.
    const FCHMODAT2: i64 = 452;
    const SETXATTRAT: i64 = 463;
    const REMOVEXATTRAT: i64 = 466;
    // Refused when argument `arg` compares to `value`; `pid_t` and an
    // `int` command are 32 bits, so their upper halves are not compared.
    let when = |arg: u8, len: SeccompCmpArgLen, op: SeccompCmpOp, value: u64| {
        SeccompRule::new(vec![SeccompCondition::new(arg, len, op, value)?])
    };
    let own = u64::from(rustix::process::getpid().as_raw_pid().cast_unsigned());
    let other_process = vec![when(0, Dword, Ne, own)?];
    let mut calls: Vec<(i64, Vec<SeccompRule>)> = [
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
        // Its target is a descriptor, which a filter cannot read.
        libc::SYS_pidfd_send_signal,
        libc::SYS_setrlimit,
    ]
    .into_iter()
    .map(|call| (call, Vec::new()))
    .collect();
    // `raise`, `abort` and a thread's signals name this process, whose pid
    // `exec` keeps.
    calls.extend(
        [
            libc::SYS_kill,
            libc::SYS_tkill,
            libc::SYS_tgkill,
            libc::SYS_rt_sigqueueinfo,
            libc::SYS_rt_tgsigqueueinfo,
            libc::SYS_pidfd_open,
        ]
        .map(|call| (call, other_process.clone())),
    );
    let thread = u64::from(libc::CLONE_THREAD.cast_unsigned());
    let owner = |cmd: u64| when(1, Dword, Eq, cmd);
    calls.extend([
        // A thread, which has `CLONE_THREAD`, is no new process.
        (libc::SYS_clone, vec![when(0, Qword, MaskedEq(thread), 0)?]),
        // Reading a limit passes no new one.
        (libc::SYS_prlimit64, vec![when(2, Qword, Ne, 0)?]),
        (
            libc::SYS_fcntl,
            vec![
                owner(libc::F_SETOWN.cast_unsigned().into())?,
                owner(F_SETOWN_EX)?,
            ],
        ),
        (libc::SYS_ioctl, vec![owner(FIOSETOWN)?, owner(SIOCSPGRP)?]),
    ]);
    // x86_64 also keeps the older calls that aarch64 replaced with the
    // `*at` forms and `clone` above.
    #[cfg(target_arch = "x86_64")]
    calls.extend(
        [
            libc::SYS_chmod,
            libc::SYS_chown,
            libc::SYS_lchown,
            libc::SYS_utime,
            libc::SYS_utimes,
            libc::SYS_futimesat,
            libc::SYS_fork,
            libc::SYS_vfork,
        ]
        .map(|call| (call, Vec::new())),
    );
    Ok(calls)
}

/// A call's number in the x32 ABI. Most take their `x86_64` number with
/// the x32 mark; these three are 64-bit only there, and x32 has its own
/// (`arch/x86/entry/syscalls/syscall_64.tbl`).
fn x32(call: i64) -> i64 {
    X32_SYSCALL_BIT
        | match call {
            libc::SYS_ioctl => 514,
            libc::SYS_rt_sigqueueinfo => 524,
            libc::SYS_rt_tgsigqueueinfo => 536,
            other => other,
        }
}

/// The resource limits first, while they can still be set: an address
/// space and a core size. Then Landlock, which also sets no-new-privileges,
/// which seccomp needs. Landlock confines files, TCP, abstract Unix sockets
/// and signals as far as this kernel supports, and must hold at least for
/// files; it also keeps the reader from tracing any process outside the
/// sandbox. seccomp refuses the calls `refused_calls` names, so no network
/// and no Unix socket (the session bus, say) is reached and no other
/// process is signalled, whatever the kernel's Landlock covers; a 32-bit
/// call ends the process. A second filter answers `clone3`, whose flags it
/// cannot read, as a kernel without it would, so libc starts a thread with
/// `clone`, which the first filter reads.
fn sandbox() -> Result<()> {
    use landlock::{
        ABI, Access as _, AccessFs, AccessNet, Ruleset, RulesetAttr as _, RulesetCreatedAttr as _,
        RulesetStatus, Scope, path_beneath_rules,
    };
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

    for (resource, most) in [
        (Resource::As, READER_ADDRESS_SPACE),
        (Resource::Core, READER_CORE),
    ] {
        // Never above a hard limit already set, which only root may raise.
        let most = getrlimit(resource)
            .maximum
            .map_or(most, |hard| hard.min(most));
        setrlimit(
            resource,
            Rlimit {
                current: Some(most),
                maximum: Some(most),
            },
        )?;
    }
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
    for filter in filters()? {
        seccompiler::apply_filter(&filter)?;
    }
    Ok(())
}

/// The two seccomp filters `sandbox` applies, in order: the calls
/// `refused_calls` names, answered `EACCES`, and `clone3`, answered
/// `ENOSYS`. Each ends the process for a call from another architecture,
/// such as a 32-bit call.
fn filters() -> Result<Vec<seccompiler::BpfProgram>> {
    use seccompiler::{SeccompAction, SeccompFilter};
    let mut refused = refused_calls()?;
    let mut absent = vec![(libc::SYS_clone3, Vec::new())];
    // The filter matches call numbers exactly, and on x86_64 a kernel that
    // enables the x32 ABI also takes each call by its x32 number, which the
    // check of the architecture does not catch.
    if cfg!(target_arch = "x86_64") {
        for calls in [&mut refused, &mut absent] {
            let x32: Vec<_> = calls.iter().map(|(c, r)| (x32(*c), r.clone())).collect();
            calls.extend(x32);
        }
    }
    let arch = std::env::consts::ARCH.try_into()?;
    [(refused, libc::EACCES), (absent, libc::ENOSYS)]
        .into_iter()
        .map(|(calls, errno)| {
            Ok(SeccompFilter::new(
                calls.into_iter().collect(),
                SeccompAction::Allow,
                SeccompAction::Errno(errno.unsigned_abs()),
                arch,
            )?
            .try_into()?)
        })
        .collect()
}

/// Enter the sandbox here, in the process that will become `reader`.
pub fn confine(reader: &str) -> Result<std::process::Command> {
    sandbox()?;
    Ok(std::process::Command::new(reader))
}

/// Run by `scripts/check.sh` in a private D-Bus session with a throwaway
/// GNOME Keyring, under their own service name.
#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

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
    /// prompt dismissed, and none waits (R8). A first write unlocks as a read does, rather
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

    /// Set in the child that `the_sandbox_confines_a_reader` starts.
    const PROBE: &str = "PROTONCTL_SANDBOX_PROBE";

    /// A rustix call's outcome as the standard library's.
    fn errno<T>(r: rustix::io::Result<T>) -> std::io::Result<()> {
        r.map(drop).map_err(std::io::Error::from)
    }

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
        // No signal reaches another process on any kernel, nor a new
        // process start, nor a limit change (R21); signal 0 sends
        // nothing.
        denied(
            "kill(0, 0), to this process group",
            errno(rustix::process::test_kill_current_process_group()),
        );
        denied(
            "a pidfd for the parent",
            errno(rustix::process::pidfd_open(
                rustix::process::getppid().unwrap(),
                rustix::process::PidfdFlags::empty(),
            )),
        );
        denied(
            "a new process",
            std::process::Command::new("/usr/bin/true")
                .status()
                .map(drop),
        );
        let core = rustix::process::Rlimit {
            current: Some(0),
            maximum: Some(0),
        };
        denied(
            "a core size of 0",
            errno(rustix::process::setrlimit(
                rustix::process::Resource::Core,
                core,
            )),
        );
        // What a reader needs still works: its own libraries, a signal to
        // itself, a thread, and its limits, which are the readers'.
        std::fs::read("/etc/ld.so.cache").unwrap();
        rustix::process::test_kill_process(rustix::process::getpid()).unwrap();
        assert_eq!(std::thread::spawn(|| 7).join().unwrap(), 7);
        for (resource, limit) in [
            (rustix::process::Resource::As, READER_ADDRESS_SPACE),
            (rustix::process::Resource::Core, READER_CORE),
        ] {
            let set = rustix::process::getrlimit(resource);
            assert_eq!((set.current, set.maximum), (Some(limit), Some(limit)));
        }
    }

    /// Set in the child that `the_sandbox_lets_no_signal_out` starts.
    const SIGNAL_PROBE: &str = "PROTONCTL_SIGNAL_PROBE";

    /// The calls `sandbox_probe` cannot make without `unsafe`, made by
    /// perl, which Debian and Ubuntu always install: each signal call and
    /// pidfd for the parent, a file owner that would send it `SIGIO`, a
    /// fork and a limit change are refused; the same signals to itself are
    /// not. Signal 0 sends nothing. Arguments are `name=number`.
    const SIGNAL_SCRIPT: &str = r#"
use strict;
use Errno qw(EACCES);
$| = 1;
# Numbers: syscall passes a string as a pointer.
my %nr = map { my ($k, $v) = split /=/; ($k, 0 + $v) } @ARGV;
my ($other, $own, $bad) = (getppid(), $$, 0);
my $info = "\0" x 128;
my $limit = pack("QQ", 0, 0);
pipe(my $r, my $w) or die "pipe: $!";
my $fd = fileno($r);
sub refused {
    my ($what, $got) = @_;
    if ($got == -1 && $! == EACCES) { print "refused: $what\n" }
    else { print "NOT REFUSED: $what ($got, $!)\n"; $bad = 1 }
}
sub allowed {
    my ($what, $got) = @_;
    if ($got != -1) { print "allowed: $what\n" }
    else { print "NOT ALLOWED: $what ($!)\n"; $bad = 1 }
}
refused("kill", syscall($nr{kill}, $other, 0));
refused("kill(-1)", syscall($nr{kill}, -1, 0));
refused("tkill", syscall($nr{tkill}, $other, 0));
refused("tgkill", syscall($nr{tgkill}, $other, $other, 0));
refused("rt_sigqueueinfo", syscall($nr{rt_sigqueueinfo}, $other, 0, $info));
refused("rt_tgsigqueueinfo", syscall($nr{rt_tgsigqueueinfo}, $other, $other, 0, $info));
refused("pidfd_open", syscall($nr{pidfd_open}, $other, 0));
refused("fcntl(F_SETOWN)", syscall($nr{fcntl}, $fd, $nr{F_SETOWN}, $other));
refused("fcntl(F_SETOWN_EX)", syscall($nr{fcntl}, $fd, $nr{F_SETOWN_EX}, pack("ii", 1, $other)));
refused("ioctl(FIOSETOWN)", syscall($nr{ioctl}, $fd, $nr{FIOSETOWN}, pack("i", $other)));
refused("ioctl(SIOCSPGRP)", syscall($nr{ioctl}, $fd, $nr{SIOCSPGRP}, pack("i", $other)));
refused("prlimit64, to set", syscall($nr{prlimit64}, 0, 4, $limit, 0));
refused("setrlimit", syscall($nr{setrlimit}, 4, $limit));
my $pid = fork();
if (defined $pid && $pid == 0) { require POSIX; POSIX::_exit(0) }
refused("fork", defined $pid ? 0 : -1);
allowed("kill to itself", syscall($nr{kill}, $own, 0));
allowed("tgkill to itself", syscall($nr{tgkill}, $own, $own, 0));
allowed("prlimit64, to read", syscall($nr{prlimit64}, $other, 4, 0, $limit));
my $pidfd = syscall($nr{pidfd_open}, $own, 0);
allowed("pidfd_open of itself", $pidfd);
refused("pidfd_send_signal", syscall($nr{pidfd_send_signal}, $pidfd, 0, 0, 0));
exit $bad;
"#;

    /// R21: before Linux 6.12 Landlock does not scope signals, so the
    /// seccomp filter alone keeps a reader from signalling the user's
    /// other processes; this checks the filter's rules on any kernel. The
    /// probe enters the sandbox, then becomes perl.
    #[test]
    fn the_sandbox_lets_no_signal_out() {
        let out = std::process::Command::new("/proc/self/exe")
            .args(["platform::linux::tests::signal_probe", "--exact"])
            .args(["--include-ignored", "--nocapture", "--test-threads=1"])
            .env(SIGNAL_PROBE, "1")
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{said}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(said.matches("refused: ").count(), 15, "{said}");
        assert_eq!(said.matches("allowed: ").count(), 4, "{said}");
    }

    #[test]
    #[ignore = "runs only as the child of the_sandbox_lets_no_signal_out"]
    fn signal_probe() {
        use std::os::unix::process::CommandExt as _;
        if std::env::var_os(SIGNAL_PROBE).is_none() {
            return;
        }
        let nr = [
            ("kill", libc::SYS_kill),
            ("tkill", libc::SYS_tkill),
            ("tgkill", libc::SYS_tgkill),
            ("rt_sigqueueinfo", libc::SYS_rt_sigqueueinfo),
            ("rt_tgsigqueueinfo", libc::SYS_rt_tgsigqueueinfo),
            ("pidfd_open", libc::SYS_pidfd_open),
            ("pidfd_send_signal", libc::SYS_pidfd_send_signal),
            ("fcntl", libc::SYS_fcntl),
            ("ioctl", libc::SYS_ioctl),
            ("prlimit64", libc::SYS_prlimit64),
            ("setrlimit", libc::SYS_setrlimit),
            ("F_SETOWN", libc::F_SETOWN.into()),
        ]
        .map(|(name, nr)| format!("{name}={nr}"))
        .into_iter()
        .chain(
            [
                ("F_SETOWN_EX", F_SETOWN_EX),
                ("FIOSETOWN", FIOSETOWN),
                ("SIOCSPGRP", SIOCSPGRP),
            ]
            .map(|(name, nr)| format!("{name}={nr}")),
        );
        sandbox().unwrap();
        let err = std::process::Command::new("/usr/bin/perl")
            .args(["-e", SIGNAL_SCRIPT])
            .args(nr)
            .env_clear()
            .exec();
        panic!("cannot run perl: {err}");
    }

    /// What seccomp does with one call: each filter run over the call's
    /// `struct seccomp_data` (`linux/seccomp.h`: the number, the
    /// architecture, the instruction pointer, six arguments), newest filter
    /// first, and the result whose action comes first in the kernel's order
    /// kept (`seccomp_run_filters`). Only the instructions seccompiler
    /// emits are run; any other fails the test.
    fn seccomp(filters: &[seccompiler::BpfProgram], arch: u32, nr: i64, args: [u64; 6]) -> u32 {
        let mut data = [0u8; 64];
        data[..4].copy_from_slice(&i32::try_from(nr).unwrap().to_ne_bytes());
        data[4..8].copy_from_slice(&arch.to_ne_bytes());
        for (i, arg) in args.iter().enumerate() {
            data[16 + 8 * i..24 + 8 * i].copy_from_slice(&arg.to_ne_bytes());
        }
        let run = |filter: &seccompiler::BpfProgram| {
            let (mut a, mut pc) = (0u32, 0usize);
            loop {
                let op = &filter[pc];
                let (code, k) = (u32::from(op.code), op.k);
                let jump = |taken: bool| usize::from(if taken { op.jt } else { op.jf });
                pc += 1;
                match code {
                    c if c == libc::BPF_LD | libc::BPF_W | libc::BPF_ABS => {
                        let at = usize::try_from(k).unwrap();
                        a = u32::from_ne_bytes(data[at..at + 4].try_into().unwrap());
                    }
                    c if c == libc::BPF_ALU | libc::BPF_AND | libc::BPF_K => a &= k,
                    c if c == libc::BPF_JMP | libc::BPF_JA => pc += usize::try_from(k).unwrap(),
                    c if c == libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K => pc += jump(a == k),
                    c if c == libc::BPF_JMP | libc::BPF_JGT | libc::BPF_K => pc += jump(a > k),
                    c if c == libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K => pc += jump(a >= k),
                    c if c == libc::BPF_RET | libc::BPF_K => return k,
                    other => panic!("an instruction seccompiler does not emit: {other:#x}"),
                }
            }
        };
        // The kernel compares actions as signed numbers, so killing the
        // process comes first and allowing last.
        let action = |ret: u32| (ret & libc::SECCOMP_RET_ACTION_FULL).cast_signed();
        filters
            .iter()
            .rev()
            .map(run)
            .fold(libc::SECCOMP_RET_ALLOW, |kept, ret| {
                if action(ret) < action(kept) {
                    ret
                } else {
                    kept
                }
            })
    }

    /// T14: the filters `sandbox` applies, run as seccomp runs them,
    /// refuse each call by its `x86_64` number and by its x32 number, allow
    /// what a reader needs, and end the process for a 32-bit call. The
    /// numbers are written out from the kernel's
    /// `arch/x86/entry/syscalls/syscall_64.tbl` (v6.18), not taken from
    /// `x32` or `refused_calls`, so a wrong mapping or a dropped rule there
    /// fails here. That seccomp sees the x32 number before the kernel
    /// looks for an x32 table is the kernel's ABI (`seccomp(2)`, "the
    /// x32 ABI").
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_filters_refuse_each_call_by_both_its_numbers() {
        /// `linux/audit.h`.
        const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
        const AUDIT_ARCH_I386: u32 = 0x4000_0003;
        /// The x32 mark, added to the table's x32 number.
        const X32: i64 = 0x4000_0000;
        let errno = |e: i32| libc::SECCOMP_RET_ERRNO | e.unsigned_abs();
        let (refused, absent) = (errno(libc::EACCES), errno(libc::ENOSYS));
        let allowed = libc::SECCOMP_RET_ALLOW;
        let filters = filters().unwrap();
        let own = u64::from(rustix::process::getpid().as_raw_pid().cast_unsigned());
        let other = own + 1;
        let check = |numbers: [i64; 2], args: [u64; 6], want: u32| {
            for nr in numbers {
                let got = seccomp(&filters, AUDIT_ARCH_X86_64, nr, args);
                assert_eq!(got, want, "call {nr:#x} with {args:?}");
            }
        };
        // Refused whatever their arguments: the x86_64 number, then the
        // x32 one.
        for numbers in [
            [41, X32 + 41],   // socket
            [425, X32 + 425], // io_uring_setup
            [91, X32 + 91],   // fchmod
            [268, X32 + 268], // fchmodat
            [452, X32 + 452], // fchmodat2
            [93, X32 + 93],   // fchown
            [260, X32 + 260], // fchownat
            [280, X32 + 280], // utimensat
            [188, X32 + 188], // setxattr
            [189, X32 + 189], // lsetxattr
            [190, X32 + 190], // fsetxattr
            [463, X32 + 463], // setxattrat
            [197, X32 + 197], // removexattr
            [198, X32 + 198], // lremovexattr
            [199, X32 + 199], // fremovexattr
            [466, X32 + 466], // removexattrat
            [76, X32 + 76],   // truncate
            [424, X32 + 424], // pidfd_send_signal
            [160, X32 + 160], // setrlimit
            [90, X32 + 90],   // chmod
            [92, X32 + 92],   // chown
            [94, X32 + 94],   // lchown
            [132, X32 + 132], // utime
            [235, X32 + 235], // utimes
            [261, X32 + 261], // futimesat
            [57, X32 + 57],   // fork
            [58, X32 + 58],   // vfork
        ] {
            check(numbers, [0; 6], refused);
        }
        // A signal or a pidfd for another process; the same for its own.
        for numbers in [
            [62, X32 + 62],   // kill
            [200, X32 + 200], // tkill
            [234, X32 + 234], // tgkill
            [129, X32 + 524], // rt_sigqueueinfo
            [297, X32 + 536], // rt_tgsigqueueinfo
            [434, X32 + 434], // pidfd_open
        ] {
            check(numbers, [other, 0, 0, 0, 0, 0], refused);
            check(numbers, [own, 0, 0, 0, 0, 0], allowed);
        }
        // A new process, not a thread.
        let thread = u64::from(libc::CLONE_THREAD.cast_unsigned());
        check([56, X32 + 56], [0; 6], refused); // clone
        check([56, X32 + 56], [thread, 0, 0, 0, 0, 0], allowed);
        // A new limit, not a read of one.
        check([302, X32 + 302], [0, 4, 1, 0, 0, 0], refused); // prlimit64
        check([302, X32 + 302], [0, 4, 0, 1, 0, 0], allowed);
        // A file owner, which would carry SIGIO to another process.
        let set_own = u64::from(libc::F_SETOWN.cast_unsigned());
        let get_fl = u64::from(libc::F_GETFL.cast_unsigned());
        for cmd in [set_own, F_SETOWN_EX] {
            check([72, X32 + 72], [3, cmd, 0, 0, 0, 0], refused); // fcntl
        }
        check([72, X32 + 72], [3, get_fl, 0, 0, 0, 0], allowed);
        for request in [FIOSETOWN, SIOCSPGRP] {
            check([16, X32 + 514], [3, request, 0, 0, 0, 0], refused); // ioctl
        }
        check([16, X32 + 514], [3, libc::FIONREAD, 0, 0, 0, 0], allowed);
        // clone3 answered as a kernel without it; getpid, which a reader
        // needs, allowed.
        check([435, X32 + 435], [0; 6], absent);
        check([39, X32 + 39], [0; 6], allowed);
        // A 32-bit call ends the process, whatever its number.
        for nr in [1, 37, 41] {
            let got = seccomp(&filters, AUDIT_ARCH_I386, nr, [0; 6]);
            assert_eq!(got, libc::SECCOMP_RET_KILL_PROCESS, "i386 call {nr}");
        }
    }

    #[test]
    fn unit_tests_never_reach_the_user_s_items() {
        let err = secret_get("protonctl", "drive-cli-pin").unwrap_err();
        assert!(err.to_string().contains("never reach"), "{err}");
    }
}
