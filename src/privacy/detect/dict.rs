//! The name dictionary (RFC section 6, Pipeline step 2): display names and
//! addresses from the result's own headers, attendees and owners, and from
//! the process dictionary (Q22), found again wherever they appear in text,
//! with the short forms of each person's name and the initials of the
//! result's own people.

use std::collections::{BTreeMap, HashMap};

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
    /// Known people's names of two or three words, for misspellings.
    near: Vec<NearName>,
    /// `near` by word count and the first letter of the first word.
    near_index: HashMap<(usize, char), Vec<usize>>,
}

/// A known person's name, as the misspelling pass compares it.
struct NearName {
    full: String,
    /// Its words, given name first.
    words: Vec<Word>,
}

/// Letters OCR and typing swap for each other, which count as the same
/// letter when names are compared.
const CONFUSED: &[(char, char)] = &[
    ('o', '0'),
    ('l', '1'),
    ('i', '1'),
    ('l', 'i'),
    ('s', '5'),
    ('b', '8'),
    ('e', 'c'),
    ('u', 'n'),
];

fn same_letter(a: char, b: char) -> bool {
    a == b
        || CONFUSED
            .iter()
            .any(|&(x, y)| (a, b) == (x, y) || (a, b) == (y, x))
}

/// Edits between two words, 0, 1, or 2 for more: a substitution, an
/// insertion or deletion, or a swap of neighbours, with a confused letter
/// counting as no edit. One walk along both words, since the pass only
/// asks whether they are within one edit.
fn edits(a: &[char], b: &[char]) -> usize {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    match long.len() - short.len() {
        0 => {
            let differ: Vec<usize> = (0..long.len())
                .filter(|&i| !same_letter(long[i], short[i]))
                .take(3)
                .collect();
            match differ.as_slice() {
                [] => 0,
                [_] => 1,
                &[i, j] if j == i + 1 && long[i] == short[j] && long[j] == short[i] => 1,
                _ => 2,
            }
        }
        1 => {
            // The one extra letter of `long` is at the first difference.
            let at = (0..short.len())
                .find(|&i| !same_letter(long[i], short[i]))
                .unwrap_or(short.len());
            if (at..short.len()).all(|i| same_letter(long[i + 1], short[i])) {
                1
            } else {
                2
            }
        }
        _ => 2,
    }
}

/// A word of text or of a name as the misspelling pass compares it: in
/// lower case as written, and with the pairs of letters OCR reads for one
/// letter ("rn" for "m", "cl" for "d", "vv" for "w") as that letter.
struct Word {
    plain: Vec<char>,
    folded: Vec<char>,
}

fn lower(word: &str) -> Vec<char> {
    word.chars().flat_map(char::to_lowercase).collect()
}

fn word(w: &str) -> Word {
    let plain = lower(w);
    let s: String = plain.iter().collect();
    let folded = s
        .replace("rn", "m")
        .replace("cl", "d")
        .replace("vv", "w")
        .chars()
        .collect();
    Word { plain, folded }
}

/// Edits between two words, as written or with OCR's letter pairs folded,
/// whichever is fewer.
fn word_edits(a: &Word, b: &Word) -> usize {
    edits(&a.plain, &b.plain).min(edits(&a.folded, &b.folded))
}

