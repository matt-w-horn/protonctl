//! Text out of a file's bytes, for the tools that return content inline, and
//! pages of that text. Text is read as UTF-8 or, with a byte-order mark,
//! UTF-16, and otherwise as Windows-1252, flagged as a guess. PDF, Word,
//! RTF and OpenDocument text comes from the document readers, each run by
//! `protonctl convert` in a sandbox (R21, Q13, Q34): on macOS PDFKit
//! through `/usr/bin/osascript` and `/usr/bin/textutil`, both part of
//! macOS; on Linux poppler and pandoc. Every reader takes the document on
//! stdin and runs as its own process, so a hostile document can crash only
//! it. Images are named for the host to show, and a PDF's pages are
//! rendered to images by the same readers: a scan has no text layer, and
//! Claude reads a page image as it reads any other. Aliases mode returns no
//! images (R22): on Linux an image, and a scan's pages, go through OCR
//! (Tesseract) instead, and their text is read as any other (M4.2).

use std::fmt::Write as _;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::content::{Attached, clean};
use crate::convert::{Format, Job, Outcome};

/// Largest file read for its text or shown as an image.
pub const MAX_SOURCE: u64 = 64 << 20;
/// Largest image returned inline, the most Claude takes as one image.
pub const MAX_IMAGE: usize = 5 << 20;
/// Most text a document may yield; a zip bomb in a .docx stops here.
const MAX_TEXT: usize = 32 << 20;
/// How long a reader may take over one document.
const HELPER_LIMIT: Duration = Duration::from_secs(60);
/// Characters a page of text holds unless the caller asks otherwise: a
/// result with 100,000 was measured at 110,496 characters of JSON, over
/// Claude Code's 25,000-token limit.
pub const PAGE_CHARS: usize = 20_000;
/// Most characters one page may hold.
pub const MAX_PAGE_CHARS: usize = 40_000;
/// PDF pages one call returns as images: within the 20 images claude.ai
/// takes in a message, with room for the conversation's others.
const PAGE_IMAGES: usize = 4;
/// The long edge of a rendered page, in pixels: 2000, the most any image may
/// have once a request holds over 20, and between the 1568 that older
/// models scale down to and the 2576 that current ones read.
const PAGE_EDGE: u32 = 2000;
/// A scan's pages that OCR reads in one call, in aliases mode: each is
/// rendered and then read, two sandboxed runs, so a long scan stays within
/// the call's 150 s (R8). A dense page (55 lines of 10 pt text) took 1.8 s
/// to read and 0.05 s to render on one thread (2026-10-10), so 10 pages
/// take about 18 s one after another; `scan_text` reads `OCR_AT_ONCE` at
/// a time.
#[cfg(target_os = "linux")]
const OCR_PAGES: usize = 10;
/// A scan's pages rendered and read at once, each by one-thread runs.
#[cfg(target_os = "linux")]
const OCR_AT_ONCE: usize = 4;

/// What a file's bytes hold, as far as the tools can return it inline.
pub enum Content {
    Text(Document),
    Image {
        mime: &'static str,
    },
    /// Neither text nor an image; the reason says what was tried.
    Other(&'static str),
}

/// How a document's text was read, as `textFrom` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum TextFrom {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-16")]
    Utf16,
    #[serde(rename = "windows-1252 (guessed: the file is not UTF-8)")]
    Windows1252Guessed,
    #[serde(rename = "pdf")]
    Pdf,
    #[serde(rename = "textutil")]
    Textutil,
    #[serde(rename = "pandoc")]
    Pandoc,
    /// A mail body, already decoded by the mail parser.
    #[serde(rename = "message")]
    Message,
    /// The text OCR found in an image or a scan's pages (M4.2).
    #[cfg(target_os = "linux")]
    #[serde(rename = "tesseract (OCR)")]
    Tesseract,
}

/// A document's text, ready to page: hidden characters removed (R6), and
/// for a PDF its pages joined by form feeds, with where each one starts.
pub struct Document {
    text: String,
    chars: usize,
    from: TextFrom,
    /// The character offset of each PDF page; empty for other files.
    page_starts: Vec<usize>,
    hidden: usize,
    /// A PDF, kept to render its pages as images.
    pdf: Option<Pdf>,
    /// Where the text came from, when the reader cannot say it in
    /// `textFrom` alone: a scan read by OCR, and how much of it.
    notice: Option<String>,
}

/// A PDF's bytes, and how many pages its text came from: the count the
/// reader found, which metadata inside the file cannot change.
struct Pdf {
    bytes: Vec<u8>,
    pages: usize,
}

impl Document {
    /// `pages` of text, each cleaned of hidden characters, joined by '\f'.
    pub fn new(pages: &[String], from: TextFrom) -> Self {
        let (mut text, mut page_starts, mut hidden) = (String::new(), Vec::new(), 0);
        let mut chars = 0;
        for (i, page) in pages.iter().enumerate() {
            if i > 0 {
                text.push('\u{C}');
                chars += 1;
            }
            page_starts.push(chars);
            let page = clean(page, &mut hidden);
            chars += page.chars().count();
            text.push_str(&page);
        }
        if pages.len() < 2 {
            page_starts.clear();
        }
        Self {
            text,
            chars,
            from,
            page_starts,
            hidden,
            pdf: None,
            notice: None,
        }
    }

