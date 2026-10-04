//! A message's text as the mail tools show it: the body without quoted
//! replies, a search row's snippet, and the Authentication-Results summary.

use mail_parser::{Message, MessageParser};

use crate::content::{clean, truncate};

/// The first copy of a header: the topmost, added by the last server to
/// handle the message. mail-parser's `header_raw` returns the last copy,
/// which for Authentication-Results can be one the sender wrote.
pub(super) fn top_header<'a>(m: &'a Message, name: &str) -> Option<&'a str> {
    let h = m
        .headers()
        .iter()
        .find(|h| h.name().eq_ignore_ascii_case(name))?;
    let span = m
        .raw_message()
        .get(h.offset_start() as usize..h.offset_end() as usize)?;
    std::str::from_utf8(span).ok()
}

/// The verdicts an Authentication-Results header records, compactly, as in
/// `dmarc=pass (header.from=x.com); dkim=pass (header.d=x.com); spf=pass;
/// by mx.example`. Comments, such as key sizes, are dropped.
pub(super) fn auth_summary(raw: &str) -> String {
    let (mut text, mut depth) = (String::new(), 0usize);
    for c in raw.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if depth == 0 => text.push(if c.is_whitespace() { ' ' } else { c }),
            _ => {}
        }
    }
    let mut clauses = text.split(';').map(str::trim);
    let by = clauses.next().unwrap_or_default().to_string();
    let mut out: Vec<String> = clauses
        .filter_map(|clause| {
            let mut words = clause.split_whitespace();
            let (method, result) = words.next()?.split_once('=')?;
            let method = method.to_ascii_lowercase();
            let property = match method.as_str() {
                "dmarc" => "header.from",
                "dkim" => "header.d",
                "spf" => "smtp.mailfrom",
                _ => return None,
            };
            let result = result.to_ascii_lowercase();
            Some(
                match words.find_map(|w| {
                    w.split_once('=')
                        .filter(|(k, _)| k.eq_ignore_ascii_case(property))
                }) {
                    Some((_, value)) => format!("{method}={result} ({property}={value})"),
                    None => format!("{method}={result}"),
                },
            )
        })
        .collect();
    if out.is_empty() {
        out.push("no spf, dkim or dmarc result".into());
    }
    if !by.is_empty() {
        out.push(format!("by {by}"));
    }
    out.join("; ")
}

/// Endings of the line that introduces a quoted reply, in the languages
/// handled so far.
const ATTRIBUTIONS: [&str; 5] = ["wrote:", "a écrit :", "a écrit:", "schrieb:", "escribió:"];

/// Whether a line starts an Outlook-style quoted original: "-----Original
/// Message-----", or a From line with a Sent line among the next three.
fn starts_original(lines: &[&str], i: usize) -> bool {
    let line = lines[i].trim();
    let dashes = line.trim_matches('-');
    if line.starts_with("--") && dashes.trim().eq_ignore_ascii_case("original message") {
        return true;
    }
    line.starts_with("From:")
        && lines[i + 1..]
            .iter()
            .take(3)
            .any(|l| l.trim_start().starts_with("Sent:"))
}

