//! The evaluation of the privacy layer (RFC section 9, defect D1): aliases
//! mode run over a labeled synthetic corpus that holds each name,
//! organization and project in the forms real results showed passing
//! raw, with what came back reported per form. The report is a snapshot,
//! so a fix of a defect shows as a change in its row.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::detect::dict::Names;
use super::key::Keys;
use super::pipeline::{Context, run};
use crate::tool::Tool;

/// One planted mention: `text` stands for `full`, an entity of type
/// `expect`, and the result's own headers name `full` when `local`.
struct Case {
    form: &'static str,
    expect: &'static str,
    full: &'static str,
    text: &'static str,
    local: bool,
}

const fn case(
    form: &'static str,
    expect: &'static str,
    full: &'static str,
    text: &'static str,
    local: bool,
) -> Case {
    Case {
        form,
        expect,
        full,
        text,
        local,
    }
}

/// The corpus. Every name is synthetic. People are Dana Ruiz, Kenji
/// Watanabe, Priya Raman (known to the process only), John and Jane Lee
/// (who share a surname) and 王小明; organizations are Acme Billing and
/// Notely, which send mail, and Globex and Initech, which never do;
/// Falcon and Bluebird are projects.
const CASES: &[Case] = &[
    // D2: full names are the baseline; short forms are the defect.
    case(
        "person full, in headers",
        "person",
        "Dana Ruiz",
        "Dana Ruiz",
        true,
    ),
    case(
        "person full, in headers",
        "person",
        "Kenji Watanabe",
        "Kenji Watanabe",
        true,
    ),
    case(
        "person full, process only",
        "person",
        "Priya Raman",
        "Priya Raman",
        false,
    ),
    case("person surname", "person", "Dana Ruiz", "Ruiz", true),
    case(
        "person surname",
        "person",
        "Kenji Watanabe",
        "Watanabe",
        true,
    ),
    case("person surname", "person", "Priya Raman", "Raman", false),
    case(
        "person surname, honorific",
        "person",
        "Kenji Watanabe",
        "Dr. Watanabe",
        true,
    ),
    case("person surname, shared", "person", "John Lee", "Lee", true),
    case("person given name", "person", "Dana Ruiz", "Dana", true),
    case(
        "person given name",
        "person",
        "Kenji Watanabe",
        "Kenji",
        true,
    ),
    case(
        "person middle initial",
        "person",
        "Dana Ruiz",
        "Dana M. Ruiz",
        true,
    ),
    // D3
    case("person initials", "person", "Dana Ruiz", "DR", true),
    case("person initials", "person", "Kenji Watanabe", "KW", true),
    case(
        "person initials, dotted",
        "person",
        "Kenji Watanabe",
        "K.W.",
        true,
    ),
    // D4
    case(
        "person misspelled, typing",
        "person",
        "Dana Ruiz",
        "Dnaa Ruiz",
        true,
    ),
    case(
        "person misspelled, typing",
        "person",
        "Kenji Watanabe",
        "Kenji Watanbe",
        true,
    ),
    case(
        "person misspelled, OCR",
        "person",
        "Dana Ruiz",
        "Dana Ru1z",
        true,
    ),
    case(
        "person misspelled, OCR",
        "person",
        "Kenji Watanabe",
        "Kenj1 Watanabe",
        true,
    ),
    case(
        "person misspelled, OCR",
        "person",
        "Priya Raman",
        "Priya Rarnan",
        false,
    ),
    // B25: a name in running text in a script without spaces.
    case(
        "person in run-in CJK text",
        "person",
        "王小明",
        "与王小明开会",
        true,
    ),
    // D7
    case(
        "organization that sends mail",
        "organization",
        "Acme Billing",
        "Acme Billing",
        true,
    ),
    case(
        "organization that sends mail",
        "organization",
        "Notely",
        "Notely",
        true,
    ),
    case(
        "organization never in mail",
        "organization",
        "Globex Corporation",
        "Globex Corporation",
        false,
    ),
    case(
        "organization never in mail",
        "organization",
        "Initech LLC",
        "Initech LLC",
        false,
    ),
    // D5
    case(
        "project in text",
        "project",
        "Falcon",
        "the Falcon rewrite",
        false,
    ),
    case(
        "project in text",
        "project",
        "Bluebird",
        "Bluebird launch",
        false,
    ),
];

/// Each person's and organization's address, for the From of a local case.
fn address(full: &str) -> String {
    match full {
        "Dana Ruiz" => "dana@ruiz-events.example".into(),
        "Kenji Watanabe" => "kenji@watanabe-design.example".into(),
        "John Lee" => "john@lee-family.example".into(),
        "王小明" => "xm@wang.example".into(),
        "Acme Billing" => "billing@acme.example".into(),
        "Notely" => "no-reply@notely.example".into(),
        other => format!("{}@example.com", other.to_lowercase().replace(' ', ".")),
    }
}

