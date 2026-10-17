//! RFC-0001 Q36 against `scripts/serve`, the plugin's launcher,
//! with a throwaway home and no other environment, as a desktop app may start
//! it with no `~/.cargo/bin` on its PATH: without an executable protonctl in
//! `~/.cargo/bin` it exits with status 1, writes nothing to stdout, which
//! carries MCP frames, and writes one line to stderr that names the install
//! step; with one there, it runs `protonctl serve`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// The launcher, run with `HOME` set to `home` and nothing else set.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a panic is a failed test"
)]
fn serve(home: &Path) -> Output {
    Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/serve"))
        .env_clear()
        .env("HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Writes `body` as `~/.cargo/bin/protonctl` under `home`, with `mode`.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a panic is a failed test"
)]
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
}

#[test]
fn without_protonctl_it_names_the_install_step() {
    let home = tempfile::tempdir().unwrap();
    assert_names_the_install_step(&serve(home.path()));
}

/// A file that cannot run counts as no protonctl, so the message still says
/// what to do.
#[test]
fn a_protonctl_that_is_not_executable_counts_as_none() {
    let home = tempfile::tempdir().unwrap();
    install(home.path(), "#!/bin/sh\necho \"$@\"\n", 0o644);
    assert_names_the_install_step(&serve(home.path()));
}

#[test]
fn with_protonctl_it_runs_protonctl_serve() {
    let home = tempfile::tempdir().unwrap();
    install(home.path(), "#!/bin/sh\necho \"$@\"\n", 0o755);
    let out = serve(home.path());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{}: {stderr}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "serve\n");
    assert!(stderr.is_empty(), "{stderr}");
}