    /// What a read returns: a PDF's pages as images from `page` (counted
    /// from 1) when one is asked for, or when it has no text layer, as a scan
    /// has not; else a page of the text from `offset`.
    pub async fn read(
        &self,
        offset: Option<usize>,
        max_chars: Option<usize>,
        page: Option<usize>,
    ) -> Result<(Value, Vec<Attached>)> {
        let blank = self.text.trim().is_empty();
        match &self.pdf {
            // Aliases mode returns no images (R22), so none is made.
            Some(_) if blank && !crate::content::images_allowed() => Ok((
                json!({ "content": null, "textLayer": false, "reason": SCAN_UNREAD }),
                Vec::new(),
            )),
            Some(pdf) if page.is_some() || blank => {
                let (mut v, images) = page_images(pdf, page.unwrap_or(1)).await?;
                if blank {
                    v["textLayer"] = json!(false);
                }
                Ok((v, images))
            }
            None if page.is_some() => bail!("page applies to PDFs; other text is read by offset"),
            _ => Ok((self.page(offset, max_chars)?, Vec::new())),
        }
    }

    /// Hidden characters removed from the whole text (R6).
    pub fn hidden(&self) -> usize {
        self.hidden
    }

    /// The characters from `offset`, at most `max_chars` of them (the
    /// defaults when `None`), with what a caller needs to read on: the
    /// offset of the next page, or null at the end, and the whole length.
    pub fn page(&self, offset: Option<usize>, max_chars: Option<usize>) -> Result<Value> {
        let offset = offset.unwrap_or(0);
        if offset > self.chars {
            bail!(
                "offset {offset} is past the end of the text ({} characters)",
                self.chars
            );
        }
        let max = max_chars.unwrap_or(PAGE_CHARS).clamp(1, MAX_PAGE_CHARS);
        let byte = |n: usize| {
            self.text
                .char_indices()
                .nth(n)
                .map_or(self.text.len(), |(i, _)| i)
        };
        // A caller can ask for any offset: aliases mode moves a start inside
        // a name or address back to where it begins (RFC R13).
        let offset = self.text[..crate::content::start_at(&self.text, byte(offset))]
            .chars()
            .count();
        let mut end = offset.saturating_add(max).min(self.chars);
        if end < self.chars {
            // Aliases mode moves the edge off a name or address (RFC R13).
            let (from, at) = (byte(offset), byte(end));
            let cut = crate::content::cut_at(&self.text, at, from);
            if cut != at {
                end = offset + self.text[from..cut].chars().count();
            }
        }
        let slice = &self.text[byte(offset)..byte(end)];
        let next = (end < self.chars).then_some(end);
        let mut v = json!({
            "content": slice,
            "offset": offset,
            "nextOffset": next,
            "totalChars": self.chars,
            "truncated": next.is_some(),
            "textFrom": self.from,
            "hiddenCharactersRemoved": self.hidden,
        });
        if let Some(next) = next {
            v["note"] = json!(format!(
                "more text follows: call again with offset {next} to read on"
            ));
        }
        if !self.page_starts.is_empty() {
            v["pdfPages"] = json!(self.page_starts.len());
            v["pageStarts"] = json!(self.page_starts);
        }
        if self.text.trim().is_empty() {
            v["note"] = json!("the file holds no text");
        }
        if let Some(notice) = &self.notice {
            v["note"] = json!(match v["note"].as_str() {
                Some(note) => format!("{notice}; {note}"),
                None => notice.clone(),
            });
        }
        Ok(v)
    }
}

/// What `bytes`, from a file called `name`, hold. Images and PDFs are known
/// by their first bytes, Word, RTF and OpenDocument files by the name's
/// extension, and text by its encoding.
pub async fn content(bytes: &[u8], name: &str) -> Result<Content> {
    if let Some(mime) = image_type(bytes) {
        #[cfg(target_os = "linux")]
        if !crate::content::images_allowed() {
            return Ok(match ocr(bytes).await? {
                Ok(text) => {
                    let mut doc = Document::new(&[text], TextFrom::Tesseract);
                    doc.notice = Some("an image: its text is what OCR found in it".into());
                    Content::Text(doc)
                }
                Err(why) => Content::Other(why),
            });
        }
        return Ok(Content::Image { mime });
    }
    if bytes.starts_with(b"%PDF-") {
        return pdf_text(bytes).await;
    }
    let ext = name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase());
    if let Some(format) = ext
        .as_deref()
        .filter(|x| ["docx", "doc", "rtf", "odt"].contains(x))
    {
        return document_text(bytes, format).await;
    }
    Ok(match decode(bytes) {
        Some((text, from)) => Content::Text(Document::new(&[text], from)),
        None => Content::Other(
            "the file is not text, a PDF, a Word, RTF or OpenDocument document, or an image",
        ),
    })
}

