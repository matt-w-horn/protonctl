//! The document readers (RFC R21, Q13, Q34), against the built binary:
//! `protonctl convert` runs each reader in its sandbox, and on Linux `drive
//! cat` reads documents through it as the server does. On Linux they need
//! poppler-utils, pandoc and tesseract-ocr; on macOS they are PDFKit and
//! `textutil`. The
//! fixtures: `two-pages.pdf` was written by hand, one Helvetica line per
//! page; the Word, OpenDocument and RTF files were made by pandoc 3.1.3
//! from "Word text, Café." and "Second paragraph.". `fills-memory.docx` was
//! made by Python's `zipfile`, deflated: a `[Content_Types].xml` and
//! `_rels/.rels` naming `word/document.xml`, which is one paragraph holding
//! one run of 200 MiB of "x". `ocr.png` is a page of two Helvetica 24 pt
//! lines, "Scanned invoice 4471" and "Paid in full, thank you.", written
//! by hand as a PDF and rendered by `pdftoppm -gray -r 100 -png`.
#![expect(
    clippy::unwrap_used,
    reason = "integration tests: a panic is a failed test"
)]

use std::io::Write as _;
#[cfg(target_os = "linux")]
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
    if cfg!(target_os = "linux") {
        for (tool, package) in [
            ("/usr/bin/pdftotext", "poppler-utils"),
            ("/usr/bin/pandoc", "pandoc"),
            ("/usr/bin/tesseract", "tesseract-ocr"),
        ] {
            assert!(
                Path::new(tool).exists(),
                "these tests need {package}: install it"
            );
        }
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

/// Each reader reads its fixture through `convert`, which on macOS is
/// `sandbox-exec` with the readers' profile and on Linux the sandboxed
/// process itself. The readers differ in detail: `pdftotext` ends a page
/// with blank lines before the form feed, where PDFKit does not, and pandoc
/// puts a blank line between paragraphs, where `textutil` does not.
#[test]
fn each_reader_runs_in_its_sandbox() {
    let pdf = std::fs::read(fixture("two-pages.pdf")).unwrap();
    let text = convert(&["pdf-text"], &pdf);
    assert!(text.status.success(), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    let pages = if cfg!(target_os = "macos") {
        "Café — accents.\u{C}Page two here.\u{C}"
    } else {
        "Café — accents.\n\n\u{C}Page two here."
    };
    assert!(text.contains(pages), "{text:?}");
    let page = convert(&["pdf-page", "--page", "2", "--edge", "300"], &pdf);
    assert!(page.status.success(), "{page:?}");
    assert!(page.stdout.starts_with(b"\xFF\xD8\xFF"), "{page:?}");
    assert!(page.stdout.ends_with(b"\xFF\xD9"), "{page:?}");
    let paragraphs = if cfg!(target_os = "macos") {
        "Word text, Café.\nSecond paragraph.\n"
    } else {
        "Word text, Café.\n\nSecond paragraph.\n"
    };
    for format in ["docx", "odt", "rtf"] {
        let doc = std::fs::read(fixture(&format!("text.{format}"))).unwrap();
        let out = convert(&["document", "--format", format], &doc);
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            paragraphs,
            "{format}"
        );
    }
    if cfg!(target_os = "linux") {
        let image = std::fs::read(fixture("ocr.png")).unwrap();
        let out = convert(&["ocr"], &image);
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "Scanned invoice 4471\nPaid in full, thank you.\n"
        );
    }
    assert!(convert(&["check"], b"").status.success());
}

/// A reader's /proc files, read while it waits on stdin once `convert`
/// has become it, in the same process.
#[cfg(target_os = "linux")]
struct Running {
    status: String,
    limits: String,
    cmdline: String,
    environ: String,
}