/// `text` without quoted replies, and the number of non-blank lines that
/// went. Lines starting with `>` go, with the attribution line before them;
/// an Outlook-style original goes to the end, except in a forward, whose
/// original is the point. Unquoted lines always stay, so inline replies keep
/// their answers.
fn strip_quotes(text: &str, forward: bool) -> (String, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let end = (0..lines.len())
        .find(|&i| !forward && starts_original(&lines, i))
        .unwrap_or(lines.len());
    let quoted = |i: usize| lines[i].trim_start().starts_with('>');
    let mut keep = vec![true; end];
    for i in 0..end {
        if quoted(i) {
            keep[i] = false;
            continue;
        }
        let line = lines[i].trim_end();
        if !ATTRIBUTIONS.iter().any(|a| line.ends_with(a)) {
            continue;
        }
        // An attribution introduces a quote, or nothing but the cut-off original.
        let next = (i + 1..end).find(|&j| !lines[j].trim().is_empty());
        if next.is_none_or(quoted) {
            keep[i] = false;
            // Gmail wraps a long attribution: "On ... <" then "... wrote:".
            if i > 0 && lines[i - 1].trim_start().starts_with("On ") {
                keep[i - 1] = false;
            }
        }
    }
    let gone = (0..lines.len())
        .filter(|&i| i >= end || !keep[i])
        .filter(|&i| !lines[i].trim().is_empty())
        .count();
    let mut out = String::new();
    let mut blank = false;
    for (line, _) in lines[..end].iter().zip(&keep).filter(|(_, k)| **k) {
        let empty = line.trim().is_empty();
        if !(empty && blank) {
            out.push_str(line);
            out.push('\n');
        }
        blank = empty;
    }
    (out.trim_end().to_string(), gone)
}

/// Where the element starting at `start` in `lower` (lower-cased HTML) ends,
/// counting nested elements of the same name; the end of the text if it
/// never closes.
fn element_end(lower: &str, start: usize, name: &str) -> usize {
    let (open, close) = (format!("<{name}"), format!("</{name}"));
    let mut depth = 0usize;
    let mut i = start;
    while let Some(at) = lower[i..].find('<').map(|j| i + j) {
        let rest = &lower[at..];
        let boundary = |tag: &str| {
            rest.starts_with(tag)
                && rest[tag.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_whitespace() || c == '>' || c == '/')
        };
        if boundary(&open) {
            depth += 1;
        } else if boundary(&close) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return lower[at..].find('>').map_or(lower.len(), |j| at + j + 1);
            }
        }
        i = at + 1;
    }
    lower.len()
}

/// `html` without the elements that hold quoted replies: every
/// `<blockquote>`, the `gmail_quote` and `protonmail_quote` containers, and
/// Outlook's reply header (`divRplyFwdMsg`) with everything after it.
fn strip_html_quotes(html: &str) -> String {
    // ASCII lower-casing keeps every byte offset.
    let lower = html.to_ascii_lowercase();
    let end = lower
        .find("id=\"divrplyfwdmsg\"")
        .and_then(|i| lower[..i].rfind('<'))
        .unwrap_or(lower.len());
    let quote_at = |from: usize| -> Option<(usize, &'static str)> {
        let mut i = from;
        while let Some(at) = lower[i..end].find('<').map(|j| i + j) {
            let tag = &lower[at..lower[at..].find('>').map_or(end, |j| at + j)];
            if tag.starts_with("<blockquote")
                && tag[11..]
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_whitespace() || c == '>')
            {
                return Some((at, "blockquote"));
            }
            if tag.starts_with("<div")
                && (tag.contains("gmail_quote") || tag.contains("protonmail_quote"))
            {
                return Some((at, "div"));
            }
            i = at + 1;
        }
        None
    };
    let (mut out, mut i) = (String::new(), 0);
    while let Some((at, name)) = quote_at(i) {
        out.push_str(&html[i..at]);
        i = element_end(&lower, at, name).min(end);
    }
    out.push_str(&html[i..end]);
    out
}