/// PNG, JPEG, GIF and WebP, the image types Claude reads.
fn image_type(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Why a reader that cannot run in its sandbox here reads nothing.
const NO_SANDBOX: &str =
    "the document readers cannot run in their sandbox on this computer; protonctl doctor says why";

/// Why a scan returns no text in aliases mode.
const SCAN_UNREAD: &str = if cfg!(target_os = "linux") {
    "the PDF has no text layer (a scan), and OCR found no text in it; aliases mode returns no page images"
} else {
    "the PDF has no text layer (a scan); aliases mode returns no page images until Phase 4 reads their text"
};

/// Why a reader that is not installed reads nothing.
fn missing(package: &str) -> &'static str {
    match package {
        "tesseract-ocr" => {
            "reading the text in images and scans here needs tesseract-ocr; install it (RFC-0001 M4.2)"
        }
        "pandoc" => {
            "reading Word, RTF and OpenDocument files here needs pandoc; install it (RFC-0001 Q34)"
        }
        "poppler-utils" => "reading PDFs here needs poppler-utils; install it (RFC-0001 Q34)",
        _ => "a document reader that is part of the system is missing; protonctl doctor says which",
    }
}

/// A PDF's text from the PDF reader, which ends every page with a form
/// feed.
async fn pdf_text(bytes: &[u8]) -> Result<Content> {
    let out = match crate::convert::read(Job::PdfText, bytes).await? {
        Outcome::Read(out) => out,
        Outcome::Unreadable => {
            return Ok(Content::Other(
                "the PDF reader could not read the PDF; it may be damaged or need a password",
            ));
        }
        Outcome::Missing(package) => return Ok(Content::Other(missing(package))),
        Outcome::NoSandbox => return Ok(Content::Other(NO_SANDBOX)),
    };
    let text = String::from_utf8(out).context("the PDF reader did not return UTF-8")?;
    let pages = pages_of(&text);
    let pdf = Pdf {
        bytes: bytes.to_vec(),
        pages: pages.len(),
    };
    #[cfg(target_os = "linux")]
    if text.trim().is_empty() && !crate::content::images_allowed() {
        return scan_text(pdf).await;
    }
    let mut doc = Document::new(&pages, TextFrom::Pdf);
    doc.pdf = Some(pdf);
    Ok(Content::Text(doc))
}

/// The text in `image`, read by OCR in the sandbox, or why there is none.
/// Tesseract 5.3 ends its text with a newline; trailing newlines and form
/// feeds are dropped.
#[cfg(target_os = "linux")]
async fn ocr(image: &[u8]) -> Result<Result<String, &'static str>> {
    Ok(match crate::convert::read(Job::Ocr, image).await? {
        Outcome::Read(out) => {
            let text = String::from_utf8(out).context("OCR did not return UTF-8")?;
            Ok(text.trim_end_matches(['\u{C}', '\n']).to_string())
        }
        Outcome::Unreadable => Err("OCR could not read the image; it may be damaged or too large"),
        Outcome::Missing(package) => Err(missing(package)),
        Outcome::NoSandbox => Err(NO_SANDBOX),
    })
}

/// A scan's text in aliases mode, where its pages cannot come as images
/// (R22): its first `OCR_PAGES` pages, each rendered and read by OCR.
#[cfg(target_os = "linux")]
async fn scan_text(pdf: Pdf) -> Result<Content> {
    use futures::{StreamExt as _, TryStreamExt as _};
    let read = pdf.pages.min(OCR_PAGES);
    let texts: Vec<Result<String, &'static str>> = futures::stream::iter(1..=read)
        .map(|page| {
            let pdf = &pdf;
            async move { ocr(&page_jpeg(pdf, page).await?).await }
        })
        .buffered(OCR_AT_ONCE)
        .try_collect()
        .await?;
    let pages = match texts.into_iter().collect::<Result<Vec<String>, _>>() {
        Ok(pages) => pages,
        Err(why) => return Ok(Content::Other(why)),
    };
    let mut doc = Document::new(&pages, TextFrom::Tesseract);
    doc.notice = Some(if read < pdf.pages {
        format!(
            "the PDF has no text layer (a scan): its text is what OCR found in its first {read} of {} pages",
            pdf.pages
        )
    } else {
        "the PDF has no text layer (a scan): its text is what OCR found in it".into()
    });
    doc.pdf = Some(pdf);
    Ok(Content::Text(doc))
}

/// The pages of the PDF reader's output, without the form feed after the
/// last one or the blank lines `pdftotext` puts at the end of each.
fn pages_of(text: &str) -> Vec<String> {
    let text = text.strip_suffix('\u{C}').unwrap_or(text);
    text.split('\u{C}')
        .map(|page| page.trim_end_matches('\n').to_string())
        .collect()
}

/// A Word, OpenDocument or RTF document's text from the document reader:
/// `textutil` on macOS, which reads the old binary Word format too, and
/// pandoc on Linux, which does not.
async fn document_text(bytes: &[u8], extension: &str) -> Result<Content> {
    let format = match extension {
        "docx" => Format::Docx,
        "odt" => Format::Odt,
        "rtf" => Format::Rtf,
        "doc" if cfg!(target_os = "macos") => Format::Doc,
        _ => {
            return Ok(Content::Other(
                "pandoc cannot read the old Word format (.doc); a copy saved as .docx can be read",
            ));
        }
    };
    let from = if cfg!(target_os = "macos") {
        TextFrom::Textutil
    } else {
        TextFrom::Pandoc
    };
    let out = match crate::convert::read(Job::Document { format }, bytes).await? {
        Outcome::Read(out) => out,
        Outcome::Unreadable => {
            return Ok(Content::Other(
                "the document reader could not read the document",
            ));
        }
        Outcome::Missing(package) => return Ok(Content::Other(missing(package))),
        Outcome::NoSandbox => return Ok(Content::Other(NO_SANDBOX)),
    };
    let text = String::from_utf8(out).context("the document reader did not return UTF-8")?;
    Ok(Content::Text(Document::new(&[text], from)))
}

