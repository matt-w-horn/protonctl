//! Regex with validators (RFC section 6, Pipeline step 3): a match counts
//! only when its checksum, numbering plan or format rules hold, so ordinary
//! numbers and words stay as they are.

use std::net::{IpAddr, Ipv6Addr};
use std::str::FromStr as _;
use std::sync::LazyLock;

use regex::Regex;

use super::{Detector, Kind, Mention};
use crate::privacy::canon;
use crate::privacy::ident::EntityType;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a fixed pattern compiles")
}

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    // The local part may hold what RFC 5322 allows beside letters and
    // digits ("o'brien"), but starts with a letter or digit, so a quote
    // before an address stays out of it.
    re(
        r"(?i)\b[a-z0-9][a-z0-9.!#$%&'*+?^_`~-]*@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,24}\b",
    )
});
static URL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)\b(?:https?|ftp)://[^\s<>"'\[\](){}]+"#));
static DOMAIN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+([a-z]{2,24})\b"));
static IPV4: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:\d{1,3}\.){3}\d{1,3}\b"));
static IPV6: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b[0-9a-f]{0,4}(?::[0-9a-f]{0,4}){2,7}\b"));
static PHONE: LazyLock<Regex> = LazyLock::new(|| re(r"\+?\(?\d[\d\s().-]{5,18}\d"));
static CARD: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:\d[ -]?){12,18}\d\b"));
// ASCII digits: `\d` takes any script's, and `iban` cuts at byte 4.
static IBAN: LazyLock<Regex> = LazyLock::new(|| re(r"\b[A-Z]{2}[0-9]{2}(?: ?[A-Z0-9]){11,30}\b"));
static SSN: LazyLock<Regex> = LazyLock::new(|| re(r"\b(\d{3})-(\d{2})-(\d{4})\b"));
static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?i)\b(?:verification code|security code|one-time code|one time code|passcode|password|passwort|code|pin|otp)\b(\s+is\b|[ \t]*[:=])?[ \t]*([^\s"'<>]{4,64})"#,
    )
});
static ACCOUNT: LazyLock<Regex> = LazyLock::new(|| re(r#"(?:/Users/|/home/)([^/\s"'<>]+)"#));

/// Top-level domains taken as a domain in plain text. A name such as
/// "report.pdf" or "main.rs" must not read as a domain, so this is a short
/// list of common ones, not every TLD; addresses and URLs find the rest.
const TLDS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "int", "info", "biz", "io", "co", "me", "uk", "de",
    "fr", "nl", "be", "ch", "at", "se", "no", "dk", "fi", "es", "it", "pt", "ie", "ca", "us", "au",
    "nz", "jp", "cn", "kr", "in", "br", "mx", "ar", "za", "eu", "example", "test",
];

/// Where national numbers without a country code are read: the region in
/// `LANG`, else the United States.
static REGION: LazyLock<phonenumber::country::Id> = LazyLock::new(|| {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .ok()
        .and_then(|l| l.split(['_', '.']).nth(1).map(str::to_uppercase))
        .and_then(|r| phonenumber::country::Id::from_str(&r).ok())
        .unwrap_or(phonenumber::country::Id::US)
});

fn push(out: &mut Vec<Mention>, start: usize, end: usize, kind: Kind, value: String) {
    out.push(Mention {
        start,
        end,
        kind,
        value,
        source: Detector::Regex,
    });
}

fn entity(out: &mut Vec<Mention>, text: &str, start: usize, end: usize, t: EntityType) {
    push(
        out,
        start,
        end,
        Kind::Entity(t),
        text[start..end].to_string(),
    );
}

/// Every mention the patterns find in `text`, overlapping ones included.
pub fn find(text: &str, out: &mut Vec<Mention>) {
    for m in EMAIL.find_iter(text) {
        entity(out, text, m.start(), m.end(), EntityType::Email);
    }
    for m in URL.find_iter(text) {
        let s = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
        push(
            out,
            m.start(),
            m.start() + s.len(),
            Kind::Link,
            s.to_string(),
        );
    }
    for c in DOMAIN.captures_iter(text) {
        let (m, tld) = (c.get(0).expect("group 0"), &c[1]);
        if TLDS.contains(&tld.to_ascii_lowercase().as_str()) {
            entity(out, text, m.start(), m.end(), EntityType::Domain);
        }
    }
    for m in IPV4.find_iter(text) {
        if m.as_str().parse::<IpAddr>().is_ok() {
            entity(out, text, m.start(), m.end(), EntityType::Ip);
        }
    }
    for m in IPV6.find_iter(text) {
        let s = m.as_str();
        let groups = s.split(':').filter(|g| !g.is_empty()).count();
        if (s.contains("::") || groups == 8) && groups >= 2 && s.parse::<Ipv6Addr>().is_ok() {
            entity(out, text, m.start(), m.end(), EntityType::Ip);
        }
    }
    for m in PHONE.find_iter(text) {
        if let Some(e164) = phone(m.as_str()) {
            push(
                out,
                m.start(),
                m.end(),
                Kind::Entity(EntityType::Phone),
                e164,
            );
        }
    }
    for m in CARD.find_iter(text) {
        let digits: Vec<u32> = m.as_str().chars().filter_map(canon::digit).collect();
        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            entity(out, text, m.start(), m.end(), EntityType::Card);
        }
    }
    for m in IBAN.find_iter(text) {
        if iban(m.as_str()) {
            entity(out, text, m.start(), m.end(), EntityType::Iban);
        }
    }
    for c in SSN.captures_iter(text) {
        let [area, group, serial] = [&c[1], &c[2], &c[3]].map(canon::ascii_digits);
        if area != "000"
            && area != "666"
            && !area.starts_with('9')
            && group != "00"
            && serial != "0000"
        {
            let m = c.get(0).expect("group 0");
            entity(out, text, m.start(), m.end(), EntityType::NationalId);
        }
    }
    for c in SECRET.captures_iter(text) {
        let v = c.get(2).expect("group 2");
        let value = v.as_str().trim_end_matches(['.', ',', ';', ')', '!', '?']);
        let labelled = c.get(1).is_some_and(|s| s.as_str().contains([':', '=']));
        if value.len() >= 4 && (labelled || value.chars().any(|ch| ch.is_ascii_digit())) {
            entity(
                out,
                text,
                v.start(),
                v.start() + value.len(),
                EntityType::Secret,
            );
        }
    }
    for c in ACCOUNT.captures_iter(text) {
        let m = c.get(1).expect("group 1");
        entity(out, text, m.start(), m.end(), EntityType::Account);
    }
}

