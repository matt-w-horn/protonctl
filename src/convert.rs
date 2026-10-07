//! `protonctl convert`: one document reader, run on stdin inside a sandbox
//! (RFC R21, Q34). On Linux, where the readers are poppler's `pdftotext`
//! and `pdftoppm`, and `pandoc`, the server starts this as a child
//! for each document it reads. The child enters the sandbox, then becomes
//! the reader, so a hostile document that takes the reader over can read
//! only the system's own programs and libraries, write nothing, and reach
//! no network. The document goes in on stdin; the text or image comes back
//! on stdout. macOS runs its readers directly until Phase 4 (Q13).

use std::os::unix::process::CommandExt as _;
use std::path::Path;

use anyhow::{Result, bail};

use crate::extract::pipe;

/// The exit code of `protonctl convert` when it fails before the reader
/// runs: no reader exits with it (`pdftotext` uses 0 to 3 and 99, pandoc
/// none of 70), so the parent can tell "the sandbox failed" from "the
/// reader could not read this document". `EX_SOFTWARE` in sysexits.h.
pub const SANDBOX_FAILED: i32 = 70;

/// One run of a reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::Subcommand)]
pub enum Job {
    /// Enter the sandbox and exit: `doctor`'s check.
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
}

/// A document format pandoc reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum Format {
    Docx,
    Odt,
    Rtf,
}

/// A reader program, and the Debian and Ubuntu package that installs it.
#[derive(Clone, Copy, Debug)]
pub struct Reader {
    pub path: &'static str,
    pub package: &'static str,
}

const PDFTOTEXT: Reader = Reader {
    path: "/usr/bin/pdftotext",
    package: "poppler-utils",
};
const PDFTOPPM: Reader = Reader {
    path: "/usr/bin/pdftoppm",
    package: "poppler-utils",
};
const PANDOC: Reader = Reader {
    path: "/usr/bin/pandoc",
    package: "pandoc",
};

impl Job {
    /// The reader and its arguments; `None` for `Check`. Each reads stdin
    /// (`-`) and writes stdout, and pandoc's own `--sandbox` also keeps it
    /// from reading files or the network a document names. pandoc's heap is
    /// held to 1 GiB, as its manual advises for untrusted input, so a file
    /// built to fill memory stops at once with "Heap exhausted" (R21, #71).
    /// Measured with pandoc 3.1.3, a 2 MB Word file whose XML is 8 MB (5.7
    /// MB of text) needs 1 GiB and fails at 768 MiB.
    fn command(self) -> Option<(Reader, Vec<String>)> {
        let args = |a: &[&str]| a.iter().map(ToString::to_string).collect::<Vec<_>>();
        Some(match self {
            Self::Check => return None,
            Self::PdfText => (
                PDFTOTEXT,
                args(&["-enc", "UTF-8", "-eol", "unix", "-", "-"]),
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
                (PDFTOPPM, args(&a))
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
                (PANDOC, args(&a))
            }
        })
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
        }
        argv
    }
}

/// In the child: enter the sandbox, then become the reader. A reader job
/// returns only when it could not start; `Check` returns `Ok`.
pub fn run(job: Job) -> Result<()> {
    crate::platform::sandbox()?;
    let Some((reader, args)) = job.command() else {
        return Ok(());
    };
    let err = std::process::Command::new(reader.path)
        .args(args)
        .env_clear()
        .exec();
    bail!("cannot run {}: {err}", reader.path)
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
/// very binary, so a binary replaced while the server runs still converts.
/// Unit tests run as the test harness, which has no `convert`, so there the
/// reader runs directly, unsandboxed, on the tests' own files; the
/// integration tests in `tests/convert.rs` run it through the sandbox.
pub async fn read(job: Job, input: &[u8]) -> Result<Outcome> {
    let Some((reader, args)) = job.command() else {
        bail!("{job:?} reads nothing");
    };
    if !Path::new(reader.path).exists() {
        return Ok(Outcome::Missing(reader.package));
    }
    let (status, out) = if cfg!(test) {
        pipe(reader.path, &args, input).await?
    } else {
        pipe("/proc/self/exe", &job.argv(), input).await?
    };
    match status.code() {
        Some(0) => Ok(Outcome::Read(out)),
        Some(SANDBOX_FAILED) => Ok(Outcome::NoSandbox),
        _ => Ok(Outcome::Unreadable),
    }
}

/// Small documents for `doctor` to read: the two-page PDF the tests use,
/// and a one-word RTF.
const SAMPLE_PDF: &[u8] = include_bytes!("../tests/fixtures/two-pages.pdf");
const SAMPLE_RTF: &[u8] = b"{\\rtf1 protonctl}";

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
    ];
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
        let mut child = tokio::process::Command::new("/proc/self/exe")
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
            read.extend(reader.map(|r| r.path));
        } else if out.status.code() == Some(SANDBOX_FAILED) {
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
                format: Format::Odt,
            },
            Job::Document {
                format: Format::Rtf,
            },
        ] {
            let argv = job.argv();
            // The test's own name in place of `protonctl convert`.
            let parsed = Cli::try_parse_from(
                std::iter::once("t").chain(argv[1..].iter().map(String::as_str)),
            );
            assert_eq!(parsed.unwrap().job, job, "{argv:?}");
        }
    }
}