/// Up to `PAGE_IMAGES` pages of `pdf` from `first` as JPEGs from the PDF
/// reader, one sandboxed run each.
async fn pdf_pages(pdf: &Pdf, first: usize) -> Result<Vec<Vec<u8>>> {
    if first > pdf.pages {
        return Ok(Vec::new());
    }
    let mut jpegs = Vec::new();
    for page in first..=pdf.pages.min(first.saturating_add(PAGE_IMAGES - 1)) {
        jpegs.push(page_jpeg(pdf, page).await?);
    }
    Ok(jpegs)
}

/// Page `page` of `pdf` as a JPEG `PAGE_EDGE` pixels on its long edge,
/// from the PDF reader.
async fn page_jpeg(pdf: &Pdf, page: usize) -> Result<Vec<u8>> {
    let job = Job::PdfPage {
        page,
        edge: PAGE_EDGE,
    };
    let jpeg = match crate::convert::read(job, &pdf.bytes).await? {
        Outcome::Read(out) => out,
        Outcome::Unreadable => bail!("the PDF reader could not read the PDF"),
        Outcome::Missing(package) => bail!("{}", missing(package)),
        Outcome::NoSandbox => bail!("{NO_SANDBOX}"),
    };
    if !jpeg.starts_with(b"\xFF\xD8\xFF") {
        bail!("the PDF reader did not return a JPEG for page {page}");
    }
    Ok(jpeg)
}

/// Pages of the PDF `bytes` from `first` (counted from 1) as JPEG images,
/// `PAGE_IMAGES` at most, each after a "Page N:" label, and the JSON that
/// says which pages they are and where the next call starts.
async fn page_images(pdf: &Pdf, first: usize) -> Result<(Value, Vec<Attached>)> {
    if first == 0 {
        bail!("page counts from 1");
    }
    let (total, jpegs) = (pdf.pages, pdf_pages(pdf, first).await?);
    if first > total {
        bail!("page {first} is past the PDF's {total} pages");
    }
    let images: Vec<Attached> = jpegs
        .into_iter()
        .enumerate()
        .map(|(i, bytes)| Attached::Image {
            mime: "image/jpeg",
            bytes,
            label: Some(format!("Page {}:", first + i)),
        })
        .collect();
    if images.is_empty() {
        bail!("the PDF reader rendered no page");
    }
    let last = first + images.len() - 1;
    let next = (last < total).then_some(last + 1);
    let mut v = json!({
        "pdfPages": total,
        "pagesShown": [first, last],
        "nextPage": next,
        "note": format!("pages {first} to {last} of {total} follow as images, each after its \"Page N:\" label"),
    });
    if let Some(next) = next {
        v["note"] = json!(format!(
            "pages {first} to {last} of {total} follow as images, each after its \"Page N:\" label; call again with page {next} to see on"
        ));
    }
    Ok((v, images))
}

/// Run `program` on `input` with only `env` set, and return how it
/// exited and what it printed, at most `MAX_TEXT` bytes, within
/// `HELPER_LIMIT`. `kill_on_drop` ends it when the call is cut off.
pub async fn pipe<S: AsRef<std::ffi::OsStr>>(
    program: &std::path::Path,
    args: &[S],
    env: &[(&str, &str)],
    input: &[u8],
) -> Result<(std::process::ExitStatus, Vec<u8>)> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("cannot run {}", program.display()))?;
    let program = program.display();
    let mut stdin = child.stdin.take().context("no stdin")?;
    let stdout = child.stdout.take().context("no stdout")?;
    // Written by its own task while the output is read, so neither pipe
    // fills and stalls, and a read that stops at the cap never waits on the
    // write; killing the helper ends the write with a broken pipe.
    let input = input.to_vec();
    tokio::spawn(async move {
        stdin.write_all(&input).await.ok();
    });
    let work = async {
        let (mut out, mut capped) = (Vec::new(), stdout.take(MAX_TEXT as u64 + 1));
        capped.read_to_end(&mut out).await?;
        if out.len() > MAX_TEXT {
            bail!(
                "the document holds more than {} MiB of text",
                MAX_TEXT >> 20
            );
        }
        Ok((child.wait().await?, out))
    };
    tokio::time::timeout(HELPER_LIMIT, work)
        .await
        .with_context(|| format!("{program} took longer than {} s", HELPER_LIMIT.as_secs()))?
}