/// A phone number in E.164 when it is a valid number: with a country code
/// as written, or else in the local region.
pub fn phone(s: &str) -> Option<String> {
    let s = &canon::ascii_digits(s);
    let digits = s.chars().filter(char::is_ascii_digit).count();
    if !(7..=15).contains(&digits) {
        return None;
    }
    let region = if s.trim_start().starts_with('+') {
        None
    } else {
        Some(*REGION)
    };
    let n = phonenumber::parse(region, s).ok()?;
    phonenumber::is_valid(&n).then(|| n.format().mode(phonenumber::Mode::E164).to_string())
}

fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                if d * 2 > 9 { d * 2 - 9 } else { d * 2 }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn iban(s: &str) -> bool {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !(15..=34).contains(&compact.len()) {
        return false;
    }
    let (head, tail) = compact.split_at(4);
    let mut rem: u32 = 0;
    for c in tail.chars().chain(head.chars()) {
        let Some(v) = c.to_digit(36) else {
            return false;
        };
        for d in v.to_string().chars().filter_map(|d| d.to_digit(10)) {
            rem = (rem * 10 + d) % 97;
        }
    }
    rem == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<(String, Kind)> {
        let mut out = Vec::new();
        find(text, &mut out);
        let mut v: Vec<_> = super::super::settle(out)
            .into_iter()
            .map(|m| (text[m.start..m.end].to_string(), m.kind))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    fn has(text: &str, part: &str, t: EntityType) -> bool {
        found(text)
            .iter()
            .any(|(s, k)| s == part && *k == Kind::Entity(t))
    }

    #[test]
    fn validated_patterns_are_found() {
        assert!(has(
            "write to dana@ruiz-events.example today",
            "dana@ruiz-events.example",
            EntityType::Email
        ));
        assert!(has(
            "call +1 415 555 0132 now",
            "+1 415 555 0132",
            EntityType::Phone
        ));
        assert!(has(
            "card 4111 1111 1111 1111.",
            "4111 1111 1111 1111",
            EntityType::Card
        ));
        assert!(has(
            "IBAN DE89 3704 0044 0532 0130 00 ok",
            "DE89 3704 0044 0532 0130 00",
            EntityType::Iban
        ));
        assert!(has(
            "SSN 123-45-6789",
            "123-45-6789",
            EntityType::NationalId
        ));
        assert!(has("Your code is 482913.", "482913", EntityType::Secret));
        assert!(has("Password: hunter2", "hunter2", EntityType::Secret));
        assert!(has(
            "see /Users/matt/Documents",
            "matt",
            EntityType::Account
        ));
        assert!(has("host 192.168.1.20 up", "192.168.1.20", EntityType::Ip));
        assert!(has("v6 2001:db8::1 up", "2001:db8::1", EntityType::Ip));
        assert!(has(
            "by mx.proton.test;",
            "mx.proton.test",
            EntityType::Domain
        ));
        let links = found("see https://example.com/a?b=c, then");
        assert_eq!(
            links,
            vec![("https://example.com/a?b=c".to_string(), Kind::Link)]
        );
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        for text in [
            "report.pdf and main.rs",
            "card 4111 1111 1111 1112",
            "SSN 000-12-3456 and 900-12-3456",
            "a code review on Monday",
            "the password reset link",
            "meet at 10:30:00",
            "order 1234567",
            "version 1.2.3",
        ] {
            assert_eq!(found(text), Vec::new(), "{text}");
        }
    }

    /// An IBAN's digits are ASCII: text shaped like one with a digit from
    /// another script is no IBAN, and does not panic the detectors, as it
    /// did when the check cut it at a byte inside that digit.
    #[test]
    fn an_iban_with_a_digit_from_another_script_is_not_one() {
        // Arabic-Indic and Devanagari threes, of two and three bytes.
        for text in ["GB3\u{663}ABCDEFGHIJK", "GB\u{969}\u{969}ABCDEFGHIJK"] {
            assert_eq!(found(text), Vec::new(), "{text}");
        }
    }

    /// A number in another script's digits is found as its ASCII form is,
    /// with the same value, and refused by the same rules (B23).
    #[test]
    fn numbers_in_another_script_s_digits_are_found() {
        let arabic = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    c.to_digit(10)
                        .map_or(c, |d| char::from_u32(0x660 + d).unwrap())
                })
                .collect()
        };
        let values = |text: &str| -> Vec<(Kind, String)> {
            let mut out = Vec::new();
            find(text, &mut out);
            let mut v: Vec<_> = super::super::settle(out)
                .into_iter()
                .map(|m| (m.kind, crate::privacy::canon::compact(&m.value)))
                .collect();
            v.sort_by(|a, b| a.1.cmp(&b.1));
            v
        };
        for text in [
            "Call +1 415 555 0123 now",
            "Card 4111 1111 1111 1111 ok",
            "SSN 123-45-6789 ok",
        ] {
            let ascii = values(text);
            assert_eq!(ascii.len(), 1, "{text}");
            assert_eq!(values(&arabic(text)), ascii, "{}", arabic(text));
        }
        for refused in ["SSN 666-12-3456", "Card 4111 1111 1111 1112"] {
            assert_eq!(found(&arabic(refused)), Vec::new(), "{refused}");
        }
    }

    #[test]
    fn checksums() {
        assert!(luhn(&[4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]));
        assert!(iban("GB82 WEST 1234 5698 7654 32"));
        assert!(!iban("GB82 WEST 1234 5698 7654 33"));
        assert_eq!(phone("+44 20 7946 0958").as_deref(), Some("+442079460958"));
    }
}
