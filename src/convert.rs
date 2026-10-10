//! `protonctl convert`: one document reader, run on stdin inside a sandbox
//! (RFC R21, Q13, Q34). The server starts this as a child for each document
//! it reads. The child confines itself, then becomes the reader, so a
//! hostile document that takes the reader over can read only the system's
//! own programs and libraries, write nothing, and reach no network and no
//! other process. The document goes in on stdin; the text or image comes
//! back on stdout. On macOS the readers are PDFKit, through a fixed script
//! in `osascript`, and `textutil`, under a `sandbox-exec` profile; on
//! Linux, poppler's `pdftotext` and `pdftoppm`, pandoc, and Tesseract for
//! the text in an image, under Landlock and seccomp. Both print a PDF's
//! text with a form feed after each page, and one page as a JPEG.

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use anyhow::{Result, bail};

use crate::extract::pipe;

/// The exit code of `protonctl convert` when it fails before the reader
/// runs: no reader exits with it (`pdftotext` uses 0 to 3 and 99, pandoc
/// none of 70, and the macOS readers 1 to 3), so the parent can tell "the
/// sandbox failed" from "the reader could not read this document".
/// `EX_SOFTWARE` in sysexits.h.
pub const SANDBOX_FAILED: i32 = 70;

/// On macOS, `sandbox-exec`'s own exits, before the reader runs: a profile
/// that does not compile (`EX_DATAERR`) and a reader it cannot run
/// (`EX_OSERR`), measured on macOS 27.
const SANDBOX_EXEC_FAILED: [i32; 2] = [65, 71];

/// A program to run under the sandbox for `Check`, which reads nothing.
const TRUE: &str = "/usr/bin/true";

/// Whether a `convert` child failed in the sandbox rather than in the
/// reader.
pub fn sandbox_failed(status: ExitStatus) -> bool {
    status.code().is_some_and(|code| {
        code == SANDBOX_FAILED || (cfg!(target_os = "macos") && SANDBOX_EXEC_FAILED.contains(&code))
    })
}

/// One run of a reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum Job {
    /// Enter the sandbox and run `true` in it: `doctor`'s check.
    Check,
    /// A PDF's text, each page followed by a form feed.
    PdfText,
    /// One page of a PDF as a JPEG.
    PdfPage {
        /// Counted from 1.
        #[arg(long)]
        page: usize,
        /// The long edge, in pixels.
        #[arg(long)]
        edge: u32,
    },
    /// A Word, OpenDocument or RTF document's text.
    Document {
        #[arg(long, value_enum)]
        format: Format,
    },
    /// The text in a PNG, JPEG, GIF or WebP image, by OCR (M4.2). Linux
    /// only: Vision on macOS is not built.
    #[cfg(target_os = "linux")]
    Ocr,
}

/// A document format the readers take. `textutil` reads the old binary
/// Word format too; pandoc does not, and `extract` says so before asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum Format {
    Docx,
    Doc,
    Odt,
    Rtf,
}

/// A reader program, and what installs it: on Linux the Debian and Ubuntu
/// package; on macOS the system, which always has it.
#[derive(Clone, Copy, Debug)]
pub struct Reader {
    pub path: &'static str,
    pub package: &'static str,
}

#[cfg(target_os = "linux")]
const PDFTOTEXT: Reader = Reader {
    path: "/usr/bin/pdftotext",
    package: "poppler-utils",
};
#[cfg(target_os = "linux")]
const PDFTOPPM: Reader = Reader {
    path: "/usr/bin/pdftoppm",
    package: "poppler-utils",
};
#[cfg(target_os = "linux")]
const PANDOC: Reader = Reader {
    path: "/usr/bin/pandoc",
    package: "pandoc",
};
#[cfg(target_os = "linux")]
const TESSERACT: Reader = Reader {
    path: "/usr/bin/tesseract",
    package: "tesseract-ocr",
};
#[cfg(target_os = "macos")]
const OSASCRIPT: Reader = Reader {
    path: "/usr/bin/osascript",
    package: "macOS",
};
#[cfg(target_os = "macos")]
const TEXTUTIL: Reader = Reader {
    path: "/usr/bin/textutil",
    package: "macOS",
};

