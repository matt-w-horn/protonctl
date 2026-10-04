//! The official Proton Drive CLI. Before protonctl runs it, the binary must be
//! signed by Proton's Apple team and report a version this code was tested
//! against (RFC R9), so a look-alike earlier on PATH never sees the session.
//! Downloads go through it, and so do listing and stat when the Proton Drive
//! app's folder is absent; the write operations arrive in Phase 1b.
//! Every run holds a lock file, so two protonctl processes (the Claude app's
//! server and Claude Code's) never run it at once.

use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::{DriveConfig, cache_dir, home};

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

/// Check the signature and version; returns the version string. `--version`
/// also asks proton.me whether a newer CLI exists (RFC Appendix A).
pub fn verify(cli: &Path) -> Result<String> {
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
    let held = lock(&lock_path())?;
    let out = Command::new(cli)
        .arg("--version")
        .env_clear()
        .envs(environment())
        .output()
        .with_context(|| format!("cannot run {}", cli.display()))?;
    drop(held);
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

/// The CLI, verified before its first use in a process (R9) and run one call
/// at a time, since parallel calls fail with "database is locked" (RFC
/// principle 6): the mutex orders this process's calls, and `lock` keeps out
/// other processes.
pub struct Cli {
    path: PathBuf,
    /// Held for each whole call; true once `verify` passed in this process.
    verified: Mutex<bool>,
}

impl Cli {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            verified: Mutex::new(false),
        }
    }

    /// A stand-in the tests build, taken as verified.
    #[cfg(test)]
    pub fn trusted(path: PathBuf) -> Self {
        Self {
            path,
            verified: Mutex::new(true),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run one command and parse the JSON it prints, with only `environment()`.
    /// Stdin is empty, so a prompt fails instead of waiting.
    pub async fn json(&self, args: &[&OsStr]) -> Result<Value> {
        let mut verified = self.verified.lock().await;
        if !*verified {
            let path = self.path.clone();
            tokio::task::spawn_blocking(move || verify(&path)).await??;
            *verified = true;
        }
        let _held = tokio::task::spawn_blocking(|| lock(&lock_path())).await??;
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
            let said: String = String::from_utf8_lossy(said)
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
