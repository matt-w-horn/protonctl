//! The official Proton Drive CLI. Before every run protonctl copies the
//! binary, through one open of its path, into a private copy, checks that
//! copy, and runs that copy (RFC R9, Q24, #72): on macOS the copy must be
//! signed by Proton's Apple team; on Linux, where
//! Proton publishes no signature or checksum, it must have the SHA-256 that
//! `setup drive` pinned in the secret store (Q33). Its version is checked
//! once per process, since `--version` also asks proton.me for updates (RFC
//! Appendix A).
//! Downloads go through it, and so do listing and stat when the Proton Drive
//! app's folder is absent. Every run holds a lock file, so two protonctl
//! processes (the Claude app's server and Claude Code's) never run it at once.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::File;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::{DriveConfig, cache_dir, home};
use crate::content::clean;
use crate::digest::Sha256;
use crate::secret::{self, Account};

/// Proton AG's Apple Developer team, shared by Proton Drive.app and the CLI.
const PROTON_TEAM: &str = "2SB5Z68H26";
/// The CLI minor series whose behaviour protonctl was written against.
const SUPPORTED: &str = "0.8.";
/// The name each run of the copy is given as its `argv[0]`.
const NAME: &str = "proton-drive";

pub fn path(cfg: Option<&DriveConfig>) -> PathBuf {
    cfg.and_then(|c| c.cli.clone())
        .unwrap_or_else(|| home().join("bin/proton-drive"))
}

/// The pin `setup drive` stored, read from the secret store item's comment
/// without loading its secret, as the privacy mode is (Q28, Q33).
pub fn stored_pin() -> Result<Option<Sha256>> {
    secret::comment(&Account::DriveCliPin)?
        .as_deref()
        .map(str::parse)
        .transpose()
}

/// Pin `pin`, as `setup drive` does once the user has confirmed it.
pub fn store_pin(pin: Sha256) -> Result<()> {
    let text = pin.to_string();
    secret::set_with_comment(
        &Account::DriveCliPin,
        &secret::Secret::from(text.clone()),
        &text,
    )
}

/// The CLI's "Node not found", so a missing path can read the same as an
/// excluded one (RFC R7).
#[derive(Debug)]
pub struct NotFound;

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the Proton Drive CLI found no such path")
    }
}

impl std::error::Error for NotFound {}

/// The CLI's whole environment. It takes its API host, credential store and
/// cache mode from `PROTON_DRIVE_*` variables (RFC Appendix A) and logs at
/// DEBUG unless told otherwise, so nothing else is passed on.
fn environment() -> [(&'static str, std::ffi::OsString); 2] {
    [
        ("HOME", home().into_os_string()),
        ("PROTON_DRIVE_LOG_LEVEL", "ERROR".into()),
    ]
}

/// One path for every protonctl process, in the cache folder in both modes
/// (RFC Q37): the per-process memory folder would lock nothing.
fn lock_path() -> PathBuf {
    // Unit tests share the real HOME; keep them out of it, as the downloads do.
    if cfg!(test) {
        std::env::temp_dir().join("protonctl-test/cli.lock")
    } else {
        cache_dir().join("cli.lock")
    }
}

/// Wait for an exclusive lock on the file at `path`, held until the returned
/// handle drops. The CLI fails with "database is locked" when two runs meet
/// (RFC principle 6), and the lock covers every protonctl process, not just
/// this one. It blocks, so async callers take it off the async threads.
fn lock(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("cannot lock {}", path.display()))?;
    Ok(file)
}

/// The CLI as one call checks and runs it: its bytes, read through one open
/// of its path into a private copy, so the copy that is checked is the one
/// that runs, for `--version` and the command alike (Q24, #72). A swap of
/// the file after the check changes nothing until the next call checks it
/// again. It is made and dropped under the lock.
struct PrivateCopy {
    /// What to run.
    path: PathBuf,
    /// The SHA-256 of the bytes copied.
    sha256: Sha256,
    /// Linux: a sealed memfd, open read-only, which `path` names as
    /// `/proc/self/fd/N`. No process can change it, and it is close-on-exec:
    /// `execve` opens it by that path, and no child inherits it.
    #[cfg(target_os = "linux")]
    _fd: File,
    /// Elsewhere: a 0700 folder holding the copy, removed on drop.
    #[cfg(not(target_os = "linux"))]
    _dir: tempfile::TempDir,
}