/// Every page's text, read by PDFKit from stdin, each followed by a form
/// feed as `pdftotext` prints them. Exits 1 for a file PDFKit cannot open
/// and 2 for one that needs a password.
#[cfg(target_os = "macos")]
const PDF_TEXT_SCRIPT: &str = r#"ObjC.import("PDFKit");
ObjC.import("stdlib");
function run() {
  const data = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
  const doc = $.PDFDocument.alloc.initWithData(data);
  if (doc.isNil()) $.exit(1);
  if (doc.isLocked) $.exit(2);
  let text = "";
  for (let i = 0; i < doc.pageCount; i++) text += (ObjC.unwrap(doc.pageAtIndex(i).string) || "") + "\f";
  $.NSFileHandle.fileHandleWithStandardOutput.writeData($(text).dataUsingEncoding($.NSUTF8StringEncoding));
}"#;

/// Page `argv[0]` (counted from 1) rendered by PDFKit with its long edge
/// `argv[1]` pixels, written to stdout as JPEG. Exits 1 and 2 as the text
/// script does, and 3 for a page the PDF does not have.
#[cfg(target_os = "macos")]
const PDF_PAGE_SCRIPT: &str = r#"ObjC.import("PDFKit");
ObjC.import("AppKit");
ObjC.import("stdlib");
function run(argv) {
  const [page, edge] = argv.map(Number);
  const doc = $.PDFDocument.alloc.initWithData($.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile);
  if (doc.isNil()) $.exit(1);
  if (doc.isLocked) $.exit(2);
  if (!(page >= 1 && page <= doc.pageCount)) $.exit(3);
  const p = doc.pageAtIndex(page - 1);
  const box = p.boundsForBox($.kPDFDisplayBoxCropBox).size;
  const scale = edge / Math.max(box.width, box.height);
  const size = $.NSMakeSize(Math.round(box.width * scale), Math.round(box.height * scale));
  const image = p.thumbnailOfSizeForBox(size, $.kPDFDisplayBoxCropBox);
  const bitmap = $.NSBitmapImageRep.imageRepWithData(image.TIFFRepresentation);
  const jpeg = bitmap.representationUsingTypeProperties($.NSBitmapImageFileTypeJPEG, $({ NSImageCompressionFactor: 0.85 }));
  $.NSFileHandle.fileHandleWithStandardOutput.writeData(jpeg);
}"#;

fn strings(a: &[&str]) -> Vec<String> {
    a.iter().map(ToString::to_string).collect()
}

impl Job {
    /// The reader and its arguments; `None` for `Check`. Each reads stdin
    /// (`-`) and writes stdout, and pandoc's own `--sandbox` also keeps it
    /// from reading files or the network a document names. pandoc's heap is
    /// held to 1 GiB, as its manual advises for untrusted input, so a file
    /// built to fill memory stops at once with "Heap exhausted" (R21, #71).
    /// Measured with pandoc 3.1.3, a 2 MB Word file whose XML is 8 MB (5.7
    /// MB of text) needs 1 GiB and fails at 768 MiB.
    #[cfg(target_os = "linux")]
    fn command(self) -> Option<(Reader, Vec<String>)> {
        Some(match self {
            Self::Check => return None,
            Self::PdfText => (
                PDFTOTEXT,
                strings(&["-enc", "UTF-8", "-eol", "unix", "-", "-"]),
            ),
            Self::PdfPage { page, edge } => {
                let page = page.to_string();
                let edge = edge.to_string();
                let a = [
                    "-f",
                    &page,
                    "-l",
                    &page,
                    "-singlefile",
                    "-jpeg",
                    "-jpegopt",
                    "quality=85",
                    "-scale-to",
                    &edge,
                    "-",
                ];
                (PDFTOPPM, strings(&a))
            }
            Self::Document { format } => {
                let from: &str = format.into();
                let a = [
                    "+RTS",
                    "-M1g",
                    "-RTS",
                    "--sandbox",
                    "-f",
                    from,
                    "-t",
                    "plain",
                    "--wrap=none",
                ];
                (PANDOC, strings(&a))
            }
            // Its own language data, English only, since no other is
            // named with `-l`; `stdin` and `stdout` are its words for them.
            Self::Ocr => (TESSERACT, strings(&["stdin", "stdout"])),
        })
    }