/// The letter a name's first word is indexed under: its first letter, or
/// the letter a digit OCR made of it stands for.
fn index_letter(c: char) -> char {
    CONFUSED
        .iter()
        .find(|&&(_, digit)| digit == c && c.is_ascii_digit())
        .map_or(c, |&(letter, _)| letter)
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
        let mut near = Vec::new();
        let mut near_index: HashMap<(usize, char), Vec<usize>> = HashMap::new();
        for (full, t) in &names.0 {
            let p = parts(full);
            let letters: usize = p.iter().map(|w| w.chars().count()).sum();
            if *t != EntityType::Person || !(2..=3).contains(&p.len()) || letters < 6 {
                continue;
            }
            let words: Vec<Word> = p.iter().map(|w| word(w)).collect();
            let first = index_letter(words[0].plain[0]);
            near_index
                .entry((words.len(), first))
                .or_default()
                .push(near.len());
            near.push(NearName {
                full: full.clone(),
                words,
            });
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
        Ok(Self {
            automaton,
            entries,
            near,
            near_index,
        })
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
        self.find_near(text, out);
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

    /// B17: each run of two or three words that is a misspelling of a known
    /// person's name, from typing or OCR: every word within one edit of
    /// the name's (a confused letter, such as 0 for o, is none), at most
    /// one edit in all for a name of fewer than ten letters and two for a
    /// longer one, a word of fewer than three letters spelled right, and a
    /// capital or digit first. The name as written is the dictionary's
    /// own match, which wins over this one.
    fn find_near(&self, text: &str, out: &mut Vec<Mention>) {
        if self.near.is_empty() {
            return;
        }
        let mut words: Vec<(usize, usize, Word)> = Vec::new();
        let mut start = None;
        for (i, c) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            match (c.is_alphanumeric(), start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    words.push((s, i, word(&text[s..i])));
                    start = None;
                }
                _ => {}
            }
        }
        for (i, (first_start, _, first)) in words.iter().enumerate() {
            let opens = text[*first_start..]
                .chars()
                .next()
                .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
            let Some(&initial) = first.plain.first() else {
                continue;
            };
            if !opens {
                continue;
            }
            for k in 2..=3 {
                let Some(window) = words.get(i..i + k) else {
                    break;
                };
                let key = (k, index_letter(initial));
                let mut of: Vec<String> = Vec::new();
                for &n in self.near_index.get(&key).into_iter().flatten() {
                    let name = &self.near[n];
                    let mut total = 0;
                    let mut fits = true;
                    for ((_, _, w), nw) in window.iter().zip(&name.words) {
                        let e = word_edits(w, nw);
                        if e > 1 || (e > 0 && nw.plain.len() < 3) {
                            fits = false;
                            break;
                        }
                        total += e;
                    }
                    let letters: usize = name.words.iter().map(|w| w.plain.len()).sum();
                    let budget = if letters < 10 { 1 } else { 2 };
                    let exact = window
                        .iter()
                        .zip(&name.words)
                        .all(|((_, _, w), nw)| w.plain == nw.plain);
                    if fits && total <= budget && !exact && !of.contains(&name.full) {
                        of.push(name.full.clone());
                    }
                }
                if !of.is_empty() {
                    of.sort();
                    let end = window[k - 1].1;
                    out.push(Mention {
                        start: *first_start,
                        end,
                        kind: Kind::Entity(EntityType::Person),
                        value: text[*first_start..end].to_string(),
                        source: Detector::Dictionary,
                        form: Form::Near(of),
                    });
                }
            }
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
                    Form::Short(of) | Form::Near(of) => of.iter().map(String::as_str).collect(),
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

    /// B17: a known name misspelled by typing (a swap, a letter added or
    /// lost) or by OCR (0 for o, 1 for i, n for u) is found as a
    /// misspelling of it; a name that differs more, another known name
    /// spelled right, and a run in lower case are not.
    #[test]
    fn misspelled_names_are_found_as_near_their_name() {
        let mut names = Names::default();
        for n in ["Dana Ruiz", "Kenji Watanabe", "Jon Lee", "John Lee"] {
            names.add(n, EntityType::Person);
        }
        let d = Dictionary::new(&names).unwrap();
        let text = "Dnaa Ruiz, Kenj1 Watanabe and Dana Rniz met Jon Lee and Dana Rulz; \
            Dina Rios, Ken Watanabe and dnaa ruiz did not.";
        let mut out = Vec::new();
        d.find(text, &mut out);
        let kept = super::super::settle(out);
        let got: Vec<(&str, Vec<&str>)> = kept
            .iter()
            .filter_map(|m| match &m.form {
                Form::Near(of) => Some((
                    &text[m.start..m.end],
                    of.iter().map(String::as_str).collect(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            got,
            [
                ("Dnaa Ruiz", vec!["Dana Ruiz"]),
                ("Kenj1 Watanabe", vec!["Kenji Watanabe"]),
                ("Dana Rniz", vec!["Dana Ruiz"]),
                ("Dana Rulz", vec!["Dana Ruiz"]),
            ]
        );
        assert_eq!(word_edits(&word("Lce"), &word("Lee")), 0);
        assert_eq!(word_edits(&word("Jonh"), &word("John")), 1);
        assert_eq!(word_edits(&word("Rarnan"), &word("Raman")), 0);
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