/// What the process knows (RFC Q22): every correspondent's display name.
fn process_names() -> Names {
    Names::people(
        [
            "Dana Ruiz",
            "Kenji Watanabe",
            "Priya Raman",
            "John Lee",
            "Jane Lee",
            "王小明",
            "Acme Billing",
            "Notely",
            "Sam Okafor",
        ]
        .map(String::from),
    )
}

/// One case as a message the pipeline rewrites: its form between « and »
/// in the body, from the person it names when the case is local.
fn message(c: &Case) -> Value {
    let from = if c.local {
        format!("{} <{}>", c.full, address(c.full))
    } else {
        "Sam Okafor <sam@okafor.example>".into()
    };
    json!({ "message": {
        "from": from,
        "to": ["Sam Okafor <sam@okafor.example>"],
        "subject": "Notes",
        "body": format!("Notes: «{}» said yes.", c.text),
    } })
}

#[derive(Default)]
struct Row {
    n: usize,
    raw: usize,
    part: usize,
    split: usize,
    linked: usize,
    mistyped: usize,
}

/// A word of `name` (two letters or more) standing alone in `text`.
fn has_word_of(text: &str, name: &str) -> bool {
    name.split_whitespace()
        .map(|w| w.trim_matches('.'))
        .filter(|w| w.chars().count() >= 2)
        .any(|w| {
            Regex::new(&format!(
                r"(?i)(^|[^\p{{L}}\p{{N}}]){}($|[^\p{{L}}\p{{N}}])",
                regex::escape(w)
            ))
            .expect("an escaped word")
            .is_match(text)
        })
}

/// The table, one row per form: how many mentions, and of those how many
/// came back whole (`raw`), with a word of the name left (`part`), with an
/// alias other than the one the name's full form has in the same result
/// (`split`), of those how many name that alias in `maybeSameAs`
/// (`linked`), and how many carry another entity type (`mistyped`).
fn report() -> String {
    static BETWEEN: LazyLock<Regex> = LazyLock::new(|| Regex::new("«([^»]*)»").expect("fixed"));
    static ALIAS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[a-z]+(?:-[a-z]+){2,}").expect("fixed"));
    let keys = Keys::derive(&[9; 32]);
    let names = process_names();
    let mut rows: BTreeMap<(&str, &str), Row> = BTreeMap::new();
    for c in CASES {
        let ctx = Context {
            tool: Tool::GetMessage,
            query: None,
            you: Some("sam@okafor.example"),
            names: &names,
            incomplete: &[],
        };
        let out = run(message(c), &keys, &ctx).expect("the pipeline rewrites every case");
        let body = out["message"]["body"].as_str().unwrap_or_default();
        let between = BETWEEN
            .captures(body)
            .map(|m| m[1].to_string())
            .unwrap_or_default();
        let row = rows.entry((c.form, c.expect)).or_default();
        row.n += 1;
        let text = out.to_string();
        if text.contains(c.text) {
            row.raw += 1;
            continue;
        }
        if has_word_of(&between, c.full) {
            row.part += 1;
        }
        let entities = &out["entities"];
        // The alias the full form has in this result, from the From header.
        let full_alias = out["message"]["from"]
            .as_str()
            .and_then(|f| f.split_once(" <"))
            .map(|(p, _)| p.to_string())
            .filter(|_| c.local);
        let alias = ALIAS.find(&between).map(|m| m.as_str().to_string());
        if let (Some(full), Some(alias)) = (&full_alias, &alias)
            && full != alias
        {
            row.split += 1;
            let same = &entities[alias]["maybeSameAs"];
            if same
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == full.as_str()))
            {
                row.linked += 1;
            }
        }
        if let Some(alias) = &alias
            && entities[alias]["type"]
                .as_str()
                .is_some_and(|t| t != c.expect)
        {
            row.mistyped += 1;
        }
    }
    let mut table = String::from("form | type | n | raw | part | split | linked | mistyped\n");
    for ((form, expect), r) in &rows {
        writeln!(
            table,
            "{form} | {expect} | {} | {} | {} | {} | {} | {}",
            r.n, r.raw, r.part, r.split, r.linked, r.mistyped
        )
        .expect("a String takes writes");
    }
    table
}

/// RFC section 9, D1: what passes raw, per form and entity type. A fix
/// changes its row here, and the snapshot records the new numbers.
#[test]
fn what_passes_raw_per_form() {
    insta::assert_snapshot!(report());
}
