//! Which parts of a parsed message are its body and which its attachments.
//!
//! mail-parser 1.0 follows RFC 8621 for these lists. protonctl keeps the rules
//! it had under 0.11, so for well-formed mail `get_attachment` indexes, and
//! which text is the body, stay as they were: an inline image stays in the
//! attachment list, and a text or HTML alternative stays a body whatever its
//! name or Content-Disposition says. mail-parser's own `attachments()`,
//! `text_body()` and `body_text()` answer differently. Broken structure, broken
//! encodings and 1.0's limits can still change the lists, since the parts come
//! from 1.0's parser.

use std::borrow::Cow;

use mail_parser::{Message, MessagePart, PartKind, html_to_text, text_to_html};

/// The root message's parts by use, each in document order. A nested message
/// is one attachment; its own parts are not listed.
#[derive(Default)]
pub(super) struct Parts<'m> {
    /// The body as text; HTML stands in for a missing text version, and inline
    /// media are kept in reading order.
    pub(super) text: Vec<MessagePart<'m>>,
    /// The body as HTML, likewise.
    pub(super) html: Vec<MessagePart<'m>>,
    /// What `get_attachment` indexes.
    pub(super) attachments: Vec<MessagePart<'m>>,
}

/// What surrounds a part being sorted.
#[derive(Clone, Copy)]
struct Scope {
    container: Container,
    in_alternative: bool,
    need_text: bool,
    need_html: bool,
    /// Parts of this container so far, the one being sorted included.
    parts: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Container {
    Message,
    Mixed,
    Alternative,
    Related,
    Digest,
}

impl<'m> Parts<'m> {
    pub(super) fn of(message: &'m Message) -> Self {
        let mut parts = Self::default();
        parts.sort(
            message.root_part(),
            &mut Scope {
                container: Container::Message,
                in_alternative: false,
                need_text: true,
                need_html: true,
                parts: 0,
            },
        );
        parts
    }

    /// Recursion stops at the parser's `max_depth`: past it a multipart is a
    /// leaf.
    fn sort(&mut self, part: MessagePart<'m>, scope: &mut Scope) {
        scope.parts += 1;
        if part.is_multipart() {
            self.sort_multipart(part, scope);
            return;
        }
        let content_type = part.content_type();
        let (plain, html, media) = match content_type.map(|c| (c.ctype(), c.subtype())) {
            // A digest's parts are messages unless they say otherwise.
            None => (scope.container != Container::Digest, false, false),
            Some(("text", Some("plain"))) => (true, false, false),
            Some(("text", Some("html"))) => (false, true, false),
            Some(("image" | "audio" | "video", _)) => (false, false, true),
            Some(_) => (false, false, false),
        };
        let named = content_type.is_some_and(|c| c.has_attribute("name"));
        let inline = (plain || html || media)
            && !part
                .content_disposition()
                .is_some_and(|d| d.is_attachment())
            && (scope.parts == 1 || scope.container != Container::Related && (media || !named));
        let (to_text, to_html) = if scope.container == Container::Alternative {
            (plain, html)
        } else if inline {
            if scope.in_alternative && html {
                scope.need_text = false;
            }
            if scope.in_alternative && plain {
                scope.need_html = false;
            }
            (scope.need_text, scope.need_html)
        } else {
            (false, false)
        };
        if to_text {
            self.text.push(part);
        }
        if to_html {
            self.html.push(part);
        }
        let is_body = match part.kind() {
            PartKind::Text => to_text,
            PartKind::Html => to_html,
            _ => false,
        };
        if !is_body {
            self.attachments.push(part);
        }
    }

    fn sort_multipart(&mut self, part: MessagePart<'m>, scope: &Scope) {
        let container = match part.content_type().and_then(|c| c.subtype()) {
            Some("alternative") => Container::Alternative,
            Some("related") => Container::Related,
            Some("digest") => Container::Digest,
            _ => Container::Mixed,
        };
        let mut inner = Scope {
            container,
            in_alternative: scope.in_alternative || container == Container::Alternative,
            parts: 0,
            ..*scope
        };
        let (text, html) = (self.text.len(), self.html.len());
        for child in part.children() {
            self.sort(child, &mut inner);
        }
        // An alternative with one version: it stands in for the other.
        if container == Container::Alternative && inner.need_text && inner.need_html {
            if self.text.len() == text {
                self.text.extend_from_slice(&self.html[html..]);
            } else if self.html.len() == html {
                self.html.extend_from_slice(&self.text[text..]);
            }
        }
    }
}

/// A body part as text: HTML converted, and nothing for a part that is not
/// text, such as an inline image placed first.
pub(super) fn as_text(part: MessagePart<'_>) -> Option<String> {
    match part.kind() {
        PartKind::Text => part.text().map(Cow::into_owned),
        PartKind::Html => part.text().map(|html| html_to_text(&html)),
        _ => None,
    }
}

/// A body part as HTML: text converted, and nothing for a part that is not
/// text.
pub(super) fn as_html(part: MessagePart<'_>) -> Option<String> {
    match part.kind() {
        PartKind::Html => part.text().map(Cow::into_owned),
        PartKind::Text => part.text().map(|text| text_to_html(&text)),
        _ => None,
    }
}

/// An attachment's bytes, which its listed `size` measures: text as UTF-8,
/// converted from its charset, since `decoded()` undoes only the transfer
/// encoding; anything else as sent, and a nested message whole.
pub(super) fn contents(part: MessagePart<'_>) -> Cow<'_, [u8]> {
    match part.text() {
        Some(Cow::Borrowed(text)) => Cow::Borrowed(text.as_bytes()),
        Some(Cow::Owned(text)) => Cow::Owned(text.into_bytes()),
        None => part.decoded(),
    }
}