impl PrivateCopy {
    /// Copy the CLI into a memfd, then seal it against every change.
    #[cfg(target_os = "linux")]
    fn of(cli: &Path) -> Result<Self> {
        use rustix::fs::{MemfdFlags, SealFlags};
        use std::os::fd::AsRawFd as _;
        let mut from = File::open(cli).with_context(|| format!("cannot read {}", cli.display()))?;
        let fd = rustix::fs::memfd_create(NAME, MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)
            .context("cannot make a memfd for the Proton Drive CLI")?;
        let mut copy = File::from(fd);
        let sha256 = Sha256::of_copy(&mut from, &mut copy)
            .with_context(|| format!("cannot copy {}", cli.display()))?;
        rustix::fs::fcntl_add_seals(
            &copy,
            SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL,
        )
        .context("cannot seal the Proton Drive CLI's copy")?;
        // execve(2) documents ETXTBSY for a file open for writing, so only a
        // read-only handle stays open.
        let fd = File::open(format!("/proc/self/fd/{}", copy.as_raw_fd()))
            .context("cannot reopen the Proton Drive CLI's copy")?;
        drop(copy);
        Ok(Self {
            path: format!("/proc/self/fd/{}", fd.as_raw_fd()).into(),
            sha256,
            _fd: fd,
        })
    }

    /// Copy the CLI into a new folder only this user can open. Its code
    /// signature is inside the Mach-O file, so the copy keeps it. Another
    /// process of this user could still change the copy, unlike a sealed
    /// memfd, but must first find the folder's random name.
    #[cfg(not(target_os = "linux"))]
    fn of(cli: &Path) -> Result<Self> {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        let mut from = File::open(cli).with_context(|| format!("cannot read {}", cli.display()))?;
        let dir = tempfile::Builder::new()
            .prefix("protonctl-cli-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .context("cannot make a folder for the Proton Drive CLI's copy")?;
        let path = dir.path().join(NAME);
        let mut copy = File::options()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        let sha256 = Sha256::of_copy(&mut from, &mut copy)
            .with_context(|| format!("cannot copy {}", cli.display()))?;
        drop(copy);
        Ok(Self {
            path,
            sha256,
            _dir: dir,
        })
    }

    /// A run of the copy, under the name the CLI has, with only
    /// `environment()` and an empty stdin, so a prompt fails instead of
    /// waiting.
    fn command(&self) -> Command {
        let mut c = Command::new(&self.path);
        c.arg0(NAME)
            .env_clear()
            .envs(environment())
            .stdin(Stdio::null());
        c
    }
}

/// What shows that a file is Proton's CLI (R9).
#[derive(Clone, Debug)]
enum Proof {
    /// macOS: signed by Proton's Apple team.
    Signature,
    /// Linux: this SHA-256, which `setup drive` pinned in the secret store
    /// (Q33), as read when the process started or just confirmed at setup;
    /// else why there is none.
    Pinned(Result<Sha256, String>),
    /// A stand-in the tests build.
    #[cfg(test)]
    Trusted,
}

impl Proof {
    /// Check the copy of the CLI at `cli`. It blocks, so async callers run
    /// it off the async threads.
    fn check(&self, copy: &PrivateCopy, cli: &Path) -> Result<()> {
        match self {
            Self::Signature => signed_by_proton(&copy.path, cli),
            Self::Pinned(pin) => matches_pin(copy.sha256, pin.as_ref(), cli),
            #[cfg(test)]
            Self::Trusted => Ok(()),
        }
    }

