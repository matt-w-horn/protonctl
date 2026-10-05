//! `protonctl convert`: one document reader, run on stdin inside a sandbox
//! (RFC R21, Q34). On Linux, where the readers are poppler's `pdftotext`,
//! `pdfinfo` and `pdftoppm` and `pandoc`, the server starts this as a child
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
    /// A PDF's metadata, with its page count.
    PdfInfo,
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
const PDFINFO: Reader = Reader {
    path: "/usr/bin/pdfinfo",
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

/// Every reader, for `doctor`.
const READERS: [Reader; 4] = [PDFTOTEXT, PDFINFO, PDFTOPPM, PANDOC];

impl Job {
    /// The reader and its arguments; `None` for `Check`. Each reads stdin
    /// (`-`) and writes stdout, and pandoc's own `--sandbox` also keeps it
    /// from reading files or the network a document names.
    fn command(self) -> Option<(Reader, Vec<String>)> {
        let args = |a: &[&str]| a.iter().map(ToString::to_string).collect::<Vec<_>>();
        Some(match self {
            Self::Check => return None,
            Self::PdfText => (
                PDFTOTEXT,
                args(&["-enc", "UTF-8", "-eol", "unix", "-", "-"]),
            ),
            Self::PdfInfo => (PDFINFO, args(&["-"])),
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
                let a = ["--sandbox", "-f", from, "-t", "plain", "--wrap=none"];
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
            Self::PdfInfo => argv.push("pdf-info".into()),
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
        Some(SANDBOX_FAILED) => {
            bail!("the document readers' sandbox could not be set up; `protonctl doctor` says why")
        }
        _ => Ok(Outcome::Unreadable),
    }
}

/// For `protonctl doctor`: the sandbox can be entered, and which readers
/// are installed.
pub async fn check() -> Result<String> {
    let out = tokio::process::Command::new("/proc/self/exe")
        .args(Job::Check.argv())
        .env_clear()
        .stdin(std::process::Stdio::null())
        .output()
        .await?;
    if !out.status.success() {
        bail!(
            "the sandbox could not be set up: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut missing: Vec<&str> = READERS
        .iter()
        .filter(|r| !Path::new(r.path).exists())
        .map(|r| r.package)
        .collect();
    missing.dedup();
    Ok(if missing.is_empty() {
        "poppler-utils and pandoc found; the sandbox holds".into()
    } else {
        format!(
            "the sandbox holds; install {} to read {}",
            missing.join(" and "),
            if missing == ["pandoc"] {
                "Word, RTF and OpenDocument files"
            } else {
                "PDFs (poppler-utils) and Word, RTF and OpenDocument files (pandoc)"
            }
        )
    })
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
            Job::PdfInfo,
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
