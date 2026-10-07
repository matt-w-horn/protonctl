//! `protonctl setup drive` on Linux, against the built binary (RFC Q33,
//! #72): the CLI's pin lives in the secret store, and the first setup asks
//! on a terminal before it pins anything, as a re-pin does.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Stdio};

/// A first `setup drive` refuses without a terminal, so a call through
/// Claude Code's Bash tool, which has none, cannot pin a file the model
/// chose. It names the file and its SHA-256, runs nothing, and writes no
/// config. The stand-in would answer every call a setup makes.
#[test]
fn a_first_drive_setup_refuses_without_a_terminal() {
    let home = tempfile::tempdir().unwrap();
    let (cli, script, ran) = (
        home.path().join("proton-drive"),
        home.path().join("proton-drive.sh"),
        home.path().join("ran"),
    );
    std::fs::write(
        &script,
        format!(
            "shift\ntouch '{}'\n\
             if [ \"$1\" = --version ]; then echo '@protontech/cli-drive@0.8.0'; else echo '[]'; fi\n",
            ran.display()
        ),
    )
    .unwrap();
    let line = format!("#!/usr/bin/env -S /bin/sh {}\n", script.display());
    std::fs::write(&cli, line).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pin = hex::encode(ring::digest::digest(
        &ring::digest::SHA256,
        &std::fs::read(&cli).unwrap(),
    ));
    let config = home.path().join("config.toml");
    let mut c = Command::new(env!("CARGO_BIN_EXE_protonctl"));
    // No D-Bus session, so the user's own keyring is never reached.
    c.env_clear()
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
        .env("HOME", home.path())
        .env("PROTONCTL_CONFIG", &config);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        c.env("LLVM_PROFILE_FILE", profile);
    }
    let out = c
        .args(["setup", "drive", "--cli"])
        .arg(&cli)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{out:?}");
    assert!(said.contains("asks first, on a terminal"), "{said}");
    assert!(
        said.contains(&format!("{} has SHA-256 {pin}", cli.display())),
        "{said}"
    );
    assert!(!ran.exists(), "setup ran the CLI before it was confirmed");
    assert!(!config.exists(), "setup wrote a config");
}
