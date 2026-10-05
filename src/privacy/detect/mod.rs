//! Detectors find mentions in text (RFC section 6, Pipeline step 3): regex
//! with validators, and the dictionary of names the result's headers and
//! the mailbox hold. `GLiNER` joins them in Phase 5.

pub mod dict;
pub mod pattern;

use super::ident::{Algorithm, EntityType};

/// What a mention is: an entity, or a link, which is numbered rather than
/// aliased (RFC Q19), or an identifier written in text, which becomes
/// what the same value in a field becomes: a digest its keyed form (R17),
/// a Proton message ID a handle (R16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Entity(EntityType),
    Link,
    Digest(Algorithm),
    MessageId,
}

/// A detector, as `detectors` names it. The order of the variants is the
/// tie order: of two overlapping mentions of equal length, the one found by
/// the earlier detector wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Detector {
    Dictionary,
    Regex,
}

/// What a mention stands for (RFC section 6, Canonical values): the value
/// as written, or another form of known names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Form {
    /// A name as written, or any detector's value.
    #[default]
    Whole,
    /// A short form of these full names: a given name, a surname, the name
    /// without its middle names, or initials.
    Short(Vec<String>),
}

/// A span of one text, in byte offsets, and the value it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    /// The text as found, before canonicalizing; a phone number in E.164.
    pub value: String,
    /// Which detector found it.
    pub source: Detector,
    pub form: Form,
}

/// Keep the longest of overlapping mentions; on a tie, the one from the
/// earlier detector, then a name as written over another form of a name.
/// The result is sorted by start.
pub fn settle(mut found: Vec<Mention>) -> Vec<Mention> {
    found.sort_by(|a, b| {
        (b.end - b.start)
            .cmp(&(a.end - a.start))
            .then(a.source.cmp(&b.source))
            .then((a.form != Form::Whole).cmp(&(b.form != Form::Whole)))
            .then(a.start.cmp(&b.start))
    });
    let mut kept: Vec<Mention> = Vec::new();
    for m in found {
        if kept.iter().all(|k| m.end <= k.start || m.start >= k.end) {
            kept.push(m);
        }
    }
    kept.sort_by_key(|m| m.start);
    kept
}

/// How far either side of a cut `around` looks: more than any mention
/// but the longest links.
const REACH: usize = 4096;

/// The mention a cut at byte `at` of `text` would split in two, in byte
/// offsets. Text is cut into pages before the pipeline sees it, so a cut
/// inside a name or an address would leave each half to pass undetected
/// (RFC R13); aliases mode moves its cuts off these spans.
pub fn around(dict: &dict::Dictionary, text: &str, at: usize) -> Option<(usize, usize)> {
    let lo = text.floor_char_boundary(at.saturating_sub(REACH));
    let hi = text.ceil_char_boundary(at.saturating_add(REACH));
    let window = &text[lo..hi];
    let mut found = Vec::new();
    pattern::find(window, &mut found);
    dict.find(window, &mut found);
    settle(found)
        .into_iter()
        .map(|m| (m.start + lo, m.end + lo))
        .find(|&(start, end)| start < at && at < end)
}

/// Whether `c` belongs to a script written without spaces between words:
/// Chinese, Japanese kana, Thai, Lao, Khmer and Myanmar. A name in such
/// text has letters on both sides, so `bounded` cannot hold for it.
pub fn unspaced(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3134F
        | 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F
        | 0x0E00..=0x0EFF | 0x1780..=0x17FF | 0x1000..=0x109F)
}

/// Whether `text[start..end]` stands alone: no letter or digit just before
/// or just after it.
pub fn bounded(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(start: usize, end: usize, source: Detector) -> Mention {
        Mention {
            start,
            end,
            kind: Kind::Link,
            value: String::new(),
            source,
            form: Form::Whole,
        }
    }

    #[test]
    fn the_longest_mention_wins_and_ties_go_to_the_dictionary() {
        use Detector::{Dictionary, Regex};
        let kept = settle(vec![
            m(0, 5, Regex),
            m(2, 10, Regex),
            m(12, 15, Regex),
            m(12, 15, Dictionary),
        ]);
        assert_eq!(kept, vec![m(2, 10, Regex), m(12, 15, Dictionary)]);
        assert!(bounded("an Ann.", 3, 6));
        assert!(!bounded("announce", 0, 3));
    }
}
