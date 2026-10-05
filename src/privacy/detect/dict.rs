//! The name dictionary (RFC section 6, Pipeline step 2): display names and
//! addresses from the result's own headers, attendees and owners, and from
//! the process dictionary (Q22), found again wherever they appear in text,
//! with the short forms of each person's name and the initials of the
//! result's own people.

use std::collections::BTreeMap;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

use super::{Detector, Form, Kind, Mention, bounded};
use crate::privacy::canon;
use crate::privacy::ident::{AliasClass, EntityType};
use crate::privacy::words;

/// Names to look for, each with its type.
#[derive(Debug, Default, Clone)]
pub struct Names(Vec<(String, EntityType)>);

impl Names {
    /// Add a name, unless it is too short or has no letter to be one: "Me"
    /// or "Al" would match ordinary words. A name written in a script
    /// without spaces may have two characters, as many Chinese names do.
    pub fn add(&mut self, text: &str, t: EntityType) {
        let text = text.trim();
        let least = if text.chars().all(super::unspaced) {
            2
        } else {
            3
        };
        if text.chars().count() >= least && text.chars().any(char::is_alphabetic) {
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

    pub fn iter(&self) -> impl Iterator<Item = &(String, EntityType)> {
        self.0.iter()
    }
}

/// Words after a name that are not part of it, compared in lower case
/// without a dot.
const SUFFIXES: &[&str] = &[
    "jr", "jnr", "sr", "snr", "ii", "iii", "iv", "phd", "md", "esq",
];

/// Capitals that are words or common abbreviations, never taken as a
/// person's initials.
const NOT_INITIALS: &[&str] = &[
    "AM", "AN", "AS", "AT", "BE", "BY", "CC", "DO", "EU", "FW", "GO", "HE", "HR", "ID", "IF", "IN",
    "IP", "IS", "IT", "ME", "MY", "NB", "NO", "OF", "OK", "ON", "OR", "OS", "PC", "PM", "PR", "PS",
    "QA", "RE", "SO", "TO", "TV", "UI", "UK", "UN", "UP", "US", "UX", "VP", "WE",
];

/// The words of a person's name, given name first, without an honorific
/// before it or a suffix after it: "Dr. Chen, Alice" gives `Alice` and
/// `Chen`.
fn parts(name: &str) -> Vec<String> {
    let ordered = match name.split_once(',') {
        Some((last, first))
            if !first.contains(',') && !last.trim().is_empty() && !first.trim().is_empty() =>
        {
            format!("{} {}", first.trim(), last.trim())
        }
        _ => name.to_string(),
    };
    let mut words: Vec<String> = ordered.split_whitespace().map(String::from).collect();
    while words.len() > 2 && canon::is_honorific(&words[0]) {
        words.remove(0);
    }
    while words.len() > 2
        && words.last().is_some_and(|w| {
            SUFFIXES.contains(&w.trim_end_matches(['.', ',']).to_lowercase().as_str())
        })
    {
        words.pop();
    }
    words
}

/// Whether a short form could stand for a name: two letters or more, not
/// an initial, an honorific or a common word, which would turn every use
/// of the word into an alias.
fn usable(form: &str) -> bool {
    let letters = form.chars().filter(|c| c.is_alphabetic()).count();
    letters >= 2
        && !form.ends_with('.')
        && !canon::is_honorific(form)
        && (form.contains(char::is_whitespace) || !words::contains(&form.to_lowercase()))
}

/// The short forms of a person's name (RFC section 6, Canonical values):
/// the given name, the surname, and the given name with the surname when
/// there are middle names.
fn short_forms(name: &str) -> Vec<String> {
    let p = parts(name);
    if p.len() < 2 {
        return Vec::new();
    }
    let (first, last) = (&p[0], &p[p.len() - 1]);
    let mut out = vec![first.clone(), last.clone()];
    if p.len() > 2 {
        out.push(format!("{first} {last}"));
    }
    out.retain(|f| usable(f));
    out
}

/// A person's initials, in capitals, plain and dotted ("JL", "J.L.",
/// "J. L."), for a name of two or three words that each start with a
/// letter that has a capital.
fn initials(name: &str) -> Vec<String> {
    let p = parts(name);
    if !(2..=3).contains(&p.len()) {
        return Vec::new();
    }
    let letters: Option<Vec<char>> = p
        .iter()
        .map(|w| {
            w.chars()
                .next()
                .filter(|c| c.is_alphabetic() && c.to_uppercase().ne(c.to_lowercase()))
                .and_then(|c| c.to_uppercase().next())
        })
        .collect();
    let Some(letters) = letters else {
        return Vec::new();
    };
    let plain: String = letters.iter().collect();
    if NOT_INITIALS.contains(&plain.as_str()) || words::contains(&plain.to_lowercase()) {
        return Vec::new();
    }
    let each: Vec<String> = letters.iter().map(|c| format!("{c}.")).collect();
    vec![plain, each.concat(), each.join(" ")]
}

/// One string the dictionary looks for.
struct Entry {
    text: String,
    t: EntityType,
    form: Form,
    /// Found only in capitals, as initials are.
    capitals: bool,
    /// A form of a name in this result's own headers, found anywhere; a
    /// one-word form of a name known only to the process is found only
    /// inside a sentence, where its capital says it is a name.
    local: bool,
}

/// Whether the word at `start` begins a sentence: nothing before it but
/// opening marks and spaces since the text's start, a line break or a
/// sentence's end.
fn sentence_start(text: &str, start: usize) -> bool {
    for c in text[..start].chars().rev() {
        match c {
            '\n' | '.' | '!' | '?' | ':' | ';' => return true,
            c if c.is_whitespace() => {}
            '"' | '\'' | '“' | '‘' | '(' | '[' | '*' | '-' | '•' => {}
            _ => return false,
        }
    }
    true
}

pub struct Dictionary {
    automaton: Option<AhoCorasick>,
    entries: Vec<Entry>,
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
    /// The names, and the short forms of every person's name.
    pub fn new(names: &Names) -> anyhow::Result<Self> {
        Self::with_initials_of(names, &Names::default())
    }

    /// As `new`, with the initials of the people in `local` too: the
    /// result's own people, the only ones whose initials are looked for,
    /// since two capitals match too much else.
    pub fn with_initials_of(names: &Names, local: &Names) -> anyhow::Result<Self> {
        // By folded text: a name as written wins over a form of another
        // name, and one form of several names stands for all of them.
        let mut by_text: BTreeMap<String, Entry> = BTreeMap::new();
        for (name, t) in &names.0 {
            by_text.entry(folded(name).0).or_insert_with(|| Entry {
                text: name.clone(),
                t: *t,
                form: Form::Whole,
                capitals: false,
                local: true,
            });
        }
        let forms = names
            .0
            .iter()
            .filter(|(_, t)| *t == EntityType::Person)
            .flat_map(|(n, t)| short_forms(n).into_iter().map(move |f| (n, *t, f, false)));
        let capitals = local
            .0
            .iter()
            .filter(|(_, t)| *t == EntityType::Person)
            .flat_map(|(n, t)| initials(n).into_iter().map(move |f| (n, *t, f, true)));
        for (full, t, text, capitals) in forms.chain(capitals) {
            let is_local = local.0.iter().any(|(n, _)| n == full);
            let entry = by_text.entry(folded(&text).0).or_insert_with(|| Entry {
                text,
                t,
                form: Form::Short(Vec::new()),
                capitals,
                local: false,
            });
            entry.capitals &= capitals;
            entry.local |= is_local;
            if let Form::Short(of) = &mut entry.form
                && !of.contains(full)
            {
                of.push(full.clone());
                of.sort();
            }
        }
        let (keys, entries): (Vec<String>, Vec<Entry>) = by_text.into_iter().unzip();
        let automaton = if keys.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(&keys)?,
            )
        };
        Ok(Self { automaton, entries })
    }