/// A message's body as text (HTML converted) and, when `quotes_removed`,
/// without quoted replies, with the number of non-blank lines that went.
/// Quotes in HTML-only mail leave no `>` once converted, so they go from
/// the HTML first.
pub(super) fn readable_body(message: &Message, quotes_removed: bool) -> (String, usize) {
    let body = message.body_text(0).unwrap_or_default().into_owned();
    if !quotes_removed {
        return (body, 0);
    }
    let subject = message.subject().unwrap_or_default().to_ascii_lowercase();
    let forward = ["fw:", "fwd:", "tr:", "wg:"]
        .iter()
        .any(|p| subject.starts_with(p));
    let html_only = message
        .text_body
        .first()
        .and_then(|&id| message.parts.get(id as usize))
        .is_some_and(|p| matches!(p.body, mail_parser::PartType::Html(_)));
    let lines = |t: &str| t.lines().filter(|l| !l.trim().is_empty()).count();
    let before = lines(&body);
    let text = match message.body_html(0) {
        Some(html) if html_only => {
            mail_parser::decoders::html::html_to_text(&strip_html_quotes(&html))
        }
        _ => body,
    };
    let (stripped, _) = strip_quotes(&text, forward);
    let gone = before.saturating_sub(lines(&stripped));
    (stripped, gone)
}

/// The characters of a search row's snippet.
const SNIPPET_CHARS: usize = 200;

