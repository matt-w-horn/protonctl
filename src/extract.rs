//! Text out of a file's bytes, for the tools that return content inline, and
//! pages of that text. Text is read as UTF-8 or, with a byte-order mark,
//! UTF-16, and otherwise as Windows-1252, flagged as a guess. On macOS, PDF
//! text comes from PDFKit through `/usr/bin/osascript`, and Word, RTF and
//! OpenDocument text from `/usr/bin/textutil`, both part of macOS. On Linux
//! they come from poppler and pandoc, each run by `protonctl convert` in a
//! sandbox (R21, Q34). Every reader takes the document on stdin and runs as
//! its own process, so a hostile document can crash only it. Images are
//! named for the host to show, and a PDF's pages are rendered to images by
//! the same readers: a scan has no text layer, and Claude reads a page image
//! as it reads any other.

use std::fmt::Write as _;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use base64::Engine as _;

use crate::content::{Attached, clean};
use crate::convert::{Format, Job, Outcome};

/// Largest file read for its text or shown as an image.
pub const MAX_SOURCE: u64 = 64 << 20;
/// Largest image returned inline, the most Claude takes as one image.
pub const MAX_IMAGE: usize = 5 << 20;
/// Most text a document may yield; a zip bomb in a .docx stops here.
const MAX_TEXT: usize = 32 << 20;
/// How long PDFKit or textutil may take over one document.
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
    /// A PDF's bytes, kept to render its pages as images.
    pdf: Option<Vec<u8>>,
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
                json!({ "content": null, "textLayer": false,
                    "reason": "the PDF has no text layer (a scan); aliases mode returns no page images until Phase 4 reads their text" }),
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
        let end = offset.saturating_add(max).min(self.chars);
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
        Ok(v)
    }
}

/// What `bytes`, from a file called `name`, hold. Images and PDFs are known
/// by their first bytes, Word, RTF and OpenDocument files by the name's
/// extension, and text by its encoding.
pub async fn content(bytes: &[u8], name: &str) -> Result<Content> {
    if let Some(mime) = image_type(bytes) {
        return Ok(Content::Image { mime });
    }
    if bytes.starts_with(b"%PDF-") {
        return if cfg!(target_os = "macos") {
            pdf(bytes).await
        } else {
            poppler_pdf(bytes).await
        };
    }
    let ext = name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase());
    if let Some(format) = ext
        .as_deref()
        .filter(|x| ["docx", "doc", "rtf", "odt"].contains(x))
    {
        if !cfg!(target_os = "macos") {
            return pandoc_document(bytes, format).await;
        }
        let out = helper(
            "/usr/bin/textutil",
            &[
                "-stdin",
                "-stdout",
                "-convert",
                "txt",
                "-encoding",
                "UTF-8",
                "-format",
                format,
            ],
            bytes,
        )
        .await?;
        let text = String::from_utf8(out).context("textutil did not return UTF-8")?;
        return Ok(Content::Text(Document::new(&[text], TextFrom::Textutil)));
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

/// Every page's text, read by PDFKit from stdin and printed as JSON.
const PDF_SCRIPT: &str = r#"ObjC.import("PDFKit");
function run() {
  const data = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
  const doc = $.PDFDocument.alloc.initWithData(data);
  if (doc.isNil()) return "";
  const pages = [];
  for (let i = 0; i < doc.pageCount; i++) pages.push(ObjC.unwrap(doc.pageAtIndex(i).string) || "");
  return JSON.stringify({ locked: doc.isLocked, pages: pages });
}"#;

async fn pdf(bytes: &[u8]) -> Result<Content> {
    let out = helper(
        "/usr/bin/osascript",
        &["-l", "JavaScript", "-e", PDF_SCRIPT],
        bytes,
    )
    .await?;
    let read: Value = match serde_json::from_slice(&out) {
        Ok(v) => v,
        Err(_) => return Ok(Content::Other("PDFKit could not open the PDF")),
    };
    if read["locked"] == true {
        return Ok(Content::Other("the PDF is encrypted"));
    }
    let pages: Vec<String> = read["pages"]
        .as_array()
        .context("PDFKit returned no pages")?
        .iter()
        .map(|p| p.as_str().unwrap_or_default().to_string())
        .collect();
    let mut doc = Document::new(&pages, TextFrom::Pdf);
    doc.pdf = Some(bytes.to_vec());
    Ok(Content::Text(doc))
}

/// Why a reader that is not installed reads nothing.
fn missing(package: &str) -> &'static str {
    if package == "pandoc" {
        "reading Word, RTF and OpenDocument files here needs pandoc; install it (RFC-0001 Q34)"
    } else {
        "reading PDFs here needs poppler-utils; install it (RFC-0001 Q34)"
    }
}