/// Text from bytes: UTF-8, with or without a byte-order mark; UTF-16 with
/// one; else, when no byte is NUL and few are control codes, Windows-1252,
/// which maps every byte and is what most legacy text on a Mac or PC is.
fn decode(bytes: &[u8]) -> Option<(String, TextFrom)> {
    // A UTF-8 mark over bytes that are not UTF-8 is read on without it.
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    for (bom, big) in [(b"\xFF\xFE", false), (b"\xFE\xFF", true)] {
        if let Some(rest) = bytes.strip_prefix(bom) {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| {
                    if big {
                        u16::from_be_bytes([p[0], p[1]])
                    } else {
                        u16::from_le_bytes([p[0], p[1]])
                    }
                })
                .collect();
            // Text has an even number of bytes and no NUL; a binary file
            // can begin with what looks like a byte-order mark.
            let text = String::from_utf16(&units)
                .ok()
                .filter(|t| rest.len() % 2 == 0 && !t.contains('\0'));
            if let Some(text) = text {
                return Some((text, TextFrom::Utf16));
            }
        }
    }
    if bytes.contains(&0) {
        return None;
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some((text.to_string(), TextFrom::Utf8));
    }
    let controls = bytes
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B))
        .count();
    (controls * 100 <= bytes.len()).then(|| {
        (
            bytes.iter().map(|&b| windows_1252(b)).collect(),
            TextFrom::Windows1252Guessed,
        )
    })
}

/// One Windows-1252 byte as its character. 0x80 to 0x9F hold punctuation and
/// a few letters; the five bytes the code page leaves undefined map to the
/// C1 controls of the same value, as the WHATWG Encoding Standard maps them.
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}

