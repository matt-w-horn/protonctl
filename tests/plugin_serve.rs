//! Tests for `plugin/scripts/serve`, the plugin's launcher, with a throwaway
//! home and no other environment, as a desktop app may start it with no
//! `~/.cargo/bin` on its PATH: without an executable protonctl in
//! `~/.cargo/bin` or where Homebrew links one, it exits with status 1, writes
//! nothing to stdout, which carries MCP frames, and writes one line to stderr
//! that names both ways to install; with one in `~/.cargo/bin`, it runs
//! `protonctl serve`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const LAUNCHER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/plugin/scripts/serve");

/// The launcher as `plugin.json` starts it (`sh` with the script in `args`),
/// with `HOME` set to `home` and `PATH` carrying only `/bin` so `sh` is found.
fn serve(home: &Path) -> Output {
    Command::new("/bin/sh")
        .arg(LAUNCHER)
        .env_clear()
        .env("HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Writes `body` as `~/.cargo/bin/protonctl` under `home`, with `mode`.
fn install(home: &Path, body: &str, mode: u32) {
    let bin = home.join(".cargo/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let protonctl = bin.join("protonctl");
    std::fs::write(&protonctl, body).unwrap();
    std::fs::set_permissions(&protonctl, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn assert_names_the_install_step(out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{}: {stderr}", out.status);
    assert!(
        out.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("scripts/install.sh"), "{stderr}");
    assert!(
        stderr.contains("brew install matt-w-horn/tap/protonctl"),
        "{stderr}"
    );
}

/// The launcher's two fixed Homebrew paths. On a computer that has protonctl
/// at one, the launcher finds it whatever `HOME` says, so a test of "no
/// protonctl" cannot run there; it says so and passes.
fn homebrew_has_one() -> bool {
    let found = ["/opt/homebrew/bin", "/home/linuxbrew/.linuxbrew/bin"]
        .iter()
        .map(|bin| Path::new(bin).join("protonctl"))
        .find(|p| p.exists());
    if let Some(path) = &found {
        eprintln!("not run: {} exists on this computer", path.display());
    }
    found.is_some()
}

#[test]
fn without_protonctl_it_names_the_install_step() {
    if homebrew_has_one() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    assert_names_the_install_step(&serve(home.path()));
}

/// A file that cannot run counts as no protonctl, so the message still says
/// what to do.
#[test]
fn a_protonctl_that_is_not_executable_counts_as_none() {
    if homebrew_has_one() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    install(home.path(), "#!/bin/sh\necho \"$@\"\n", 0o644);
    assert_names_the_install_step(&serve(home.path()));
}

/// The one in `~/.cargo/bin` runs first, so a build from a clone is not
/// passed over for a Homebrew install, both through `/bin/sh` and directly.
#[test]
fn with_protonctl_it_runs_protonctl_serve() {
    let home = tempfile::tempdir().unwrap();
    install(home.path(), "#!/bin/sh\necho \"$@\"\n", 0o755);
    for out in [
        serve(home.path()),
        Command::new(LAUNCHER)
            .env_clear()
            .env("HOME", home.path())
            .stdin(Stdio::null())
            .output()
            .unwrap(),
    ] {
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{}: {stderr}", out.status);
        assert_eq!(String::from_utf8_lossy(&out.stdout), "serve\n");
        assert!(stderr.is_empty(), "{stderr}");
    }
}