    /// The pin as the secret store held it, or why there is none (Q33): a
    /// pin left in the config from before is not trusted, since the model
    /// can edit that file in Claude Code (#72).
    fn pinned(stored: Result<Option<Sha256>>, cfg: Option<&DriveConfig>) -> Self {
        let item = secret::shown(&Account::DriveCliPin);
        Self::Pinned(match stored {
            Ok(Some(pin)) => Ok(pin),
            Ok(None) => {
                let mut why = format!(
                    "no SHA-256 is pinned for the Proton Drive CLI in {item}; run `protonctl \
                     setup drive` on a terminal, then restart Claude Code and Claude Desktop"
                );
                if cfg.is_some_and(|c| c.old_cli_sha256.is_some()) {
                    write!(
                        why,
                        " (cli_sha256 in [drive] of {} is no longer read: the pin moved to the \
                         secret store)",
                        crate::config::path().display()
                    )
                    .expect("writing to a String cannot fail");
                }
                Err(why)
            }
            Err(e) => Err(format!(
                "cannot read {item}, the Proton Drive CLI's pin: {e:#}"
            )),
        })
    }
}

/// `codesign` on the copy; the error names the file it was made from.
fn signed_by_proton(copy: &Path, cli: &Path) -> Result<()> {
    let requirement =
        format!("=anchor apple generic and certificate leaf[subject.OU] = \"{PROTON_TEAM}\"");
    let sig = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", &format!("-R{requirement}")])
        .arg(copy)
        .output()
        .context("cannot run /usr/bin/codesign")?;
    if !sig.status.success() {
        // Seen 2026-10-02: inside Claude Code's Bash sandbox, codesign reports a
        // valid Proton binary as "code or signature have been modified".
        bail!(
            "{} failed verification as signed by Proton's Apple team {PROTON_TEAM}: {} \
             (codesign misreports inside a sandbox; confirm from a normal terminal)",
            cli.display(),
            String::from_utf8_lossy(&sig.stderr).trim()
        );
    }
    Ok(())
}

fn matches_pin(found: Sha256, pin: Result<&Sha256, &String>, cli: &Path) -> Result<()> {
    let pin = match pin {
        Ok(pin) => *pin,
        Err(why) => bail!("{why}"),
    };
    if found != pin {
        bail!(
            "{} has SHA-256 {found}, not the {pin} pinned at setup; if you updated the CLI \
             from Proton, run `protonctl setup drive` to pin the new one, then restart \
             Claude Code and Claude Desktop, whose servers keep the pin they started with",
            cli.display()
        );
    }
    Ok(())
}

/// Run the copy with `--version` and check the series. The caller holds the
/// lock.
fn read_version(copy: &PrivateCopy, cli: &Path) -> Result<String> {
    let out = copy
        .command()
        .arg("--version")
        .output()
        .with_context(|| format!("cannot run {}", cli.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text
        .lines()
        .find_map(|l| l.split("cli-drive@").nth(1))
        .and_then(|v| v.split(['+', ' ']).next())
        .with_context(|| format!("unexpected --version output: {text:?}"))?;
    if !version.starts_with(SUPPORTED) {
        bail!(
            "Proton Drive CLI {version} is untested; protonctl supports {SUPPORTED}x. Re-run the Drive fixture tests before widening this."
        );
    }
    Ok(version.to_string())
}

/// Under the lock: copy the CLI, check the copy, and read its version
/// unless this process has. The copy is what the caller runs.
fn checked_copy(cli: &Path, proof: &Proof, known: bool) -> Result<(PrivateCopy, Option<String>)> {
    let copy = PrivateCopy::of(cli)?;
    proof.check(&copy, cli)?;
    let version = if known {
        None
    } else {
        Some(read_version(&copy, cli)?)
    };
    Ok((copy, version))
}

/// The CLI, checked before every run (R9, Q24) and run one call at a time,
/// since parallel calls fail with "database is locked" (RFC principle 6):
/// the mutex orders this process's calls, and `lock` keeps out other
/// processes.
#[derive(Debug)]
pub struct Cli {
    path: PathBuf,
    proof: Proof,
    /// Held for each whole call; the version once checked in this process.
    version: Mutex<Option<String>>,
}

impl Cli {
    /// The CLI that `[drive]` names, or the default one. On Linux, with
    /// Drive set up, this reads the pin from the secret store; a Mac checks
    /// Proton's signature instead.
    pub fn new(cfg: Option<&DriveConfig>) -> Self {
        let stored = if cfg.is_some() && !cfg!(target_os = "macos") {
            stored_pin()
        } else {
            Ok(None)
        };
        Self::with_stored(cfg, stored)
    }

    /// `new`, given what the secret store held for the pin.
    fn with_stored(cfg: Option<&DriveConfig>, stored: Result<Option<Sha256>>) -> Self {
        let proof = if cfg!(target_os = "macos") {
            Proof::Signature
        } else {
            Proof::pinned(stored, cfg)
        };
        Self {
            path: path(cfg),
            proof,
            version: Mutex::new(None),
        }
    }

    /// The CLI that `[drive]` names, held to `pin`, which the user has just
    /// confirmed (`setup drive` on Linux).
    pub fn pinned(cfg: Option<&DriveConfig>, pin: Sha256) -> Self {
        Self::with_stored(cfg, Ok(Some(pin)))
    }

    /// A stand-in the tests build, taken as Proton's.
    #[cfg(test)]
    pub fn trusted(path: PathBuf) -> Self {
        Self {
            path,
            proof: Proof::Trusted,
            version: Mutex::new(Some("test".into())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The pin each run is held to: `None` on macOS, which checks Proton's
    /// signature instead; else the pin, or why there is none.
    pub fn pin(&self) -> Option<Result<Sha256, &str>> {
        match &self.proof {
            Proof::Pinned(pin) => Some(pin.as_ref().copied().map_err(String::as_str)),
            Proof::Signature => None,
            #[cfg(test)]
            Proof::Trusted => None,
        }
    }

    /// Check the binary and its version now, and return the version.
    pub async fn check(&self) -> Result<String> {
        let mut version = self.version.lock().await;
        let (path, proof) = (self.path.clone(), self.proof.clone());
        let checked = crate::content::blocking(move || {
            let _held = lock(&lock_path())?;
            let (_copy, checked) = checked_copy(&path, &proof, false)?;
            checked.context("the version was not read")
        })
        .await??;
        *version = Some(checked.clone());
        Ok(checked)
    }

    /// Run one command and parse the JSON it prints, with only `environment()`.
    /// Stdin is empty, so a prompt fails instead of waiting.
    pub async fn json(&self, args: &[&OsStr]) -> Result<Value> {
        let mut version = self.version.lock().await;
        let (path, proof, known) = (self.path.clone(), self.proof.clone(), version.is_some());
        let (held, copy, checked) = crate::content::blocking(move || -> Result<_> {
            let held = lock(&lock_path())?;
            // Under the lock; the copy checked here is the one that runs.
            let (copy, checked) = checked_copy(&path, &proof, known)?;
            Ok((held, copy, checked))
        })
        .await??;
        if checked.is_some() {
            *version = checked;
        }
        let out = tokio::process::Command::from(copy.command())
            .args(args)
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| format!("cannot run {}", self.path.display()));
        drop((copy, held));
        let out = out?;
        if !out.status.success() {
            // CLI 0.8.0's wording for a missing path (RFC Appendix A).
            if out.stderr.starts_with(b"Node not found") {
                return Err(NotFound.into());
            }
            let said = if out.stderr.is_empty() {
                &out.stdout
            } else {
                &out.stderr
            };
            // The CLI's words can repeat a node's name, which a third party wrote (R6).
            let said: String = clean(&String::from_utf8_lossy(said), &mut 0)
                .trim()
                .chars()
                .take(500)
                .collect();
            bail!("the Proton Drive CLI failed: {said}");
        }
        serde_json::from_slice(&out.stdout).context("the Proton Drive CLI did not print JSON")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Write a stand-in CLI, `proton-drive` in `dir`, that runs `body` with
    /// `/bin/sh` and returns its path. Every run is of a copy, on Linux at
    /// `/proc/self/fd/N` and close-on-exec, which a `#!/bin/sh` script
    /// cannot use: the shell is handed that path after it has closed. So the
    /// stand-in is one line that has `env -S` run `body`, kept beside it as
    /// `proton-drive.sh`; `$ran` holds the path the kernel ran, `$0` is
    /// `body`'s own file in `dir`, and the arguments are the CLI's.
    pub(crate) fn stand_in(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let (cli, script) = (dir.join("proton-drive"), dir.join("proton-drive.sh"));
        std::fs::write(&script, format!("ran=\"$1\"\nshift\n{body}")).unwrap();
        let line = format!("#!/usr/bin/env -S /bin/sh {}\n", script.display());
        std::fs::write(&cli, line).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        cli
    }

    /// RFC R10: when `serve` stops, shutting the runtime down drops a call
    /// still waiting on the CLI, and `kill_on_drop` ends the CLI with it, so
    /// it cannot keep writing into the download folder after it is deleted.
    /// The stand-in writes a heartbeat line every 50 ms until it dies.
    #[test]
    fn a_running_cli_dies_with_the_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let beat = dir.path().join("beat");
        let script = stand_in(
            dir.path(),
            &format!(
                "while :; do echo x >> '{}'; /bin/sleep 0.05; done\n",
                beat.display()
            ),
        );
        let beats = || std::fs::read_to_string(&beat).map_or(0, |b| b.len());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.spawn(async move { drop(Cli::trusted(script).json(&[]).await) });
        let deadline = Instant::now() + Duration::from_secs(10);
        while beats() == 0 {
            assert!(Instant::now() < deadline, "the stand-in CLI never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        rt.shutdown_timeout(Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(200));
        let before = beats();
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(beats(), before, "the CLI outlived the runtime");
    }

    /// The CLI's own words can repeat a node's name, which whoever named the
    /// file wrote, so they reach the error cleaned (R6).
    #[tokio::test]
    async fn the_cli_s_failure_text_is_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        // \342\200\256 is U+202E, RIGHT-TO-LEFT OVERRIDE, in UTF-8.
        let script = stand_in(
            dir.path(),
            "printf 'cannot read plan\\342\\200\\256dm.exe\\n' >&2\nexit 1\n",
        );
        let err = Cli::trusted(script)
            .json(&[])
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains('\u{202E}'), "{err:?}");
        assert!(err.contains("cannot read plandm.exe"), "{err}");
    }

    /// A stand-in that answers `--version` as CLI 0.8.0 does and `{}` to
    /// anything else, logging each run's arguments to `runs` beside it, held
    /// to `pin`, or by default to its own SHA-256. `extra` runs first.
    fn pinned_stand_in(dir: &Path, pin: Option<Sha256>, extra: &str) -> (Cli, PathBuf) {
        let runs = dir.join("runs");
        let script = stand_in(
            dir,
            &format!(
                "{extra}echo \"$*\" >> '{}'\n\
                 if [ \"$1\" = --version ]; then echo '@protontech/cli-drive@0.8.0'; else echo '{{}}'; fi\n",
                runs.display()
            ),
        );
        let pin = pin.unwrap_or_else(|| Sha256::of_file(&script).unwrap());
        let cli = Cli {
            path: script,
            proof: Proof::Pinned(Ok(pin)),
            version: Mutex::new(None),
        };
        (cli, runs)
    }

    /// Q24 and Q33: the pin is checked before every run, not once per
    /// process, so a CLI changed after the first run is never run again; and
    /// `--version` runs once per process.
    #[tokio::test]
    async fn a_pinned_cli_runs_only_while_it_matches_its_pin() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, runs) = pinned_stand_in(dir.path(), None, "");
        let list = [OsStr::new("list")];
        cli.json(&list).await.unwrap();
        cli.json(&list).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&runs).unwrap(),
            "--version\nlist\nlist\n"
        );
        let mut changed = std::fs::read_to_string(cli.path()).unwrap();
        changed.push_str("# swapped\n");
        std::fs::write(cli.path(), changed).unwrap();
        let err = cli.json(&list).await.unwrap_err().to_string();
        assert!(err.contains("pinned at setup"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&runs).unwrap(),
            "--version\nlist\nlist\n"
        );
    }

    /// Q24, #72: what runs is the copy that was checked, for `--version` and
    /// the command alike. The stand-in swaps another CLI in over itself
    /// while it answers `--version`, between the check and the command, as
    /// the security review's stand-in did; the command still runs the
    /// checked bytes, and the next call refuses the swapped file before it
    /// runs anything. On Linux the runs are of the sealed memfd.
    #[tokio::test]
    async fn a_cli_swapped_in_after_the_check_never_runs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (evil, evil_ran, ran) = (
            dir.path().join("evil"),
            dir.path().join("evil-ran"),
            dir.path().join("ran"),
        );
        let evil_script = dir.path().join("evil.sh");
        std::fs::write(
            &evil_script,
            format!("touch '{}'\necho '{{}}'\n", evil_ran.display()),
        )
        .unwrap();
        let line = format!("#!/usr/bin/env -S /bin/sh {}\n", evil_script.display());
        std::fs::write(&evil, line).unwrap();
        std::fs::set_permissions(&evil, std::fs::Permissions::from_mode(0o755)).unwrap();
        let swap = format!(
            "echo \"$ran\" >> '{}'\n[ \"$1\" = --version ] && mv '{}' \"$(dirname \"$0\")/proton-drive\"\n",
            ran.display(),
            evil.display()
        );
        let (cli, runs) = pinned_stand_in(dir.path(), None, &swap);
        let list = [OsStr::new("list")];
        cli.json(&list).await.unwrap();
        assert!(!evil_ran.exists(), "the swapped-in CLI ran");
        assert!(!evil.exists(), "the stand-in did not swap the file");
        assert_eq!(std::fs::read_to_string(&runs).unwrap(), "--version\nlist\n");
        let err = cli.json(&list).await.unwrap_err().to_string();
        assert!(err.contains("pinned at setup"), "{err}");
        assert!(!evil_ran.exists(), "the swapped-in CLI ran");
        let ran = std::fs::read_to_string(&ran).unwrap();
        for path in ran.lines() {
            assert_ne!(Path::new(path), cli.path(), "{ran}");
            if cfg!(target_os = "linux") {
                assert!(path.starts_with("/proc/self/fd/"), "{ran}");
            }
        }
    }