/// A media type for a file returned inline, by its name's extension.
pub fn mime_type(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase());
    match ext.as_deref() {
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        Some("md") => "text/markdown",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("html" | "htm") => "text/html",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// `path` for a URI: every byte but ASCII letters, digits, `-._~` and '/'
/// percent-encoded (RFC 3986).
pub fn uri_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(char::from(b));
        } else {
            write!(out, "%{b:02X}").expect("writing to a String cannot fail");
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn text(c: Content) -> Document {
        match c {
            Content::Text(d) => d,
            Content::Image { mime } => panic!("an image: {mime}"),
            Content::Other(why) => panic!("not text: {why}"),
        }
    }

    #[tokio::test]
    async fn text_is_read_in_its_encoding() {
        let utf8 = text(content("Café".as_bytes(), "a.txt").await.unwrap());
        assert_eq!((utf8.text.as_str(), utf8.from), ("Café", TextFrom::Utf8));
        // A Latin-1 file is not lost: é is 0xE9, and 0x93/0x94 are curly quotes.
        let latin = text(content(b"Caf\xE9 \x93hi\x94", "a.txt").await.unwrap());
        assert_eq!(latin.text, "Café \u{201C}hi\u{201D}");
        assert_eq!(latin.from, TextFrom::Windows1252Guessed);
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("hé".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(text(content(&utf16, "a.txt").await.unwrap()).text, "hé");
        // NUL bytes, or many control codes, mean binary.
        assert!(matches!(
            content(b"a\0b", "a.bin").await.unwrap(),
            Content::Other(_)
        ));
        assert!(matches!(
            content(&[1, 2, 3, 0xE9], "a.bin").await.unwrap(),
            Content::Other(_)
        ));
        // A UTF-8 byte-order mark over a Latin-1 byte still reads.
        let marked = text(content(b"\xEF\xBB\xBFCaf\xE9", "a.csv").await.unwrap());
        assert_eq!(marked.text, "Café");
        // A binary file that starts like a UTF-16 byte-order mark.
        assert!(matches!(
            content(&[0xFF, 0xFE, 0x00], "a.bin").await.unwrap(),
            Content::Other(_)
        ));
        assert!(matches!(
            content(b"\x89PNG\r\n\x1a\nrest", "x").await.unwrap(),
            Content::Image { mime: "image/png" }
        ));
    }

    /// RFC R22: in aliases mode a scan gives a reason, and no page image is
    /// made, since none would be returned.
    #[tokio::test]
    async fn a_scan_in_aliases_mode_gives_a_reason_and_no_images() {
        let mut scan = Document::new(&[String::new()], TextFrom::Pdf);
        scan.pdf = Some(Pdf {
            bytes: b"%PDF-1.4 not rendered".to_vec(),
            pages: 1,
        });
        let (v, images) =
            crate::content::restricted(crate::content::no_mentions(), scan.read(None, None, None))
                .await
                .unwrap();
        assert!(images.is_empty());
        assert_eq!(
            (v["content"].clone(), v["textLayer"].clone()),
            (Value::Null, json!(false))
        );
        assert!(
            v["reason"].as_str().unwrap().contains("no text layer"),
            "{v}"
        );
    }

    /// RFC R13: in aliases mode a page asked to start inside a mention
    /// starts where the mention does, so it never shows the mention's tail
    /// alone; off mode starts where asked.
    #[tokio::test]
    async fn aliases_mode_starts_a_page_at_a_mention_s_start() {
        let d = Document::new(&["to ann@x.io now".to_string()], TextFrom::Utf8);
        // A stand-in detector: the mention is bytes 3 to 11.
        let cut: crate::content::Cut =
            std::sync::Arc::new(|_, at| (3 < at && at < 11).then_some((3, 11)));
        let page = crate::content::restricted(cut, async { d.page(Some(6), Some(20)) })
            .await
            .unwrap();
        assert_eq!(
            (page["offset"].clone(), page["content"].clone()),
            (json!(3), json!("ann@x.io now"))
        );
        assert_eq!(d.page(Some(6), Some(20)).unwrap()["content"], "@x.io now");
    }

    #[test]
    fn pages_of_text_follow_on_to_the_end() {
        let d = Document::new(&["héllo wörld".to_string()], TextFrom::Utf8);
        let first = d.page(None, Some(5)).unwrap();
        assert_eq!(first["content"], "héllo");
        assert_eq!(
            (first["nextOffset"].clone(), first["totalChars"].clone()),
            (json!(5), json!(11))
        );
        assert!(first["note"].as_str().unwrap().contains("offset 5"));
        let last = d.page(Some(5), Some(100)).unwrap();
        assert_eq!(last["content"], " wörld");
        assert_eq!(
            (last["nextOffset"].clone(), last["truncated"].clone()),
            (Value::Null, json!(false))
        );
        assert_eq!(d.page(Some(11), None).unwrap()["content"], "");
        // Pages with no text say so.
        let blank = Document::new(&[String::new(), " ".to_string()], TextFrom::Utf8);
        assert_eq!(
            blank.page(None, None).unwrap()["note"],
            "the file holds no text"
        );
        assert!(d.page(Some(12), None).is_err());
        // Hidden characters go before offsets are counted, so pages line up.
        let hidden = Document::new(&["a\u{200B}b".to_string(), "c".to_string()], TextFrom::Pdf);
        let all = hidden.page(None, None).unwrap();
        assert_eq!(all["content"], "ab\u{C}c");
        assert_eq!(
            (
                all["pageStarts"].clone(),
                all["hiddenCharactersRemoved"].clone()
            ),
            (json!([0, 3]), json!(1))
        );
    }

    /// A one-page PDF made by macOS's own text-to-PDF filter.
    #[cfg(target_os = "macos")]
    pub(crate) fn sample_pdf() -> Vec<u8> {
        pdf_of("Hello from page one.\nCafé — accents.\n")
    }

    /// A PDF of `text`, a new page at each form feed.
    #[cfg(target_os = "macos")]
    fn pdf_of(text: &str) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("t.txt");
        std::fs::write(&src, text).unwrap();
        let out = std::process::Command::new("/usr/sbin/cupsfilter")
            .args(["-i", "text/plain"])
            .arg(&src)
            .stderr(Stdio::null())
            .output()
            .unwrap();
        assert!(out.stdout.starts_with(b"%PDF-"), "cupsfilter made no PDF");
        out.stdout
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn pdf_pages_come_as_images_and_a_scan_s_without_asking() {
        let six = text(
            content(&pdf_of("one\u{C}two\u{C}3\u{C}4\u{C}5\u{C}six"), "a.pdf")
                .await
                .unwrap(),
        );
        let (v, images) = six.read(None, None, Some(1)).await.unwrap();
        assert_eq!(
            (
                v["pdfPages"].clone(),
                v["pagesShown"].clone(),
                v["nextPage"].clone()
            ),
            (json!(6), json!([1, 4]), json!(5)),
            "{v}"
        );
        let labels: Vec<&str> = images
            .iter()
            .map(|a| match a {
                Attached::Image {
                    mime: "image/jpeg",
                    bytes,
                    label: Some(label),
                } => {
                    assert!(bytes.starts_with(b"\xFF\xD8\xFF"), "{label} is not a JPEG");
                    label.as_str()
                }
                other => panic!("not a labelled JPEG: {other:?}"),
            })
            .collect();
        assert_eq!(labels, ["Page 1:", "Page 2:", "Page 3:", "Page 4:"]);
        let (rest, images) = six.read(None, None, Some(5)).await.unwrap();
        assert_eq!(
            (
                rest["pagesShown"].clone(),
                rest["nextPage"].clone(),
                images.len()
            ),
            (json!([5, 6]), Value::Null, 2)
        );
        assert!(six.read(None, None, Some(7)).await.is_err());
        assert!(six.read(None, None, Some(0)).await.is_err());
        // Without page, a PDF with text comes as text.
        let (page, images) = six.read(None, None, None).await.unwrap();
        assert!(
            page["content"].as_str().unwrap().starts_with("one"),
            "{page}"
        );
        assert!(images.is_empty());
        // page is for PDFs.
        let plain = Document::new(&["x".to_string()], TextFrom::Utf8);
        assert!(plain.read(None, None, Some(1)).await.is_err());

        // A scan: a page rendered to JPEG, made back into a PDF by sips,
        // has no text layer, and its pages come as images unasked.
        let Attached::Image { bytes: jpeg, .. } =
            &six.read(None, None, Some(6)).await.unwrap().1[0]
        else {
            panic!("no image");
        };
        let dir = tempfile::tempdir().unwrap();
        let (src, scan) = (dir.path().join("p.jpg"), dir.path().join("scan.pdf"));
        std::fs::write(&src, jpeg).unwrap();
        let made = std::process::Command::new("/usr/bin/sips")
            .args(["-s", "format", "pdf"])
            .arg(&src)
            .arg("--out")
            .arg(&scan)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
        let scanned = text(
            content(&std::fs::read(&scan).unwrap(), "scan.pdf")
                .await
                .unwrap(),
        );
        let (v, images) = scanned.read(None, None, None).await.unwrap();
        assert_eq!(
            (
                v["textLayer"].clone(),
                v["pagesShown"].clone(),
                images.len()
            ),
            (json!(false), json!([1, 1]), 1),
            "{v}"
        );
    }

    /// A fixture of `tests/fixtures`, which `tests/convert.rs` describes.
    pub(crate) fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    /// The two-page PDF fixture: "Hello from page one.\nCafé — accents."
    /// and "Page two here.".
    #[cfg(target_os = "linux")]
    pub(crate) fn sample_pdf() -> Vec<u8> {
        fixture("two-pages.pdf")
    }

    #[test]
    fn poppler_output_splits_into_pages() {
        assert_eq!(pages_of("one\n\n\u{C}two\n\n\u{C}"), ["one", "two"]);
        assert_eq!(pages_of("\u{C}\u{C}"), ["", ""]);
        assert_eq!(pages_of(""), [""]);
    }

    /// A PDF with one Helvetica line per page and this Title, written by
    /// hand as `tests/fixtures/two-pages.pdf` was.
    #[cfg(target_os = "linux")]
    fn pdf_titled(title: &str, pages: &[&str]) -> Vec<u8> {
        use std::fmt::Write as _;
        let kids: Vec<String> = (0..pages.len())
            .map(|i| format!("{} 0 R", 5 + 2 * i))
            .collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                pages.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            format!("<< /Title ({}) >>", title.replace('\n', "\\n")),
        ];
        for (i, text) in pages.iter().enumerate() {
            let body = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                6 + 2 * i
            ));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{body}\nendstream",
                body.len()
            ));
        }
        let mut out = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            write!(out, "{} 0 obj\n{o}\nendobj\n", i + 1).unwrap();
        }
        let xref = out.len();
        write!(out, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
        for o in offsets {
            writeln!(out, "{o:010} 00000 n ").unwrap();
        }
        write!(
            out,
            "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .unwrap();
        out.into_bytes()
    }

    /// A scan as a scanner makes one: each JPEG a page of its own, drawn
    /// over the whole page, with no text layer.
    #[cfg(target_os = "linux")]
    pub(crate) fn scan_of(jpegs: &[Vec<u8>]) -> Vec<u8> {
        // The width and height from the JPEG's start-of-frame marker.
        let size = |j: &[u8]| {
            let mut i = 2;
            while !matches!(j[i + 1], 0xC0 | 0xC2) {
                i += 2 + usize::from(u16::from_be_bytes([j[i + 2], j[i + 3]]));
            }
            let at = |n: usize| u16::from_be_bytes([j[i + n], j[i + n + 1]]);
            (at(7), at(5))
        };
        let draw = "q 612 0 0 792 0 0 cm /Im Do Q";
        let kids: Vec<String> = (0..jpegs.len())
            .map(|i| format!("{} 0 R", 3 + 3 * i))
            .collect();
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                kids.join(" "),
                jpegs.len()
            )
            .into_bytes(),
        ];
        for (i, jpeg) in jpegs.iter().enumerate() {
            let (width, height) = size(jpeg);
            objects.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                     /Resources << /XObject << /Im {} 0 R >> >> /Contents {} 0 R >>",
                    4 + 3 * i,
                    5 + 3 * i
                )
                .into_bytes(),
            );
            let mut image = format!(
                "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
                 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                jpeg.len()
            )
            .into_bytes();
            image.extend(jpeg);
            image.extend(b"\nendstream");
            objects.push(image);
            objects.push(
                format!("<< /Length {} >>\nstream\n{draw}\nendstream", draw.len()).into_bytes(),
            );
        }
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend(o);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// M4.2 and R22: in aliases mode an image, and a scan's first
    /// `OCR_PAGES` pages, come back as the text OCR finds, with a note that
    /// says so; off mode keeps the image, and the scan's pages as images.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn images_and_scans_are_read_by_ocr_in_aliases_mode() {
        use crate::content::{no_mentions, restricted};
        let png = fixture("ocr.png");
        assert!(matches!(
            content(&png, "a.png").await.unwrap(),
            Content::Image { mime: "image/png" }
        ));
        let image = text(
            restricted(no_mentions(), content(&png, "a.png"))
                .await
                .unwrap(),
        );
        assert_eq!(
            (image.from, image.text.as_str()),
            (
                TextFrom::Tesseract,
                "Scanned invoice 4471\nPaid in full, thank you."
            )
        );
        let note = image.page(None, None).unwrap()["note"].clone();
        assert_eq!(note, "an image: its text is what OCR found in it");

        let words: Vec<String> = (1..=OCR_PAGES + 1)
            .map(|n| format!("Scanned page number {n}"))
            .collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let printed = Pdf {
            bytes: pdf_titled("", &words),
            pages: words.len(),
        };
        let mut jpegs = Vec::new();
        for page in 1..=words.len() {
            jpegs.push(page_jpeg(&printed, page).await.unwrap());
        }
        let scan = scan_of(&jpegs);
        let unread = text(content(&scan, "s.pdf").await.unwrap());
        assert_eq!((unread.from, unread.text.trim()), (TextFrom::Pdf, ""));
        let read = text(
            restricted(no_mentions(), content(&scan, "s.pdf"))
                .await
                .unwrap(),
        );
        assert_eq!(read.from, TextFrom::Tesseract);
        assert_eq!(read.page_starts.len(), OCR_PAGES);
        assert!(
            read.text.starts_with("Scanned page number 1\u{C}")
                && read
                    .text
                    .ends_with(&format!("Scanned page number {OCR_PAGES}")),
            "{:?}",
            read.text
        );
        let (v, images) = restricted(no_mentions(), read.read(None, None, None))
            .await
            .unwrap();
        assert!(images.is_empty());
        assert_eq!(
            v["note"],
            format!(
                "the PDF has no text layer (a scan): its text is what OCR found in its first {OCR_PAGES} of {} pages",
                OCR_PAGES + 1
            )
        );
    }

    /// The page count comes from the text, one form feed per page, not from
    /// metadata a document writes: a Title holding "Pages: 1" once hid the
    /// rest of a 6-page PDF. And a page past the end is an error, however
    /// large, not an overflow.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_page_count_is_the_pdf_s_not_its_title_s() {
        let six = ["one", "two", "three", "four", "five", "six"];
        let pdf = pdf_titled("Report\nPages: 1", &six);
        let doc = text(content(&pdf, "a.pdf").await.unwrap());
        let (v, images) = doc.read(None, None, Some(1)).await.unwrap();
        assert_eq!(
            (
                v["pdfPages"].clone(),
                v["pagesShown"].clone(),
                v["nextPage"].clone()
            ),
            (json!(6), json!([1, 4]), json!(5)),
            "{v}"
        );
        assert_eq!(images.len(), 4);
        assert!(doc.read(None, None, Some(7)).await.is_err());
        assert!(doc.read(None, None, Some(usize::MAX)).await.is_err());
    }

    /// The readers run here directly (`convert::read` under test); the
    /// sandboxed runs are in `tests/convert.rs`.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn poppler_and_pandoc_read_pdfs_and_documents() {
        let pdf = text(content(&sample_pdf(), "a.pdf").await.unwrap());
        assert_eq!(
            pdf.text,
            "Hello from page one.\nCafé — accents.\u{C}Page two here."
        );
        assert_eq!(
            (pdf.from, pdf.page_starts.clone()),
            (TextFrom::Pdf, vec![0, 37])
        );
        let (v, images) = pdf.read(None, None, Some(2)).await.unwrap();
        assert_eq!(
            (v["pdfPages"].clone(), v["pagesShown"].clone(), images.len()),
            (json!(2), json!([2, 2]), 1),
            "{v}"
        );
        assert!(pdf.read(None, None, Some(3)).await.is_err());
        for format in ["docx", "odt", "rtf"] {
            let name = format!("text.{format}");
            let doc = text(content(&fixture(&name), &name).await.unwrap());
            assert_eq!(
                doc.text, "Word text, Café.\n\nSecond paragraph.\n",
                "{format}"
            );
            assert_eq!(doc.from, TextFrom::Pandoc);
        }
        for (bytes, name, why) in [
            (
                &b"%PDF-1.4 damaged"[..],
                "a.pdf",
                "PDF reader could not read",
            ),
            (b"\xD0\xCF\x11\xE0 old Word", "t.doc", "old Word format"),
            (
                b"PK\x03\x04 damaged",
                "t.docx",
                "document reader could not read",
            ),
        ] {
            let read = content(bytes, name).await.unwrap();
            assert!(
                matches!(read, Content::Other(w) if w.contains(why)),
                "{name}"
            );
        }
        // Text and images need no reader.
        assert!(matches!(
            content(b"plain", "t.txt").await.unwrap(),
            Content::Text(_)
        ));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn pdf_and_word_text_come_from_the_system_s_readers() {
        let pdf = text(content(&sample_pdf(), "a.pdf").await.unwrap());
        assert!(pdf.text.contains("Café — accents."), "{:?}", pdf.text);
        assert_eq!(pdf.from, TextFrom::Pdf);
        // A file that only claims to be a PDF.
        let fake = content(b"%PDF-1.4 nothing else", "a.pdf").await.unwrap();
        assert!(matches!(fake, Content::Other(_)));
        // The fixtures pandoc made, and the old Word format, which only
        // textutil reads: a file it makes itself.
        for format in ["docx", "odt", "rtf"] {
            let name = format!("text.{format}");
            let doc = text(content(&fixture(&name), &name).await.unwrap());
            assert_eq!(
                doc.text, "Word text, Café.\nSecond paragraph.\n",
                "{format}"
            );
            assert_eq!(doc.from, TextFrom::Textutil);
        }
        let dir = tempfile::tempdir().unwrap();
        let (src, old) = (dir.path().join("t.txt"), dir.path().join("t.doc"));
        std::fs::write(&src, "Word text, Café.\n").unwrap();
        let made = std::process::Command::new("/usr/bin/textutil")
            .args(["-convert", "doc"])
            .arg(&src)
            .arg("-output")
            .arg(&old)
            .status()
            .unwrap();
        assert!(made.success());
        let word = text(
            content(&std::fs::read(&old).unwrap(), "t.doc")
                .await
                .unwrap(),
        );
        assert_eq!(word.text.trim(), "Word text, Café.");
        // textutil exits 0 and prints nothing for a file it cannot open
        // ("Error reading stdin" on stderr), so a damaged file reads as
        // empty text, where pandoc's failure is reported.
        let damaged = text(content(b"PK\x03\x04 damaged", "t.docx").await.unwrap());
        assert_eq!(damaged.text, "");
    }
}