    /// The reader and its arguments; `None` for `Check`. PDFKit runs in
    /// `osascript` from a script given on the command line, and `textutil`
    /// reads stdin; each writes stdout.
    #[cfg(target_os = "macos")]
    fn command(self) -> Option<(Reader, Vec<String>)> {
        Some(match self {
            Self::Check => return None,
            Self::PdfText => (
                OSASCRIPT,
                strings(&["-l", "JavaScript", "-e", PDF_TEXT_SCRIPT]),
            ),
            Self::PdfPage { page, edge } => {
                let page = page.to_string();
                let edge = edge.to_string();
                let a = ["-l", "JavaScript", "-e", PDF_PAGE_SCRIPT, &page, &edge];
                (OSASCRIPT, strings(&a))
            }
            Self::Document { format } => {
                let format: &str = format.into();
                let a = [
                    "-stdin",
                    "-stdout",
                    "-convert",
                    "txt",
                    "-encoding",
                    "UTF-8",
                    "-format",
                    format,
                ];
                (TEXTUTIL, strings(&a))
            }
        })
    }

    /// The reader's environment, which is otherwise empty. Tesseract runs
    /// one thread: with OpenMP's default of one per core, eight runs at once
    /// took 147 s here, where one took 0.3 s; with one thread each, eight
    /// took 0.7 s (4 cores, 2026-10-10). Calls run at once, and beside the
    /// name model's threads.
    fn env(self) -> &'static [(&'static str, &'static str)] {
        match self {
            #[cfg(target_os = "linux")]
            Self::Ocr => &[("OMP_THREAD_LIMIT", "1")],
            _ => &[],
        }
    }

    /// This job as `protonctl` arguments.
    fn argv(self) -> Vec<String> {
        let mut argv = vec!["convert".to_string()];
        match self {
            Self::Check => argv.push("check".into()),
            Self::PdfText => argv.push("pdf-text".into()),
            Self::PdfPage { page, edge } => argv.extend([
                "pdf-page".into(),
                "--page".into(),
                page.to_string(),
                "--edge".into(),
                edge.to_string(),
            ]),
            Self::Document { format } => {
                let format: &str = format.into();
                argv.extend(["document".into(), "--format".into(), format.into()]);
            }
            #[cfg(target_os = "linux")]
            Self::Ocr => argv.push("ocr".into()),
        }
        argv
    }
}

/// In the child: confine the reader, then become it. Returns only when it
/// could not start.
pub fn run(job: Job) -> Result<()> {
    let (program, args) = match job.command() {
        Some((reader, args)) => (reader.path, args),
        None => (TRUE, Vec::new()),
    };
    let err = crate::platform::confine(program)?
        .args(args)
        .env_clear()
        .envs(job.env().iter().copied())
        .exec();
    bail!("cannot run {program}: {err}")
}

/// This program, to start `protonctl convert` from. On Linux
/// `/proc/self/exe` is this very program even once its file is replaced;
/// macOS keeps no such handle, so a server whose binary was deleted reads
/// no more documents until it is restarted.
fn own_program() -> Result<PathBuf> {
    if cfg!(target_os = "linux") {
        Ok(PathBuf::from("/proc/self/exe"))
    } else {
        Ok(std::env::current_exe()?)
    }
}