/// The start of a message's text, from its header block and the first part
/// of its body (`BODY.PEEK[TEXT]<0.N>`), joined into a message mail-parser
/// can decode: without quoted replies, whitespace collapsed, at most 200
/// characters.
pub(super) fn snippet(header_block: &[u8], partial: &[u8], removed: &mut usize) -> Option<String> {
    let mut raw = header_block.to_vec();
    raw.extend_from_slice(partial);
    let message = MessageParser::default().parse(&raw)?;
    let (text, _) = readable_body(&message, true);
    let mut text = clean(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        removed,
    );
    truncate(&mut text, SNIPPET_CHARS);
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_replies_leave_plain_text_bodies() {
        let reply = "Thanks, see below.\n\nOn Mon, 5 Oct 2026 at 10:00, Ann <ann@x.test> wrote:\n> Can you send it?\n> Ann\n";
        assert_eq!(strip_quotes(reply, false), ("Thanks, see below.".into(), 3));
        // Gmail wraps a long attribution over two lines.
        let wrapped =
            "Done.\n\nOn Mon, Oct 5, 2026 at 10:00 AM Ann Example <\nann@x.test> wrote:\n\n> Old\n";
        assert_eq!(strip_quotes(wrapped, false), ("Done.".into(), 3));
        // Inline answers stay; only the quoted lines go.
        let inline = "> Question one?\nAnswer one.\n> Question two?\nAnswer two.";
        assert_eq!(
            strip_quotes(inline, false),
            ("Answer one.\nAnswer two.".into(), 2)
        );
        // A line that merely ends in "wrote:" and quotes nothing stays.
        let prose = "She wrote:\nthe plan is fine.";
        assert_eq!(strip_quotes(prose, false).0, prose);
        // Outlook puts the original under a header block; a forward keeps it.
        let outlook = "Sounds good.\n\nFrom: Bob <b@x.test>\nSent: Monday, October 5, 2026 10:00 AM\nTo: Ann\nSubject: Re: Plan\n\nOld text";
        assert_eq!(strip_quotes(outlook, false), ("Sounds good.".into(), 5));
        assert_eq!(strip_quotes(outlook, true).0, outlook);
        let original = "Yes.\n-----Original Message-----\nFrom: Bob\nOld";
        assert_eq!(strip_quotes(original, false), ("Yes.".into(), 3));
    }

    #[test]
    fn quoted_replies_leave_html_bodies() {
        let gmail = "<div>New text</div><div class=\"gmail_quote\"><div class=\"gmail_attr\">On Mon, Ann wrote:</div>\
                     <blockquote class=\"gmail_quote\"><div>Old</div><blockquote>Older</blockquote></blockquote></div><p>After</p>";
        assert_eq!(strip_html_quotes(gmail), "<div>New text</div><p>After</p>");
        let proton = "<p>Hi</p><div class=\"protonmail_quote\">Ann wrote:<blockquote type=\"cite\">x</blockquote></div>";
        assert_eq!(strip_html_quotes(proton), "<p>Hi</p>");
        // Outlook's reply header and everything after it go.
        let outlook =
            "<p>Yes</p><hr><div id=\"divRplyFwdMsg\"><b>From:</b> Bob</div><div>Old</div>";
        assert_eq!(strip_html_quotes(outlook), "<p>Yes</p><hr>");
        // Other elements stay, and an unclosed quote runs to the end.
        assert_eq!(
            strip_html_quotes("<div class=\"x\">a</div><blockquote>b"),
            "<div class=\"x\">a</div>"
        );
        assert_eq!(
            strip_html_quotes("<blockquotes>kept</blockquotes>"),
            "<blockquotes>kept</blockquotes>"
        );
    }

    #[test]
    fn authentication_comes_from_the_receiving_server_s_header() {
        // The topmost header is the receiving server's; one lower down was
        // written by the sender.
        let block = "Authentication-Results: mx.proton.test; dkim=fail header.d=bank.test;\r\n \
                     dmarc=fail (p=reject) header.from=bank.test; spf=softfail smtp.mailfrom=x@evil.test\r\n\
                     Authentication-Results: evil.test; dkim=pass header.d=bank.test; dmarc=pass header.from=bank.test\r\n\r\n";
        let m = MessageParser::default()
            .parse_headers(block.as_bytes())
            .unwrap();
        let top = top_header(&m, "Authentication-Results").unwrap();
        assert_eq!(
            auth_summary(top),
            "dkim=fail (header.d=bank.test); dmarc=fail (header.from=bank.test); spf=softfail (smtp.mailfrom=x@evil.test); by mx.proton.test"
        );
        let two = "mx.test; dkim=pass (2048-bit key) header.d=a.test header.b=xyz; dkim=pass header.d=b.test; spf=none";
        assert_eq!(
            auth_summary(two),
            "dkim=pass (header.d=a.test); dkim=pass (header.d=b.test); spf=none; by mx.test"
        );
        assert_eq!(
            auth_summary("nonsense"),
            "no spf, dkim or dmarc result; by nonsense"
        );
    }

    #[test]
    fn snippets_are_the_start_of_the_text_without_quotes() {
        let mut n = 0;
        let mut snip = |head: &str, body: &[u8]| snippet(head.as_bytes(), body, &mut n);
        // The plain alternative, its quoted reply gone, whitespace collapsed.
        let alternative = "Content-Type: multipart/alternative; boundary=\"b\"\r\n\r\n";
        let body = b"--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nSee   you\r\nat noon.\r\n\r\n\
                     On Mon, Ann wrote:\r\n> earlier\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>See you</p>\r\n--b--\r\n";
        assert_eq!(snip(alternative, body).as_deref(), Some("See you at noon."));
        // HTML only: styles and the quoted part go; the fetch cut it mid-document.
        let html = format!(
            "<html><head><style>{}</style></head><body><p>New text</p><blockquote>old</blockquote><p>cut he",
            "p{color:red}".repeat(500)
        );
        let snipped = snip(
            "Content-Type: text/html; charset=utf-8\r\n\r\n",
            html.as_bytes(),
        );
        assert_eq!(snipped.as_deref(), Some("New text cut he"));
        let qp = "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n";
        assert_eq!(snip(qp, b"Caf=C3=A9 at 5").as_deref(), Some("Café at 5"));
        // Base64 cut mid-quad by the fetch limit still gives its start.
        let b64 = "Content-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n";
        let cut = snip(b64, b"SGVsbG8gd29ybGQsIHRoaXMgaXMgYSB0ZXN0");
        assert!(
            cut.as_deref().is_some_and(|s| s.starts_with("Hello world")),
            "{cut:?}"
        );
        let long = "x ".repeat(300);
        assert_eq!(
            snip("Content-Type: text/plain\r\n\r\n", long.as_bytes()).map(|s| s.chars().count()),
            Some(200)
        );
        assert_eq!(snip("Content-Type: text/plain\r\n\r\n", b"  \r\n "), None);
    }
}