    /// A Linux `[drive]` without a pin is refused before the CLI runs.
    #[tokio::test]
    async fn a_cli_with_no_pin_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (mut cli, runs) = pinned_stand_in(dir.path(), None, "");
        cli.proof = Proof::pinned(Ok(None), None);
        let err = cli.check().await.unwrap_err().to_string();
        assert!(
            err.contains("run `protonctl setup drive` on a terminal"),
            "{err}"
        );
        assert!(!runs.exists());
        // And a pin for other bytes is refused the same way.
        let (cli, runs) = pinned_stand_in(dir.path(), Some(Sha256::of(b"other")), "");
        assert!(cli.check().await.is_err());
        assert!(!runs.exists());
    }

    /// #72: a pin written into the config, where the model can edit it in
    /// Claude Code, is not trusted, even when it matches the file; the
    /// error says the pin moved and what to run. Only the secret store's
    /// pin counts, and unit tests cannot reach a store, so `new` refuses.
    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn a_pin_in_the_config_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, runs) = pinned_stand_in(dir.path(), None, "");
        let pin = Sha256::of_file(cli.path()).unwrap();
        let cfg: DriveConfig = toml::from_str(&format!(
            "cli = {}\ncli_sha256 = \"{pin}\"\n",
            toml::Value::from(cli.path().to_str().unwrap())
        ))
        .unwrap();
        let none_stored = Cli::with_stored(Some(&cfg), Ok(None));
        assert_eq!(none_stored.path(), cli.path());
        let err = none_stored.check().await.unwrap_err().to_string();
        assert!(
            err.contains("run `protonctl setup drive` on a terminal"),
            "{err}"
        );
        assert!(err.contains("cli_sha256 in [drive]"), "{err}");
        let err = Cli::new(Some(&cfg)).check().await.unwrap_err().to_string();
        assert!(err.contains("drive-cli-pin"), "{err}");
        assert!(!runs.exists(), "the CLI ran on the config's pin");
        // The same pin from the secret store is trusted.
        Cli::with_stored(Some(&cfg), Ok(Some(pin)))
            .check()
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&runs).unwrap(), "--version\n");
    }

    /// Two handles on the lock file exclude each other. flock(2) locks belong
    /// to each open file, so this holds within one process as across two.
    #[test]
    fn lock_handles_exclude_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new/cli.lock");
        let held = lock(&path).unwrap();
        let other = File::options().write(true).open(&path).unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(held);
        other.try_lock().unwrap();
    }

    /// A run waits while the lock is held, as it is when another protonctl
    /// process (the Claude app's server, say) is running the CLI.
    #[tokio::test]
    async fn a_cli_run_waits_for_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let ran = dir.path().join("ran");
        let script = stand_in(
            dir.path(),
            &format!("touch '{}'\necho '{{}}'\n", ran.display()),
        );
        let held = lock(&lock_path()).unwrap();
        let run = tokio::spawn(async move { Cli::trusted(script).json(&[]).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!ran.exists(), "the CLI ran while the lock was held");
        drop(held);
        run.await.unwrap().unwrap();
        assert!(ran.exists());
    }
}