/// What a reader made of one input.
#[derive(Debug)]
pub enum Outcome {
    /// What it printed.
    Read(Vec<u8>),
    /// It could not read this input: a damaged file, or one that needs a
    /// password.
    Unreadable,
    /// It is not installed; this package installs it.
    Missing(&'static str),
    /// It could not run in the sandbox here; `doctor` says why.
    NoSandbox,
}

/// In the parent: run `job` on `input` in the sandbox, as a child of this
/// very binary. Unit tests run as the test harness, which has no
/// `convert`, so there the reader runs directly, unsandboxed, on the tests'
/// own files; the integration tests in `tests/convert.rs` run it through
/// the sandbox.
pub async fn read(job: Job, input: &[u8]) -> Result<Outcome> {
    let Some((reader, args)) = job.command() else {
        bail!("{job:?} reads nothing");
    };
    if !Path::new(reader.path).exists() {
        return Ok(Outcome::Missing(reader.package));
    }
    let (status, out) = if cfg!(test) {
        pipe(Path::new(reader.path), &args, job.env(), input).await?
    } else {
        pipe(&own_program()?, &job.argv(), &[], input).await?
    };
    if status.success() {
        Ok(Outcome::Read(out))
    } else if sandbox_failed(status) {
        Ok(Outcome::NoSandbox)
    } else {
        Ok(Outcome::Unreadable)
    }
}

/// Small documents for `doctor` to read: the two-page PDF the tests use,
/// a one-word RTF, and on Linux the tests' image of two lines of text.
const SAMPLE_PDF: &[u8] = include_bytes!("../tests/fixtures/two-pages.pdf");
const SAMPLE_RTF: &[u8] = b"{\\rtf1 protonctl}";
#[cfg(target_os = "linux")]
const SAMPLE_IMAGE: &[u8] = include_bytes!("../tests/fixtures/ocr.png");

/// For `protonctl doctor`: the sandbox can be entered, and each installed
/// reader reads a sample inside it, as a call would; a failure quotes the
/// child's own words, which can say why a reader cannot start.
pub async fn check() -> Result<String> {
    let jobs = [
        (Job::Check, &b""[..]),
        (Job::PdfText, SAMPLE_PDF),
        (Job::PdfPage { page: 1, edge: 100 }, SAMPLE_PDF),
        (
            Job::Document {
                format: Format::Rtf,
            },
            SAMPLE_RTF,
        ),
        #[cfg(target_os = "linux")]
        (Job::Ocr, SAMPLE_IMAGE),
    ];
    let program = own_program()?;
    let (mut read, mut missing, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for (job, sample) in jobs {
        let reader = job.command().map(|(r, _)| r);
        if let Some(r) = reader
            && !Path::new(r.path).exists()
        {
            missing.push(r.package);
            continue;
        }
        let name = reader.map_or("the sandbox", |r| r.path);
        let mut child = tokio::process::Command::new(&program)
            .args(job.argv())
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt as _;
            stdin.write_all(sample).await?;
        }
        let out = child.wait_with_output().await?;
        if out.status.success() && (reader.is_none() || !out.stdout.is_empty()) {
            read.extend(reader.map(|r| r.path).filter(|p| !read.contains(p)));
        } else if sandbox_failed(out.status) {
            let said = String::from_utf8_lossy(&out.stderr);
            failed.push(format!("{name} cannot run in the sandbox: {}", said.trim()));
        } else {
            failed.push(format!(
                "{name} could not read its sample in the sandbox ({})",
                out.status
            ));
        }
    }
    if !failed.is_empty() {
        bail!("{}", failed.join("; "));
    }
    missing.dedup();
    let mut said = if read.is_empty() {
        "the sandbox holds".to_string()
    } else {
        format!("{} each read a sample in the sandbox", read.join(", "))
    };
    if !missing.is_empty() {
        said = format!("{said}; install {} to read the rest", missing.join(" and "));
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    /// What the parent passes is what the child parses.
    #[test]
    fn jobs_round_trip_through_their_arguments() {
        #[derive(clap::Parser)]
        struct Cli {
            #[command(subcommand)]
            job: Job,
        }
        for job in [
            Job::Check,
            Job::PdfText,
            Job::PdfPage {
                page: 3,
                edge: 2000,
            },
            Job::Document {
                format: Format::Docx,
            },
            Job::Document {
                format: Format::Doc,
            },
            Job::Document {
                format: Format::Odt,
            },
            Job::Document {
                format: Format::Rtf,
            },
            #[cfg(target_os = "linux")]
            Job::Ocr,
        ] {
            let argv = job.argv();
            // The test's own name in place of `protonctl convert`.
            let parsed = Cli::try_parse_from(
                std::iter::once("t").chain(argv[1..].iter().map(String::as_str)),
            );
            assert_eq!(parsed.unwrap().job, job, "{argv:?}");
        }
    }

    /// The sandbox's own exits are told from a reader's: `convert`'s 70 on
    /// both systems, and on macOS `sandbox-exec`'s 65 and 71; a reader's 1
    /// to 3 are the document's fault.
    #[test]
    fn a_sandbox_failure_is_told_from_a_reader_s() {
        use std::os::unix::process::ExitStatusExt as _;
        let exit = |code: i32| ExitStatus::from_raw(code << 8);
        assert!(sandbox_failed(exit(SANDBOX_FAILED)));
        for code in [0, 1, 2, 3, 99] {
            assert!(!sandbox_failed(exit(code)), "{code}");
        }
        for code in SANDBOX_EXEC_FAILED {
            assert_eq!(sandbox_failed(exit(code)), cfg!(target_os = "macos"));
        }
        // A signal is no sandbox failure either.
        assert!(!sandbox_failed(ExitStatus::from_raw(9)));
    }
}
