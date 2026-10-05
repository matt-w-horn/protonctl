//! The official Proton Drive CLI. Before every run protonctl checks that the
//! binary is Proton's (RFC R9, Q24): on macOS, signed by Proton's Apple team;
//! on Linux, where Proton publishes no signature or checksum, the SHA-256
//! that `setup drive` pinned (Q33). Its version is checked once per process,
//! since `--version` also asks proton.me for updates (RFC Appendix A).
//! Downloads go through it, and so do listing and stat when the Proton Drive
//! app's folder is absent. Every run holds a lock file, so two protonctl
//! processes (the Claude app's server and Claude Code's) never run it at once.

use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::{DriveConfig, cache_dir, home};
use crate::content::clean;
use crate::digest::Sha256;

/// Proton AG's Apple Developer team, shared by Proton Drive.app and the CLI.
const PROTON_TEAM: &str = "2SB5Z68H26";
/// The CLI minor series whose behaviour protonctl was written against.
const SUPPORTED: &str = "0.8.";

pub fn path(cfg: Option<&DriveConfig>) -> PathBuf {
    cfg.and_then(|c| c.cli.clone())
        .unwrap_or_else(|| home().join("bin/proton-drive"))
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

/// What shows that a file is Proton's CLI (R9).
#[derive(Clone, Debug)]
enum Proof {
    /// macOS: signed by Proton's Apple team.
    Signature,
    /// Linux: this SHA-256, pinned at `setup drive`; `None` when `[drive]`
    /// has none (Q33).
    Pinned(Option<Sha256>),
    /// A stand-in the tests build.
    #[cfg(test)]
    Trusted,
}

impl Proof {
    /// It blocks, so async callers run it off the async threads.
    fn check(&self, cli: &Path) -> Result<()> {
        match self {
            Self::Signature => signed_by_proton(cli),
            Self::Pinned(pin) => matches_pin(cli, *pin),
            #[cfg(test)]
            Self::Trusted => Ok(()),
        }
    }
}

fn signed_by_proton(cli: &Path) -> Result<()> {
    let requirement =
        format!("=anchor apple generic and certificate leaf[subject.OU] = \"{PROTON_TEAM}\"");
    let sig = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", &format!("-R{requirement}")])
        .arg(cli)
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

fn matches_pin(cli: &Path, pin: Option<Sha256>) -> Result<()> {
    let Some(pin) = pin else {
        bail!(
            "no SHA-256 is pinned for the Proton Drive CLI (cli_sha256 in [drive]); run `protonctl setup drive`"
        );
    };
    let found = Sha256::of_file(cli)?;
    if found != pin {
        bail!(
            "{} has SHA-256 {found}, not the {pin} pinned at setup; if you updated the CLI \
             from Proton, run `protonctl setup drive` to pin the new one",
            cli.display()
        );
    }
    Ok(())
}

/// Run `--version` and check the series. The caller holds the lock.
fn read_version(cli: &Path) -> Result<String> {
    let out = Command::new(cli)
        .arg("--version")
        .env_clear()
        .envs(environment())
        .stdin(Stdio::null())
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
    /// The CLI that `[drive]` names, or the default one.
    pub fn new(cfg: Option<&DriveConfig>) -> Self {
        let proof = if cfg!(target_os = "macos") {
            Proof::Signature
        } else {
            Proof::Pinned(cfg.and_then(|c| c.cli_sha256))
        };
        Self {
            path: path(cfg),
            proof,
            version: Mutex::new(None),
        }
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

    /// Check the binary and its version now, and return the version.
    pub async fn check(&self) -> Result<String> {
        let mut version = self.version.lock().await;
        let (path, proof) = (self.path.clone(), self.proof.clone());
        let checked = crate::content::blocking(move || {
            let _held = lock(&lock_path())?;
            proof.check(&path)?;
            read_version(&path)
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
        let (_held, checked) = crate::content::blocking(move || -> Result<_> {
            let held = lock(&lock_path())?;
            // Under the lock, as close to the run as this process can put it.
            proof.check(&path)?;
            let checked = if known {
                None
            } else {
                Some(read_version(&path)?)
            };
            Ok((held, checked))
        })
        .await??;
        if checked.is_some() {
            *version = checked;
        }
        let out = tokio::process::Command::new(&self.path)
            .args(args)
            .env_clear()
            .envs(environment())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| format!("cannot run {}", self.path.display()))?;
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
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// RFC R10: when `serve` stops, shutting the runtime down drops a call
    /// still waiting on the CLI, and `kill_on_drop` ends the CLI with it, so
    /// it cannot keep writing into the download folder after it is deleted.
    /// The stand-in writes a heartbeat line every 50 ms until it dies.
    #[test]
    fn a_running_cli_dies_with_the_runtime() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (script, beat) = (dir.path().join("proton-drive"), dir.path().join("beat"));
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nwhile :; do echo x >> '{}'; /bin/sleep 0.05; done\n",
                beat.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
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
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("proton-drive");
        // \342\200\256 is U+202E, RIGHT-TO-LEFT OVERRIDE, in UTF-8.
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'cannot read plan\\342\\200\\256dm.exe\\n' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = Cli::trusted(script)
            .json(&[])
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains('\u{202E}'), "{err:?}");
        assert!(err.contains("cannot read plandm.exe"), "{err}");
    }

    /// A stand-in that answers `--version` as CLI 0.8.0 does and `{}` to
    /// anything else, logging each run's arguments to `runs` beside it.
    fn pinned_stand_in(dir: &Path, pin: Option<Sha256>) -> (Cli, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let (script, runs) = (dir.join("proton-drive"), dir.join("runs"));
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\n\
                 if [ \"$1\" = --version ]; then echo '@protontech/cli-drive@0.8.0'; else echo '{{}}'; fi\n",
                runs.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pin = pin.or_else(|| Some(Sha256::of_file(&script).unwrap()));
        let cli = Cli {
            path: script,
            proof: Proof::Pinned(pin),
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
        let (cli, runs) = pinned_stand_in(dir.path(), None);
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

    /// A Linux `[drive]` without a pin is refused before the CLI runs.
    #[tokio::test]
    async fn a_cli_with_no_pin_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (mut cli, runs) = pinned_stand_in(dir.path(), None);
        cli.proof = Proof::Pinned(None);
        let err = cli.check().await.unwrap_err().to_string();
        assert!(err.contains("run `protonctl setup drive`"), "{err}");
        assert!(!runs.exists());
        // And a pin for other bytes is refused the same way.
        let (cli, runs) = pinned_stand_in(dir.path(), Some(Sha256::of(b"other")));
        assert!(cli.check().await.is_err());
        assert!(!runs.exists());
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
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (script, ran) = (dir.path().join("proton-drive"), dir.path().join("ran"));
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\necho '{{}}'\n", ran.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let held = lock(&lock_path()).unwrap();
        let run = tokio::spawn(async move { Cli::trusted(script).json(&[]).await });
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!ran.exists(), "the CLI ran while the lock was held");
        drop(held);
        run.await.unwrap().unwrap();
        assert!(ran.exists());
    }
}
