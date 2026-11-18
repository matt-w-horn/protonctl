//! Tests for `scripts/sign.sh`, with stand-ins for `uname`,
//! `security` and `codesign` first on its PATH, so it runs on any computer and
//! touches no keychain: it signs a copy with the identity, the identifier and
//! the hardened runtime, and renames the copy into place only once the copy
//! shows the hardened runtime.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

/// A folder of stand-ins. `codesign` records each call in `calls`, and
/// reports the hardened runtime unless `plain` exists beside it.
fn stand_ins(dir: &Path) {
    let write = |name: &str, body: &str| {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    write("uname", "echo Darwin");
    // The identity is there already, so none is made.
    write("security", r#"echo '  1) ABCDEF "protonctl"'"#);
    write(
        "codesign",
        r#"here=$(dirname "$0")
echo "$*" >> "$here/calls"
case "$1" in
--force) [ ! -e "$here/refuses" ] ;;
-d) if [ "$2" = -v ]; then
        if [ -e "$here/plain" ]; then echo "flags=0x2(adhoc)" >&2; else echo "flags=0x10000(runtime)" >&2; fi
    else
        echo 'designated => identifier "protonctl" and certificate leaf = H"00"' >&2
    fi ;;
esac"#,
    );
}

fn sign(tools: &Path, args: &[&Path]) -> Output {
    let path = format!("{}:/usr/bin:/bin", tools.display());
    Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/sign.sh"))
        .args(args)
        .env_clear()
        .env("PATH", path)
        .env("HOME", tools)
        .output()
        .unwrap()
}

#[test]
fn it_signs_a_copy_and_renames_it_into_place() {
    let dir = tempfile::tempdir().unwrap();
    stand_ins(dir.path());
    let built = dir.path().join("built");
    std::fs::write(&built, "new build").unwrap();
    let installed = dir.path().join("bin/protonctl");
    std::fs::create_dir(dir.path().join("bin")).unwrap();
    std::fs::write(&installed, "old build").unwrap();

    let out = sign(dir.path(), &[&built, &installed]);
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{}: {said}", out.status);
    assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new build");
    // The build itself is left as it was built, and no copy is left behind.
    assert_eq!(std::fs::read_to_string(&built).unwrap(), "new build");
    assert!(!dir.path().join("bin/.protonctl.new").exists());
    let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
    let signed = calls.lines().next().unwrap();
    assert!(
        signed.starts_with("--force --sign protonctl --identifier protonctl --options runtime "),
        "{calls}"
    );
    assert!(signed.ends_with("/bin/.protonctl.new"), "{calls}");
}

/// With no second path, the signed copy takes the build's own place, as a
/// Homebrew install needs.
#[test]
fn with_one_path_it_signs_in_place() {
    let dir = tempfile::tempdir().unwrap();
    stand_ins(dir.path());
    let built = dir.path().join("protonctl");
    std::fs::write(&built, "build").unwrap();
    let out = sign(dir.path(), &[&built]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&built).unwrap(), "build");
    assert!(!dir.path().join(".protonctl.new").exists());
}

/// A copy without the hardened runtime, or one codesign did not sign, never
/// replaces the binary installed. `install.sh` checked after it had replaced.
#[test]
fn a_copy_that_fails_the_check_leaves_the_installed_binary() {
    for fault in ["plain", "refuses"] {
        let dir = tempfile::tempdir().unwrap();
        stand_ins(dir.path());
        std::fs::write(dir.path().join(fault), "").unwrap();
        let built = dir.path().join("built");
        std::fs::write(&built, "new build").unwrap();
        let installed = dir.path().join("protonctl");
        std::fs::write(&installed, "old build").unwrap();
        let out = sign(dir.path(), &[&built, &installed]);
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{fault}: {said}");
        assert!(said.contains("is as it was"), "{fault}: {said}");
        assert_eq!(std::fs::read_to_string(&installed).unwrap(), "old build");
        assert!(!dir.path().join(".protonctl.new").exists(), "{fault}");
    }
}

/// A rename onto a link would sign nothing the link points to.
#[test]
fn a_link_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    stand_ins(dir.path());
    let built = dir.path().join("built");
    std::fs::write(&built, "build").unwrap();
    let link = dir.path().join("protonctl");
    std::os::unix::fs::symlink(&built, &link).unwrap();
    let out = sign(dir.path(), &[&built, &link]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is a link"));
    assert!(!dir.path().join("calls").exists(), "codesign ran");
}
