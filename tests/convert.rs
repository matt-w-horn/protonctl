//! The document readers on Linux (RFC R21, Q34), against the built binary:
//! `protonctl convert` runs each reader in its sandbox, and `drive cat`
//! reads documents through it as the server does. They need poppler-utils
//! and pandoc. The fixtures: `two-pages.pdf` was written by hand, one
//! Helvetica line per page; the Word, OpenDocument and RTF files were made
//! by pandoc 3.1.3 from "Word text, Café." and "Second paragraph.".
#![cfg(target_os = "linux")]
#![expect(
    clippy::unwrap_used,
    reason = "integration tests: a panic is a failed test"
)]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// `protonctl convert`'s own failure, as `convert::SANDBOX_FAILED` says.
const SANDBOX_FAILED: i32 = 70;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The built binary, with an empty environment and no D-Bus session, so it
/// never reaches the user's own keyring.
fn protonctl() -> Command {
    for (tool, package) in [
        ("/usr/bin/pdftotext", "poppler-utils"),
        ("/usr/bin/pandoc", "pandoc"),
    ] {
        assert!(
            Path::new(tool).exists(),
            "these tests need {package}: install it"
        );
    }
    let mut c = Command::new(env!("CARGO_BIN_EXE_protonctl"));
    c.env_clear()
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent");
    // Under cargo-llvm-cov, where the binary writes its coverage; without
    // it, a profile lands in the working folder.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        c.env("LLVM_PROFILE_FILE", profile);
    }
    c
}

fn convert(args: &[&str], input: &[u8]) -> Output {
    let mut child = protonctl()
        .arg("convert")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn each_reader_runs_in_its_sandbox() {
    let pdf = std::fs::read(fixture("two-pages.pdf")).unwrap();
    let text = convert(&["pdf-text"], &pdf);
    assert!(text.status.success(), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains("Café — accents.\n\n\u{C}Page two here."),
        "{text:?}"
    );
    let info = String::from_utf8(convert(&["pdf-info"], &pdf).stdout).unwrap();
    assert!(
        info.lines()
            .any(|l| l.split_whitespace().eq(["Pages:", "2"])),
        "{info}"
    );
    let page = convert(&["pdf-page", "--page", "2", "--edge", "300"], &pdf);
    assert!(page.stdout.starts_with(b"\xFF\xD8\xFF"), "{page:?}");
    for format in ["docx", "odt", "rtf"] {
        let doc = std::fs::read(fixture(&format!("text.{format}"))).unwrap();
        let out = convert(&["document", "--format", format], &doc);
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "Word text, Café.\n\nSecond paragraph.\n",
            "{format}"
        );
    }
    assert!(convert(&["check"], b"").status.success());
}

/// A damaged PDF is the reader's failure, which the server reports as an
/// unreadable file, and not the sandbox's, which it reports as an error.
#[test]
fn a_damaged_pdf_fails_in_the_reader_not_the_sandbox() {
    let out = convert(&["pdf-text"], b"%PDF-1.4 damaged");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_ne!(out.status.code(), Some(SANDBOX_FAILED));
}

/// The whole path a server takes: the Drive CLI (a stand-in, pinned as
/// `setup drive` pins it) downloads the file, and protonctl reads it
/// through `/proc/self/exe convert`. The stand-in fails any call but the
/// ones a read makes, and records it, so the test fails on it (R1).
#[test]
fn drive_reads_documents_through_the_sandboxed_readers() {
    let home = tempfile::tempdir().unwrap();
    let cli = home.path().join("proton-drive");
    let unexpected = home.path().join("unexpected.txt");
    let fixtures = fixture("");
    std::fs::write(
        &cli,
        format!(
            r#"#!/bin/sh
f='{}'
case "$#:$*" in
"1:--version") echo '@protontech/cli-drive@0.8.0' ;;
"4:filesystem info -j "*)
  name=$(basename "$4")
  printf '{{"uid":"v~n","parentUid":"v~p","name":{{"ok":true,"value":"%s"}},"type":"file","modificationTime":"2026-09-28T17:04:05.123Z","activeRevision":{{"uid":"r","claimedSize":%s,"claimedModificationTime":"2026-09-01T08:00:00.000Z"}}}}' "$name" "$(wc -c < "$f/$name")" ;;
"9:filesystem download -j -f skip -d skip "*)
  for dest; do :; done
  cp "$f/$(basename "$8")" "$dest/"
  echo '{{"transferredItems":1,"transferredBytes":1,"skippedItems":0,"failedItems":0,"failures":[]}}' ;;
*)
  echo "$*" >> '{}'
  echo "protonctl never makes this call (R1): $*" >&2
  exit 2 ;;
esac
"#,
            fixtures.display(),
            unexpected.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pin = ring::digest::digest(&ring::digest::SHA256, &std::fs::read(&cli).unwrap());
    let config = home.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[drive]\ncli = \"{}\"\ncli_sha256 = \"{}\"\n",
            cli.display(),
            hex::encode(pin)
        ),
    )
    .unwrap();
    let cat = |args: &[&str]| -> serde_json::Value {
        let out = protonctl()
            .env("HOME", home.path())
            .env("PROTONCTL_CONFIG", &config)
            .args(["drive", "cat"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let pdf = cat(&["/two-pages.pdf"]);
    assert_eq!(
        pdf["content"],
        "Hello from page one.\nCafé — accents.\u{C}Page two here."
    );
    assert_eq!(pdf["textFrom"], "pdf");
    assert_eq!(pdf["pageStarts"], serde_json::json!([0, 37]));
    let pages = cat(&["/two-pages.pdf", "--page", "2"]);
    assert_eq!(pages["pagesShown"], serde_json::json!([2, 2]), "{pages}");
    assert_eq!(pages["pdfPages"], 2);
    let word = cat(&["/text.docx"]);
    assert_eq!(word["content"], "Word text, Café.\n\nSecond paragraph.\n");
    assert_eq!(word["textFrom"], "pandoc");
    // The download folder goes once read.
    let downloads = home.path().join(".cache/protonctl/downloads");
    let left = std::fs::read_dir(&downloads).map_or(0, |d| d.flatten().count());
    assert_eq!(left, 0, "{}", downloads.display());
    let calls = std::fs::read_to_string(&unexpected).unwrap_or_default();
    assert!(
        calls.is_empty(),
        "protonctl ran the Drive CLI with a call it must never make (R1):\n{calls}"
    );
}
