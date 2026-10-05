//! The name dictionary (RFC section 6, Pipeline step 2): display names and
//! addresses from the result's own headers, attendees and owners, and from
//! the process dictionary (Q22), found again wherever they appear in text.

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

use super::{Detector, Kind, Mention, bounded};
use crate::privacy::ident::{AliasClass, EntityType};
use crate::privacy::words;

/// Names to look for, each with its type.
#[derive(Debug, Default, Clone)]
pub struct Names(Vec<(String, EntityType)>);

impl Names {
    /// Add a name, unless it is too short or has no letter to be one: "Me"
    /// or "Al" would match ordinary words.
    pub fn add(&mut self, text: &str, t: EntityType) {
        let text = text.trim();
        if text.chars().count() >= 3 && text.chars().any(char::is_alphabetic) {
            self.0.push((text.to_string(), t));
        }
    }

    /// Display names from mail and calendars, as people (RFC Q22). An
    /// address given as its own name is left to the address detector, and a
    /// one-word name on the alias word list ("Support", "Security") is left
    /// out, or every use of the word would become an alias. The list is not
    /// every common word: "Notifications" or "Admin" still joins.
    pub fn people(names: impl IntoIterator<Item = String>) -> Self {
        let mut out = Self::default();
        for name in names {
            let name = name.trim();
            let common =
                !name.contains(char::is_whitespace) && words::contains(&name.to_lowercase());
            if !common && !name.contains('@') {
                out.add(name, EntityType::Person);
            }
        }
        out
    }

    /// An address, and its domain, so the domain is found on its own too
    /// ("header.d=" in `authentication`), whatever its top-level domain.
    pub fn add_address(&mut self, email: &str) {
        self.add(email, EntityType::Email);
        if let Some((_, domain)) = email.rsplit_once('@') {
            self.add(domain, EntityType::Domain);
        }
    }

    pub fn extend(&mut self, other: &Self) {
        self.0.extend(other.0.iter().cloned());
    }
}

pub struct Dictionary {
    automaton: Option<AhoCorasick>,
    names: Vec<(String, EntityType)>,
}

/// `text` case-folded (Unicode default case folding, as `canon` uses),
/// with a map from each byte of the copy to where its character starts in
/// `text`, plus one entry for the end.
fn folded(text: &str) -> (String, Vec<usize>) {
    use caseless::Caseless as _;
    let mut copy = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        for f in std::iter::once(c).default_case_fold() {
            copy.push(f);
            map.extend(std::iter::repeat_n(i, f.len_utf8()));
        }
    }
    map.push(text.len());
    (copy, map)
}

impl Dictionary {
    pub fn new(names: &Names) -> anyhow::Result<Self> {
        let mut names = names.0.clone();
        names.sort();
        names.dedup();
        let automaton = if names.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(names.iter().map(|(n, _)| folded(n).0))?,
            )
        };
        Ok(Self { automaton, names })
    }

    /// Each place a name appears on its own. A one-word name must start
    /// with a capital letter there, as names do, so "Grace" the person is
    /// found and "grace" the word is not.
    pub fn find(&self, text: &str, out: &mut Vec<Mention>) {
        let Some(ac) = &self.automaton else { return };
        let (copy, map) = folded(text);
        for m in ac.find_iter(&copy) {
            let (name, t) = &self.names[m.pattern().as_usize()];
            // Back to `text`: from the start of the first character to the
            // end of the last, which folding may have made longer.
            let start = map[m.start()];
            let last = map[m.end() - 1];
            let end = last + text[last..].chars().next().map_or(0, char::len_utf8);
            let found = &text[start..end];
            // Only a name needs a capital: an address or a domain does not.
            let one_word = t.class() == AliasClass::Name && !name.contains(char::is_whitespace);
            if !bounded(text, start, end)
                || (one_word && !found.chars().next().is_some_and(char::is_uppercase))
            {
                continue;
            }
            out.push(Mention {
                start,
                end,
                kind: Kind::Entity(*t),
                value: found.to_string(),
                source: Detector::Dictionary,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC Q22: display names join as people, but not a common word or an address.
    #[test]
    fn display_names_join_unless_a_common_word_or_an_address() {
        let names = Names::people(
            [
                "Dana Ruiz",
                "Support",
                "Billing Team",
                "Okonkwo",
                "dana@ruiz-events.example",
            ]
            .map(String::from),
        );
        let kept: Vec<&str> = names.0.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(kept, ["Dana Ruiz", "Billing Team", "Okonkwo"]);
        assert!(names.0.iter().all(|(_, t)| *t == EntityType::Person));
    }

    #[test]
    fn names_are_found_on_their_own_and_capitalized() {
        let mut names = Names::default();
        names.add("Dana Ruiz", EntityType::Person);
        names.add("Grace", EntityType::Person);
        names.add("Me", EntityType::Person);
        names.add("dana@ruiz-events.example", EntityType::Email);
        let d = Dictionary::new(&names).unwrap();
        let mut out = Vec::new();
        let text = "DANA RUIZ wrote; Grace agreed, by grace. Me too. Danaruiz no. dana@ruiz-events.example";
        d.find(text, &mut out);
        let got: Vec<&str> = out.iter().map(|m| &text[m.start..m.end]).collect();
        assert_eq!(got, ["DANA RUIZ", "Grace", "dana@ruiz-events.example"]);
    }

    /// Case beyond ASCII: "RENÉE" is "Renée", and "STRASSE" is "Straße".
    #[test]
    fn names_match_in_any_case_in_any_script() {
        let mut names = Names::default();
        names.add("Renée Dupont", EntityType::Person);
        names.add("Jürgen Straße", EntityType::Person);
        let d = Dictionary::new(&names).unwrap();
        let mut out = Vec::new();
        let text = "Thanks, RENÉE DUPONT, and jürgen strasse. Bye, Renée Dupont";
        d.find(text, &mut out);
        let got: Vec<&str> = out.iter().map(|m| &text[m.start..m.end]).collect();
        assert_eq!(got, ["RENÉE DUPONT", "jürgen strasse", "Renée Dupont"]);
    }
}