    /// Each place a name appears on its own. A one-word name must not start
    /// with a lower-case letter there, so "Grace" the person is found and
    /// "grace" the word is not; a script without case, such as Chinese, has
    /// no lower-case word to mistake for the name. A name written wholly in
    /// a script without spaces is found inside running text too, where
    /// letters stand on both sides of it. Initials count only in capitals,
    /// and a one-word form of a name the result does not hold only inside
    /// a sentence.
    pub fn find(&self, text: &str, out: &mut Vec<Mention>) {
        let Some(ac) = &self.automaton else { return };
        let (copy, map) = folded(text);
        for m in ac.find_iter(&copy) {
            let e = &self.entries[m.pattern().as_usize()];
            // Back to `text`: from the start of the first character to the
            // end of the last, which folding may have made longer.
            let start = map[m.start()];
            let last = map[m.end() - 1];
            let end = last + text[last..].chars().next().map_or(0, char::len_utf8);
            let found = &text[start..end];
            // Only a name needs a capital: an address or a domain does not.
            let one_word = e.t.class() == AliasClass::Name && !e.text.contains(char::is_whitespace);
            let run_in = e.text.chars().all(super::unspaced);
            if !(run_in || bounded(text, start, end))
                || (one_word && found.chars().next().is_some_and(char::is_lowercase))
                || (e.capitals && found.chars().any(char::is_lowercase))
                || (one_word && !e.local && sentence_start(text, start))
            {
                continue;
            }
            out.push(Mention {
                start,
                end,
                kind: Kind::Entity(e.t),
                value: found.to_string(),
                source: Detector::Dictionary,
                form: e.form.clone(),
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

    /// A one-word name in a script without case, such as a Chinese name
    /// with no space in it, is found as a capitalized one is.
    #[test]
    fn a_one_word_name_in_a_script_without_case_is_found() {
        let mut names = Names::default();
        names.add("王小明", EntityType::Person);
        let d = Dictionary::new(&names).unwrap();
        let mut out = Vec::new();
        let text = "Review with 王小明 (Friday)";
        d.find(text, &mut out);
        let got: Vec<&str> = out.iter().map(|m| &text[m.start..m.end]).collect();
        assert_eq!(got, ["王小明"]);
    }

    /// B25: a Chinese or Japanese name inside running text in its own
    /// script, which has no spaces, is found, a two-character one too; a
    /// Latin name inside a longer word is still not.
    #[test]
    fn a_name_in_running_text_without_spaces_is_found() {
        let mut names = Names::default();
        names.add("王小明", EntityType::Person);
        names.add("李明", EntityType::Person);
        names.add("やまだはなこ", EntityType::Person);
        names.add("Dana", EntityType::Person);
        let d = Dictionary::new(&names).unwrap();
        let mut out = Vec::new();
        let text = "与王小明开会，和李明说，やまだはなこさんへ。Danaruiz";
        d.find(text, &mut out);
        let got: Vec<&str> = out.iter().map(|m| &text[m.start..m.end]).collect();
        assert_eq!(got, ["王小明", "李明", "やまだはなこ"]);
    }

    /// B15 and B16: a person's given name, surname and name without middle
    /// names are found, and, for the result's own people only, their
    /// initials in capitals; one form of two people stands for both; and a
    /// one-word form of a name the result does not hold counts only inside
    /// a sentence, not where any word starts with a capital.
    #[test]
    fn short_forms_and_initials_are_found() {
        let mut names = Names::default();
        for n in [
            "Dana Ruiz",
            "Dr. Kenji Q. Watanabe",
            "Lee, John",
            "Jane Lee",
            "Will Mason",
        ] {
            names.add(n, EntityType::Person);
        }
        let mut local = Names::default();
        local.add("Dr. Kenji Q. Watanabe", EntityType::Person);
        let d = Dictionary::with_initials_of(&names, &local).unwrap();
        let mut out = Vec::new();
        let text = "So Ruiz told Dana; Kenji Watanabe and Lee met KQW, K. Q. W. and DR; kqw. \
            Will you ask Will?";
        d.find(text, &mut out);
        let got: Vec<(&str, Vec<&str>)> = out
            .iter()
            .map(|m| {
                let of = match &m.form {
                    Form::Short(of) => of.iter().map(String::as_str).collect(),
                    Form::Whole => Vec::new(),
                };
                (&text[m.start..m.end], of)
            })
            .collect();
        let kenji = "Dr. Kenji Q. Watanabe";
        assert_eq!(
            got,
            [
                ("Ruiz", vec!["Dana Ruiz"]),
                ("Dana", vec!["Dana Ruiz"]),
                ("Kenji Watanabe", vec![kenji]),
                ("Lee", vec!["Jane Lee", "Lee, John"]),
                ("KQW", vec![kenji]),
                ("K. Q. W.", vec![kenji]),
                ("Will", vec!["Will Mason"]),
            ]
        );
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