/// A PDF's text from `pdftotext`, which ends every page with a form feed.
async fn poppler_pdf(bytes: &[u8]) -> Result<Content> {
    let out = match crate::convert::read(Job::PdfText, bytes).await? {
        Outcome::Read(out) => out,
        Outcome::Unreadable => {
            return Ok(Content::Other(
                "poppler could not read the PDF; it may be damaged or need a password",
            ));
        }
        Outcome::Missing(package) => return Ok(Content::Other(missing(package))),
    };
    let text = String::from_utf8(out).context("pdftotext did not return UTF-8")?;
    let mut doc = Document::new(&pages_of(&text), TextFrom::Pdf);
    doc.pdf = Some(bytes.to_vec());
    Ok(Content::Text(doc))
}

/// The pages of `pdftotext`'s output, without the form feed after the last
/// one or the blank lines it puts at the end of each.
fn pages_of(text: &str) -> Vec<String> {
    let text = text.strip_suffix('\u{C}').unwrap_or(text);
    text.split('\u{C}')
        .map(|page| page.trim_end_matches('\n').to_string())
        .collect()
}

/// A Word, OpenDocument or RTF document's text from pandoc, which cannot
/// read the old binary Word format.
async fn pandoc_document(bytes: &[u8], extension: &str) -> Result<Content> {
    let format = match extension {
        "docx" => Format::Docx,
        "odt" => Format::Odt,
        "rtf" => Format::Rtf,
        _ => {
            return Ok(Content::Other(
                "pandoc cannot read the old Word format (.doc); a copy saved as .docx can be read",
            ));
        }
    };
    let out = match crate::convert::read(Job::Document { format }, bytes).await? {
        Outcome::Read(out) => out,
        Outcome::Unreadable => return Ok(Content::Other("pandoc could not read the document")),
        Outcome::Missing(package) => return Ok(Content::Other(missing(package))),
    };
    let text = String::from_utf8(out).context("pandoc did not return UTF-8")?;
    Ok(Content::Text(Document::new(&[text], TextFrom::Pandoc)))
}

/// Up to `argv[1]` pages from page `argv[0]` (counted from 0), each
/// rendered with its long edge `argv[2]` pixels and saved as JPEG, printed
/// as JSON with the page count.
const RENDER_SCRIPT: &str = r#"ObjC.import("PDFKit");
ObjC.import("AppKit");
function run(argv) {
  const [first, count, edge] = argv.map(Number);
  const doc = $.PDFDocument.alloc.initWithData($.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile);
  if (doc.isNil() || doc.isLocked) return "";
  const images = [];
  for (let i = first; i < Math.min(doc.pageCount, first + count); i++) {
    const page = doc.pageAtIndex(i);
    const box = page.boundsForBox($.kPDFDisplayBoxCropBox).size;
    const scale = edge / Math.max(box.width, box.height);
    const size = $.NSMakeSize(Math.round(box.width * scale), Math.round(box.height * scale));
    const image = page.thumbnailOfSizeForBox(size, $.kPDFDisplayBoxCropBox);
    const bitmap = $.NSBitmapImageRep.imageRepWithData(image.TIFFRepresentation);
    const jpeg = bitmap.representationUsingTypeProperties($.NSBitmapImageFileTypeJPEG, $({ NSImageCompressionFactor: 0.85 }));
    images.push(jpeg.base64EncodedStringWithOptions(0).js);
  }
  return JSON.stringify({ pages: Number(doc.pageCount), images: images });
}"#;

/// The PDF's page count and up to `PAGE_IMAGES` pages from `first` as
/// JPEGs, rendered by PDFKit.
async fn pdfkit_pages(bytes: &[u8], first: usize) -> Result<(usize, Vec<Vec<u8>>)> {
    let args = [
        first.saturating_sub(1).to_string(),
        PAGE_IMAGES.to_string(),
        PAGE_EDGE.to_string(),
    ];
    let mut command = vec!["-l", "JavaScript", "-e", RENDER_SCRIPT];
    command.extend(args.iter().map(String::as_str));
    let out = helper("/usr/bin/osascript", &command, bytes).await?;
    let read: Value = serde_json::from_slice(&out).context("PDFKit could not render the PDF")?;
    let total = read["pages"]
        .as_u64()
        .map_or(0, |n| usize::try_from(n).unwrap_or(0));
    let mut jpegs = Vec::new();
    for data in read["images"].as_array().into_iter().flatten() {
        jpegs.push(
            base64::engine::general_purpose::STANDARD
                .decode(data.as_str().unwrap_or_default())
                .context("PDFKit returned an image that is not base64")?,
        );
    }
    Ok((total, jpegs))
}

