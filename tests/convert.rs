//! The document readers on Linux (RFC R21, Q34), against the built binary:
//! `protonctl convert` runs each reader in its sandbox, and `drive cat`
//! reads documents through it as the server does. They need poppler-utils
//! and pandoc. The fixtures: `two-pages.pdf` was written by hand, one
//! Helvetica line per page; the Word, OpenDocument and RTF files were made
//! by pandoc 3.1.3 from "Word text, Café." and "Second paragraph.".
//! `fills-memory.docx` was made by Python's `zipfile`, deflated: a
//! `[Content_Types].xml` and `_rels/.rels` naming `word/document.xml`, which
//! is one paragraph holding one run of 200 MiB of "x".
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

/// A reader's /proc files, read while it waits on stdin once `convert`
/// has become it, in the same process.
struct Running {
    status: String,
    limits: String,
    cmdline: String,
}

fn running(args: &[&str], reader: &str) -> Running {
    let mut child = protonctl()
        .arg("convert")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let proc = PathBuf::from(format!("/proc/{}", child.id()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::fs::read_link(proc.join("exe")).ok().as_deref() != Some(Path::new(reader)) {
        assert!(
            std::time::Instant::now() < deadline,
            "{reader} never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let read = |name| std::fs::read_to_string(proc.join(name)).unwrap();
    let seen = Running {
        status: read("status"),
        limits: read("limits"),
        cmdline: read("cmdline").replace('\0', " "),
    };
    drop(child.stdin.take());
    child.wait().unwrap();
    seen
}

/// The reader itself runs confined: `convert` becomes `pdftotext` or
/// pandoc in the same process, which then holds a seccomp filter (two: the
/// second answers `clone3`), no new privileges, and the readers' limits
/// (R21, #71): 2 GiB of address space and a core size of 1, which keeps a
/// crashed reader's core from a core handler; pandoc also has a 1 GiB heap.
#[test]
fn the_reader_holds_the_sandbox() {
    for (args, reader) in [
        (&["pdf-text"][..], "/usr/bin/pdftotext"),
        (&["document", "--format", "docx"][..], "/usr/bin/pandoc"),
    ] {
        let seen = running(args, reader);
        for mark in ["Seccomp:\t2", "Seccomp_filters:\t2", "NoNewPrivs:\t1"] {
            assert!(
                seen.status.lines().any(|l| l == mark),
                "{reader}: no {mark:?} in\n{}",
                seen.status
            );
        }
        for (limit, value) in [
            ("Max address space", "2147483648"),
            ("Max core file size", "1"),
        ] {
            let line = seen.limits.lines().find(|l| l.starts_with(limit));
            let words: Vec<_> = line.unwrap_or_default().split_whitespace().collect();
            assert!(
                words.ends_with(&[value, value, "bytes"]),
                "{reader}: {limit} is not {value} in\n{}",
                seen.limits
            );
        }
        if reader.ends_with("pandoc") {
            assert!(
                seen.cmdline.contains(" +RTS -M1g -RTS "),
                "{}",
                seen.cmdline
            );
        }
    }
}

/// A document built to exhaust memory (#71): 200 KB of Word that holds
/// 200 MiB of XML. Without a limit pandoc grows past a gigabyte and is
/// still reading at 20 s, and the server's 60 s limit is all that stops
/// it; with its heap held to 1 GiB it stops in about a second. The reader
/// is killed at 15 s, so a run without the limit fails here without
/// taking the machine's memory.
#[test]
fn a_document_built_to_fill_memory_stops_at_the_heap_limit() {
    let doc = std::fs::read(fixture("fills-memory.docx")).unwrap();
    let mut child = protonctl()
        .args(["convert", "document", "--format", "docx"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // pandoc may stop before it has read it all.
    let writer = std::thread::spawn(move || stdin.write_all(&doc).ok());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("pandoc was still reading at 15 s: no memory limit stopped it");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    writer.join().unwrap();
    let mut said = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut said).unwrap();
    assert!(said.contains("Heap exhausted"), "{status}: {said}");
    assert_ne!(status.code(), Some(SANDBOX_FAILED));
    assert!(!status.success());
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