#[cfg(target_os = "linux")]
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
        environ: read("environ").replace('\0', " "),
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
/// Tesseract's environment holds only its one-thread limit.
#[cfg(target_os = "linux")]
#[test]
fn the_reader_holds_the_sandbox() {
    for (args, reader) in [
        (&["pdf-text"][..], "/usr/bin/pdftotext"),
        (&["document", "--format", "docx"][..], "/usr/bin/pandoc"),
        (&["ocr"][..], "/usr/bin/tesseract"),
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
        let environ = if reader.ends_with("tesseract") {
            "OMP_THREAD_LIMIT=1 "
        } else {
            ""
        };
        assert_eq!(seen.environ, environ, "{reader}");
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
#[cfg(target_os = "linux")]
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
/// unreadable file, and not the sandbox's, which it reports as an error:
/// `convert`'s 70, or on macOS `sandbox-exec`'s 65 and 71.
#[test]
fn a_damaged_pdf_fails_in_the_reader_not_the_sandbox() {
    for args in [
        &["pdf-text"][..],
        &["pdf-page", "--page", "1", "--edge", "100"],
    ] {
        let out = convert(args, b"%PDF-1.4 damaged");
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        assert_ne!(out.status.code(), Some(SANDBOX_FAILED));
    }
}

/// On macOS the readers run through `sandbox-exec`, whose own failures
/// exit 65 and 71: the server reports those as the sandbox's, not the
/// document's. A profile that names no reader it can run is such a failure.
#[cfg(target_os = "macos")]
#[test]
fn a_reader_the_profile_cannot_run_is_the_sandbox_s_failure() {
    let out = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", "(version 1)(deny default)", "/usr/bin/true"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(71), "{out:?}");
    let out = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", "(version 1)(deny default", "/usr/bin/true"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(65), "{out:?}");
}

/// The D-Bus session of the throwaway keyring that `scripts/check.sh`
/// starts. The keyring's folder is a new one under the temporary folder,
/// which tells it from the user's own, which this never writes to.
#[cfg(target_os = "linux")]
fn throwaway_keyring() -> String {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    assert!(
        data.is_some_and(|d| d.starts_with(std::env::temp_dir())),
        "run this through scripts/check.sh, which starts a throwaway keyring"
    );
    std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
}

/// Pin `cli` as `protonctl setup drive` does once the user confirms it: the
/// SHA-256 in the comment of the Secret Service item `protonctl`/
/// `drive-cli-pin` (RFC Q33, #72), as `src/platform/linux.rs` stores one.
#[cfg(target_os = "linux")]
fn pin_in_the_keyring(cli: &Path) {
    use secret_service::{EncryptionType, blocking::SecretService};
    let pin = ring::digest::digest(&ring::digest::SHA256, &std::fs::read(cli).unwrap());
    let pin = hex::encode(pin);
    let store = SecretService::connect(EncryptionType::Plain).unwrap();
    let attributes = std::collections::HashMap::from([
        ("service", "protonctl"),
        ("account", "drive-cli-pin"),
        ("comment", pin.as_str()),
    ]);
    store
        .get_default_collection()
        .unwrap()
        .create_item(
            "protonctl/drive-cli-pin",
            attributes,
            pin.as_bytes(),
            true,
            "text/plain",
        )
        .unwrap();
}

/// The whole path a server takes: the Drive CLI (a stand-in, pinned as
/// `setup drive` pins it) downloads the file, and protonctl reads it
/// through `/proc/self/exe convert`. The stand-in fails any call but the
/// ones a read makes, and records it, so the test fails on it (R1). It
/// runs from a sealed memfd, as `/proc/self/fd/N`, which a `#!/bin/sh`
/// script cannot, so it is one line that has `env -S` run the script.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs a Secret Service; scripts/check.sh runs it in a private D-Bus session"]
fn drive_reads_documents_through_the_sandboxed_readers() {
    let bus = throwaway_keyring();
    let home = tempfile::tempdir().unwrap();
    let cli = home.path().join("proton-drive");
    let script = home.path().join("proton-drive.sh");
    let unexpected = home.path().join("unexpected.txt");
    let fixtures = fixture("");
    std::fs::write(
        &script,
        format!(
            r#"shift
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
    let line = format!("#!/usr/bin/env -S /bin/sh {}\n", script.display());
    std::fs::write(&cli, line).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    pin_in_the_keyring(&cli);
    let config = home.path().join("config.toml");
    std::fs::write(&config, format!("[drive]\ncli = \"{}\"\n", cli.display())).unwrap();
    let cat = |args: &[&str]| -> serde_json::Value {
        let out = protonctl()
            .env("DBUS_SESSION_BUS_ADDRESS", &bus)
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