/// The PDF's page count from `pdfinfo`, and up to `PAGE_IMAGES` pages from
/// `first` as JPEGs from `pdftoppm`, one sandboxed run each.
async fn poppler_pages(bytes: &[u8], first: usize) -> Result<(usize, Vec<Vec<u8>>)> {
    let read = async |job| match crate::convert::read(job, bytes).await? {
        Outcome::Read(out) => Ok(out),
        Outcome::Unreadable => bail!("poppler could not read the PDF"),
        Outcome::Missing(package) => bail!("{}", missing(package)),
    };
    let info = read(Job::PdfInfo).await?;
    let total =
        page_count(&String::from_utf8_lossy(&info)).context("pdfinfo gave no page count")?;
    let mut jpegs = Vec::new();
    for page in first..=total.min(first + PAGE_IMAGES - 1) {
        let jpeg = read(Job::PdfPage {
            page,
            edge: PAGE_EDGE,
        })
        .await?;
        if !jpeg.starts_with(b"\xFF\xD8\xFF") {
            bail!("pdftoppm did not return a JPEG for page {page}");
        }
        jpegs.push(jpeg);
    }
    Ok((total, jpegs))
}

/// The "Pages:" line of `pdfinfo`'s output.
fn page_count(info: &str) -> Option<usize> {
    info.lines()
        .find_map(|l| l.strip_prefix("Pages:"))
        .and_then(|n| n.trim().parse().ok())
}

/// Pages of the PDF `bytes` from `first` (counted from 1) as JPEG images,
/// `PAGE_IMAGES` at most, each after a "Page N:" label, and the JSON that
/// says which pages they are and where the next call starts.
async fn page_images(bytes: &[u8], first: usize) -> Result<(Value, Vec<Attached>)> {
    if first == 0 {
        bail!("page counts from 1");
    }
    let (total, jpegs) = if cfg!(target_os = "macos") {
        pdfkit_pages(bytes, first).await?
    } else {
        poppler_pages(bytes, first).await?
    };
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

/// Run one of macOS's own helpers on `input` and return what it prints;
/// any exit but success is an error.
async fn helper(program: &str, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    let (status, out) = pipe(program, args, input).await?;
    if !status.success() {
        bail!("{program} could not read the document ({status})");
    }
    Ok(out)
}

/// Run `program` on `input` with an empty environment, and return how it
/// exited and what it printed, at most `MAX_TEXT` bytes, within
/// `HELPER_LIMIT`. `kill_on_drop` ends it when the call is cut off.
pub async fn pipe<S: AsRef<std::ffi::OsStr>>(
    program: &str,
    args: &[S],
    input: &[u8],
) -> Result<(std::process::ExitStatus, Vec<u8>)> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("cannot run {program}"))?;
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
        scan.pdf = Some(b"%PDF-1.4 not rendered".to_vec());
        let (v, images) = crate::content::restricted(scan.read(None, None, None))
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
    #[cfg(target_os = "linux")]
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
        assert_eq!(
            page_count("Producer: x\nPages:           12\nEncrypted: no\n"),
            Some(12)
        );
        assert_eq!(page_count("Pages: many"), None);
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
            (&b"%PDF-1.4 damaged"[..], "a.pdf", "poppler could not read"),
            (b"\xD0\xCF\x11\xE0 old Word", "t.doc", "old Word format"),
            (b"PK\x03\x04 damaged", "t.docx", "pandoc could not read"),
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
        let dir = tempfile::tempdir().unwrap();
        let (src, docx) = (dir.path().join("t.txt"), dir.path().join("t.docx"));
        std::fs::write(&src, "Word text, Café.\n").unwrap();
        let made = std::process::Command::new("/usr/bin/textutil")
            .args(["-convert", "docx"])
            .arg(&src)
            .arg("-output")
            .arg(&docx)
            .status()
            .unwrap();
        assert!(made.success());
        let word = text(
            content(&std::fs::read(&docx).unwrap(), "t.docx")
                .await
                .unwrap(),
        );
        assert_eq!(word.text.trim(), "Word text, Café.");
    }
}
