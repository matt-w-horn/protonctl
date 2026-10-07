//! Canonical forms (RFC section 6): one value in any spelling gets one alias,
//! and canonicalizing twice changes nothing. A rule here must never merge
//! two people, so it errs toward keeping forms apart.

use std::net::IpAddr;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

/// Honorifics, removed only before a full name (a given name and a surname):
/// before a surname alone one is all that tells "Mr Chen" from "Mme Chen".
/// Compared in lower case, without a trailing dot.
const HONORIFICS: &[&str] = &[
    "mr", "mrs", "ms", "miss", "mx", "dr", "prof", "sir", "dame", "herr", "frau", "m", "mme",
    "mlle", "sr", "sra", "srta", "sig", "sig.ra", "dott", "dott.ssa", "dhr", "mevr", "pan", "pani",
    "sri", "smt",
];

/// Whether `word` is an honorific, in any case, with or without its dot.
pub fn is_honorific(word: &str) -> bool {
    HONORIFICS.contains(&word.trim_end_matches('.').to_lowercase().as_str())
}

/// A person's, organization's or place's name: NFKC; "Last, First" reordered;
/// leading honorifics removed before a full name; Unicode case folding
/// without Turkish rules; diacritics removed from Latin, Greek and Cyrillic
/// letters only, since in other scripts the marks are letters; whitespace
/// collapsed.
pub fn name(s: &str) -> String {
    // Each pass is close to idempotent; repeating until nothing changes makes it so.
    let mut out = name_pass(s);
    for _ in 0..4 {
        let again = name_pass(&out);
        if again == out {
            break;
        }
        out = again;
    }
    out
}

fn name_pass(s: &str) -> String {
    let s: String = s.nfkc().collect();
    let s = match s.split_once(',') {
        Some((last, first))
            if !first.contains(',') && !last.trim().is_empty() && !first.trim().is_empty() =>
        {
            format!("{} {}", first.trim(), last.trim())
        }
        _ => s,
    };
    let mut words: Vec<&str> = s.split_whitespace().collect();
    while words.len() >= 3 {
        if !is_honorific(words[0]) {
            break;
        }
        words.remove(0);
    }
    let folded = caseless::default_case_fold_str(&words.join(" "));
    let stripped = strip_marks(&folded.nfkc().collect::<String>());
    stripped
        .nfkc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Latin, Greek and Cyrillic letters, whose combining marks are diacritics.
fn marks_are_diacritics(c: char) -> bool {
    matches!(c as u32,
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF
        | 0x370..=0x3FF | 0x1F00..=0x1FFF | 0x400..=0x52F)
}

fn strip_marks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut base_strips = false;
    for c in s.nfd() {
        if is_combining_mark(c) {
            if !base_strips {
                out.push(c);
            }
        } else {
            base_strips = marks_are_diacritics(c);
            out.push(c);
        }
    }
    out.nfc().collect()
}

/// An email address: lower case, with NFKC on the domain.
pub fn email(s: &str) -> String {
    let s = s.trim();
    match s.rsplit_once('@') {
        // NFKC before lower case: it can give capitals ("𝒢" to "G").
        Some((local, domain)) => format!(
            "{}@{}",
            local.to_lowercase(),
            domain.nfkc().collect::<String>().to_lowercase()
        ),
        None => s.to_lowercase(),
    }
}

/// Whether `c` is a decimal digit in some script (Unicode's `Nd`).
fn is_decimal(c: char) -> bool {
    static ND: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\A\p{Nd}\z").expect("a fixed pattern"));
    ND.is_match(c.encode_utf8(&mut [0; 4]))
}

/// The value of a decimal digit in any script. Unicode encodes each
/// script's digits as one run from 0 to 9, so a digit's value is how far
/// it is from the start of its run.
pub fn digit(c: char) -> Option<u32> {
    if c.is_ascii() {
        return c.to_digit(10);
    }
    if !is_decimal(c) {
        return None;
    }
    let mut at = u32::from(c);
    let mut below = 0;
    while char::from_u32(at - 1).is_some_and(is_decimal) {
        at -= 1;
        below += 1;
    }
    Some(below % 10)
}

/// `s` with each decimal digit, in any script, as an ASCII digit.
pub fn ascii_digits(s: &str) -> String {
    s.chars()
        .map(|c| digit(c).and_then(|d| char::from_digit(d, 10)).unwrap_or(c))
        .collect()
}

