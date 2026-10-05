//! Detectors find mentions in text (RFC section 6, Pipeline step 3): regex
//! with validators, and the dictionary of names the result's headers and
//! the mailbox hold. `GLiNER` joins them in Phase 5.

pub mod dict;
pub mod pattern;

use super::ident::EntityType;

/// What a mention is: an entity, or a link, which is numbered rather than
/// aliased (RFC Q19).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Entity(EntityType),
    Link,
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
}

/// Keep the longest of overlapping mentions; on a tie, the one from the
/// earlier detector. The result is sorted by start.
pub fn settle(mut found: Vec<Mention>) -> Vec<Mention> {
    found.sort_by(|a, b| {
        (b.end - b.start)
            .cmp(&(a.end - a.start))
            .then(a.source.cmp(&b.source))
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