/// A card number or IBAN: digits and capital letters only.
pub fn compact(s: &str) -> String {
    ascii_digits(s)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// A secret, national ID or account name: as found, without whitespace.
pub fn plain(s: &str) -> String {
    ascii_digits(s)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// A domain name: lower case, without a trailing dot. Dots and whitespace
/// come off the end together, so a space left before a dot goes too and a
/// second pass changes nothing.
pub fn domain(s: &str) -> String {
    s.trim_start()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_lowercase()
}

/// An IP address in its standard form (IPv6 compressed), or as given in
/// lower case when it does not parse.
pub fn ip(s: &str) -> String {
    s.trim()
        .parse::<IpAddr>()
        .map_or_else(|_| s.trim().to_lowercase(), |a| a.to_string())
}

/// A URL, for numbering links within one result: scheme, if any, and host
/// in lower case, the path and query kept, the fragment removed.
pub fn url(s: &str) -> String {
    let s = s.split('#').next().unwrap_or_default();
    let (scheme, rest) = match s.split_once("://") {
        Some((scheme, rest)) => (format!("{}://", scheme.to_lowercase()), rest),
        None => (String::new(), s),
    };
    let cut = rest.find(['/', '?']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(cut);
    format!("{scheme}{}{tail}", host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn spellings_of_one_name_meet() {
        let one = name("Alice Chen");
        for other in [
            "Chen, Alice",
            "ALICE  CHEN",
            "Dr Alice Chen",
            "Dr. Alice Chen",
            " alice chen ",
        ] {
            assert_eq!(name(other), one, "{other}");
        }
        assert_eq!(name("Zoë Müller"), "zoe muller");
        assert_eq!(name("Straße"), "strasse");
    }

    #[test]
    fn rules_never_merge_two_people() {
        // An honorific before a surname alone is all that tells them apart.
        assert_ne!(name("Mr Chen"), name("Mme Chen"));
        // In Devanagari the marks are letters.
        assert_ne!(name("कमल"), name("कमला"));
        assert_ne!(name("Chen, Alice, Bob"), name("Alice Chen"));
        // Section 7's cases: honorifics that name different people, a
        // generational suffix, and a Thai tone mark, which is a letter's
        // part, not an accent.
        assert_ne!(name("M. Chen"), name("Mme Chen"));
        assert_ne!(name("Mr Chen"), name("Ms Chen"));
        assert_ne!(name("John Smith Sr."), name("John Smith"));
        assert_ne!(name("\u{0E01}\u{0E32}"), name("\u{0E01}\u{0E48}\u{0E32}"));
    }

    #[test]
    fn other_forms() {
        assert_eq!(email("Alice.Chen@Example.ORG"), "alice.chen@example.org");
        assert_eq!(
            compact("de89 3704-0044 0532 0130 00"),
            "DE89370400440532013000"
        );
        assert_eq!(plain(" 123 456 "), "123456");
        assert_eq!(domain("Example.COM."), "example.com");
        assert_eq!(ip("2001:0db8:0000:0000:0000:0000:0000:0001"), "2001:db8::1");
        assert_eq!(
            url("HTTPS://Example.COM/Path?q=A#frag"),
            "https://example.com/Path?q=A"
        );
        assert_eq!(url("Zoom.US/j/1?pwd=A#x"), "zoom.us/j/1?pwd=A");
        assert_eq!(url("WebCal://Proton.ME/c?K=A"), "webcal://proton.me/c?K=A");
    }

    /// Each script's decimal digits read as their values. `digit` relies on
    /// every run of decimal digits being whole runs of 0 to 9.
    #[test]
    fn every_script_s_digits_read_as_their_values() {
        for (c, v) in [
            ('0', 0),
            ('9', 9),
            ('\u{0663}', 3),  // Arabic-Indic
            ('\u{06F7}', 7),  // Extended Arabic-Indic
            ('\u{0969}', 3),  // Devanagari
            ('\u{09EB}', 5),  // Bengali
            ('\u{0E55}', 5),  // Thai
            ('\u{FF19}', 9),  // fullwidth
            ('\u{1D7D5}', 7), // mathematical bold, the second of five runs
        ] {
            assert_eq!(digit(c), Some(v), "{c}");
        }
        for c in ['a', '\u{00B2}', '\u{00BD}', '\u{2167}'] {
            assert_eq!(digit(c), None, "{c}");
        }
        let mut run = 0;
        for c in (0..=0x0010_FFFF).filter_map(char::from_u32) {
            if digit(c).is_some() {
                run += 1;
            } else {
                assert_eq!(run % 10, 0, "a run of digits ends before {c:?}");
                run = 0;
            }
        }
        assert_eq!(ascii_digits("+\u{0661} \u{0664}\u{0661}\u{0665}"), "+1 415");
        assert_eq!(compact("\u{0664}\u{0661}\u{0661}\u{0661} 1111"), "41111111");
        assert_eq!(plain("\u{0661}\u{0662}\u{0663}-45"), "123-45");
    }

    proptest! {
        #[test]
        fn canonical_forms_are_fixed_points(s in "\\PC{0,40}") {
            prop_assert_eq!(name(&name(&s)), name(&s));
            prop_assert_eq!(email(&email(&s)), email(&s));
            prop_assert_eq!(compact(&compact(&s)), compact(&s));
            prop_assert_eq!(domain(&domain(&s)), domain(&s));
            prop_assert_eq!(url(&url(&s)), url(&s));
        }
    }
}
