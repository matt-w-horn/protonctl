//! The evaluation of the privacy layer (RFC section 9, defect D1; section
//! 7, Recall): aliases mode run over the labeled synthetic corpus
//! `tests/fixtures/names.jsonl`, which holds each name, organization,
//! project, product and location in the forms real results showed passing
//! raw, and texts in 28 languages, with what came back reported per form,
//! per entity type and per language. The reports are snapshots, one
//! without the model (the Phase 2 detectors, run everywhere) and one with
//! it (M5.2's exit, run where the model is installed), so a fix of a
//! defect shows as a change in its row.
//!
//! Each corpus line is `{"id", "lang", "kind", "text"}`, with `"from"`
//! naming the sender when the result's own headers hold that person.
//! A mention is marked `{type:form|surface}`, or `{type:form|surface=Full
//! Name}` when the surface is a form of a longer name. The marks are
//! stripped before the pipeline sees the text; what it rewrote is found by
//! aligning the result's words with the text's, so the measure perturbs
//! nothing.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use super::canon;
use super::detect::dict::Names;
use super::detect::model::{self, Model};
use super::ident::EntityType;
use super::key::Keys;
use super::pipeline::{Context, run};
use crate::tool::Tool;

/// One text of the corpus.
struct Item {
    id: String,
    lang: String,
    kind: String,
    from: Option<String>,
    /// The text with its marks stripped.
    text: String,
    planted: Vec<Planted>,
}

/// One marked mention, in byte offsets of `Item::text`.
struct Planted {
    expect: String,
    form: String,
    full: Option<String>,
    start: usize,
    end: usize,
}

/// `tests/fixtures/names.jsonl`, parsed.
fn corpus() -> Vec<Item> {
    static MARK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\{(\w+):([\w-]+)\|([^}=]*)(?:=([^}]*))?\}").expect("a fixed pattern")
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/names.jsonl");
    let file = std::fs::read_to_string(&path).expect("the corpus file");
    file.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: Value = serde_json::from_str(line).expect("a JSON line");
            let marked = v["text"].as_str().expect("text");
            let mut text = String::new();
            let mut planted = Vec::new();
            let mut at = 0;
            for m in MARK.captures_iter(marked) {
                let whole = m.get(0).expect("group 0");
                text.push_str(&marked[at..whole.start()]);
                let start = text.len();
                text.push_str(&m[3]);
                planted.push(Planted {
                    expect: m[1].to_string(),
                    form: m[2].to_string(),
                    full: m.get(4).map(|f| f.as_str().to_string()),
                    start,
                    end: text.len(),
                });
                at = whole.end();
            }
            text.push_str(&marked[at..]);
            Item {
                id: v["id"].as_str().expect("id").to_string(),
                lang: v["lang"].as_str().expect("lang").to_string(),
                kind: v["kind"].as_str().expect("kind").to_string(),
                from: v["from"].as_str().map(str::to_string),
                text,
                planted,
            }
        })
        .collect()
}

/// Each known person's and organization's address, for the From of a case
/// the result's own headers hold.
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

/// What the process knows (RFC Q22): every correspondent's display name,
/// with their address. Every other name in the corpus is in no header.
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
        .map(|n| (n.to_string(), Some(address(n)))),
    )
}

/// The field of a message an item's text goes in: a subject-kind text is
/// the subject, every other the body.
fn field(item: &Item) -> &'static str {
    if item.kind == "subject" {
        "subject"
    } else {
        "body"
    }
}

/// One item as a message the pipeline rewrites, from the person it names
/// when the headers hold that person.
fn message(item: &Item) -> Value {
    let from = match &item.from {
        Some(f) => format!("{f} <{}>", address(f)),
        None => "Sam Okafor <sam@okafor.example>".into(),
    };
    let (subject, body) = if field(item) == "subject" {
        (item.text.as_str(), "Notes.")
    } else {
        ("Notes", item.text.as_str())
    };
    json!({ "message": {
        "from": from,
        "to": ["Sam Okafor <sam@okafor.example>"],
        "subject": subject,
        "body": body,
    } })
}

/// A replacement the pipeline made: the alias (or `link N`, with its
/// domain's alias in brackets), the type its entity has, and the bytes of
/// the input it stands for.
struct Replaced {
    label: String,
    kind: String,
    /// Byte offsets of the input, `start..end`; empty when the alias stands
    /// for nothing the alignment can see.
    start: usize,
    end: usize,
}

/// The canonical value an entity of type `t` has for the text `s`, as the
/// pipeline's registry keys it and a `ref` holds it.
fn canonical(t: EntityType, s: &str) -> String {
    match t {
        EntityType::Person
        | EntityType::Organization
        | EntityType::Location
        | EntityType::Project
        | EntityType::Product => canon::name(s),
        EntityType::Email => canon::email(s),
        EntityType::Domain => canon::domain(s),
        _ => canon::plain(s),
    }
}

/// The replacements in `output`: an alias is a key of `entities`, or
/// `link N` with its domain's alias. The text between two aliases is the
/// input's, unchanged, so each alias starts where the text before it ends
/// in the input. It ends where the input, read from there, has its `ref`'s
/// canonical value and is followed by the text after it; when no span does
/// (a short form stands under its full name's alias, a link has no `ref`),
/// where the text after it is first found again.
fn replacements(
    input: &str,
    output: &str,
    entities: &Map<String, Value>,
    keys: &Keys,
) -> Vec<Replaced> {
    static ALIAS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\blink [0-9]+\b(?: \([a-z]+(?:-[a-z]+){2,}\))?|[a-z]+(?:-[a-z]+){2,}")
            .expect("fixed")
    });
    let aliases: Vec<(usize, usize)> = ALIAS
        .find_iter(output)
        .filter(|m| m.as_str().starts_with("link ") || entities.contains_key(m.as_str()))
        .map(|m| (m.start(), m.end()))
        .collect();
    let mut out = Vec::new();
    let mut at = 0;
    let mut from = 0;
    for (k, &(a, b)) in aliases.iter().enumerate() {
        let label = output[a..b].to_string();
        let before = &output[from..a];
        if input[at..].starts_with(before) {
            at += before.len();
        } else if let Some(i) = input[at..].find(before) {
            at += i + before.len();
        }
        from = b;
        let after = aliases
            .get(k + 1)
            .map_or(&output[b..], |&(next, _)| &output[b..next]);
        let last = k + 1 == aliases.len();
        let follows = |end: usize| {
            if last {
                input[end..] == *after
            } else {
                input[end..].starts_with(after)
            }
        };
        let (kind, value) = if label.starts_with("link ") {
            ("link".to_string(), None)
        } else {
            let e = &entities[&label];
            (
                e["type"].as_str().unwrap_or("?").to_string(),
                e["ref"].as_str().and_then(|r| keys.open_ref(r).ok()),
            )
        };
        let start = at;
        let ends = || (start + 1..=input.len()).filter(|&e| input.is_char_boundary(e));
        let end = value
            .as_ref()
            .and_then(|(t, v)| {
                ends().find(|&e| follows(e) && canonical(*t, &input[start..e]) == *v)
            })
            .or_else(|| {
                (last || !after.is_empty())
                    .then(|| ends().find(|&e| follows(e)))
                    .flatten()
            });
        let end = end.unwrap_or(start);
        at = end;
        out.push(Replaced {
            label,
            kind,
            start,
            end,
        });
    }
    out
}

/// How one planted mention came back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Came {
    /// Every word of it, under one alias.
    Whole,
    /// Every word of it under some alias, but more than one.
    Pieces,
    /// A word of it left as written.
    Part,
    /// All of it as written.
    Raw,
}

#[derive(Default)]
struct Row {
    n: usize,
    raw: usize,
    part: usize,
    pieces: usize,
    split: usize,
    linked: usize,
    mistyped: usize,
}

#[derive(Default)]
struct Tally {
    mentions: usize,
    found: usize,
    aliases: usize,
    spurious: usize,
}

#[derive(Default)]
struct Report {
    forms: BTreeMap<(String, String), Row>,
    types: BTreeMap<String, Tally>,
    langs: BTreeMap<String, Tally>,
    spurious: Vec<String>,
}

/// Run every item through the pipeline, with `model` when given.
fn evaluate(model: Option<&Model>) -> Report {
    let keys = Keys::derive(&[9; 32]);
    let names = process_names();
    let mut r = Report::default();
    for item in corpus() {
        let ctx = Context {
            tool: Tool::GetMessage,
            query: None,
            you: Some("sam@okafor.example"),
            names: &names,
            incomplete: &[],
            model,
            deadline: None,
        };
        let out = run(message(&item), &keys, &ctx).expect("the pipeline rewrites every item");
        let text = out["message"][field(&item)].as_str().unwrap_or_default();
        let empty = Map::new();
        let entities = out["entities"].as_object().unwrap_or(&empty);
        let replaced = replacements(&item.text, text, entities, &keys);
        // The alias the From header's name has in this result.
        let from_alias = out["message"]["from"]
            .as_str()
            .and_then(|f| f.split_once(" <"))
            .map(|(p, _)| p.to_string());
        let mut touched = vec![false; replaced.len()];
        for p in &item.planted {
            let covering: Vec<usize> = (0..replaced.len())
                .filter(|&k| {
                    let r = &replaced[k];
                    r.start < r.end && r.start < p.end && p.start < r.end
                })
                .collect();
            for &k in &covering {
                touched[k] = true;
            }
            let covered = |i: usize| {
                covering
                    .iter()
                    .any(|&k| replaced[k].start <= i && i < replaced[k].end)
            };
            let came = if covering.is_empty() {
                Came::Raw
            } else if !(p.start..p.end).all(covered) {
                Came::Part
            } else if covering.len() == 1 {
                Came::Whole
            } else {
                Came::Pieces
            };
            let row = r
                .forms
                .entry((p.form.clone(), p.expect.clone()))
                .or_default();
            row.n += 1;
            let by_type = r.types.entry(p.expect.clone()).or_default();
            let by_lang = r.langs.entry(item.lang.clone()).or_default();
            by_type.mentions += 1;
            by_lang.mentions += 1;
            match came {
                Came::Raw => row.raw += 1,
                Came::Part => row.part += 1,
                Came::Pieces => row.pieces += 1,
                Came::Whole => {}
            }
            if matches!(came, Came::Whole | Came::Pieces) {
                by_type.found += 1;
                by_lang.found += 1;
            }
            let Some(&k) = covering.first() else { continue };
            let first_alias = &replaced[k];
            if first_alias.kind != p.expect {
                row.mistyped += 1;
            }
            if came == Came::Whole
                && item.from.is_some()
                && p.full.as_deref() == item.from.as_deref()
                && let Some(full) = &from_alias
                && full != &first_alias.label
            {
                row.split += 1;
                let same = &entities[&first_alias.label]["maybeSameAs"];
                if same
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == full.as_str()))
                {
                    row.linked += 1;
                }
            }
        }
        r.count_aliases(&item, &replaced, &touched);
    }
    r
}

impl Report {
    /// Every alias in one item's result counts for its type and language;
    /// one that touched no planted mention is spurious, listed with the
    /// text it stood for.
    fn count_aliases(&mut self, item: &Item, replaced: &[Replaced], touched: &[bool]) {
        for (rep, &touched) in replaced.iter().zip(touched) {
            let by_type = self.types.entry(rep.kind.clone()).or_default();
            by_type.aliases += 1;
            self.langs.entry(item.lang.clone()).or_default().aliases += 1;
            if !touched {
                by_type.spurious += 1;
                self.langs.entry(item.lang.clone()).or_default().spurious += 1;
                let stood_for = &item.text[rep.start..rep.end];
                self.spurious
                    .push(format!("{}: {stood_for:?} as {}", item.id, rep.kind));
            }
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "counts of a few hundred mentions"
)]
fn ratio(a: usize, b: usize) -> String {
    if b == 0 {
        "-".into()
    } else {
        format!("{:.3}", a as f64 / b as f64)
    }
}

/// The report as text: per form and type how many mentions, and of those
/// how many came back whole as written (`raw`), with a word left (`part`),
/// aliased in more than one piece (`pieces`), with an alias other than the
/// one the name's full form has in the same result (`split`), of those
/// how many name that alias in `maybeSameAs` (`linked`), and how many
/// carry another entity type (`mistyped`); then recall (`found`, whole or
/// in pieces, over `mentions`) and precision (aliases that touch a planted
/// mention over all aliases) per entity type and per language; then each
/// spurious alias with the words it replaced.
fn text(r: &Report) -> String {
    let mut t = String::from("form | type | n | raw | part | pieces | split | linked | mistyped\n");
    for ((form, expect), row) in &r.forms {
        writeln!(
            t,
            "{form} | {expect} | {} | {} | {} | {} | {} | {} | {}",
            row.n, row.raw, row.part, row.pieces, row.split, row.linked, row.mistyped
        )
        .expect("a String takes writes");
    }
    let table = |t: &mut String, head: &str, rows: &BTreeMap<String, Tally>| {
        writeln!(
            t,
            "\n{head} | mentions | found | recall | aliases | spurious | precision"
        )
        .expect("writes");
        let mut all = Tally::default();
        for (key, c) in rows {
            all.mentions += c.mentions;
            all.found += c.found;
            all.aliases += c.aliases;
            all.spurious += c.spurious;
            writeln!(
                t,
                "{key} | {} | {} | {} | {} | {} | {}",
                c.mentions,
                c.found,
                ratio(c.found, c.mentions),
                c.aliases,
                c.spurious,
                ratio(c.aliases - c.spurious, c.aliases)
            )
            .expect("writes");
        }
        writeln!(
            t,
            "all | {} | {} | {} | {} | {} | {}",
            all.mentions,
            all.found,
            ratio(all.found, all.mentions),
            all.aliases,
            all.spurious,
            ratio(all.aliases - all.spurious, all.aliases)
        )
        .expect("writes");
    };
    table(&mut t, "type", &r.types);
    table(&mut t, "language", &r.langs);
    t.push_str("\nspurious\n");
    for s in &r.spurious {
        writeln!(t, "{s}").expect("writes");
    }
    t
}

/// RFC section 9, D1: what passes raw, per form and entity type, with the
/// Phase 2 detectors alone. A fix changes its row here, and the snapshot
/// records the new numbers.
#[test]
fn what_passes_raw_per_form() {
    insta::assert_snapshot!(text(&evaluate(None)));
}

/// RFC section 7, Recall (M5.2's exit): the same, with the model, per
/// entity type and language. Checks nothing where the model is not
/// installed (CI), with a printed notice.
#[test]
fn what_passes_raw_with_the_model() {
    let Some(model) = model::tests::shared() else {
        return;
    };
    insta::assert_snapshot!(text(&evaluate(Some(&model))));
}

/// Recall and precision at each threshold, to set `model::THRESHOLD`:
/// `cargo test --release threshold_sweep -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, run by hand"]
fn threshold_sweep() {
    let Some(model) = model::tests::shared() else {
        return;
    };
    for threshold in [0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5] {
        let at = model.at_threshold(threshold);
        let r = evaluate(Some(&at));
        let (mut mentions, mut found, mut aliases, mut spurious) = (0, 0, 0, 0);
        for c in r.types.values() {
            mentions += c.mentions;
            found += c.found;
            aliases += c.aliases;
            spurious += c.spurious;
        }
        eprintln!(
            "threshold {threshold}: recall {found}/{mentions} = {}, precision {}/{aliases} = {}",
            ratio(found, mentions),
            aliases - spurious,
            ratio(aliases - spurious, aliases)
        );
        for (t, c) in &r.types {
            eprintln!(
                "  {t}: recall {}, precision {}",
                ratio(c.found, c.mentions),
                ratio(c.aliases - c.spurious, c.aliases)
            );
        }
    }
}

/// The alignment sees what the pipeline replaced, to the byte, through
/// repeated names, a short form under its full name's alias, two aliases
/// with one letter between them, a name with a suffix joined to it, as
/// Korean and Turkish write them, two names with one space between them,
/// where only the `ref` says where the first ends, and a link with its
/// domain's alias.
#[test]
fn the_alignment_finds_what_was_replaced() {
    let keys = Keys::derive(&[4; 32]);
    let entity = |t: EntityType, canonical: &str| json!({ "type": <&str>::from(t), "ref": keys.reference(t, canonical) });
    let entities: Map<String, Value> = [
        (
            "amber-falcon-river",
            entity(EntityType::Person, "dana ruiz"),
        ),
        (
            "copper-lantern-mist",
            entity(EntityType::Organization, "acme"),
        ),
        ("velvet-otter-canyon", entity(EntityType::Person, "박 팀장")),
        (
            "quiet-maple-forge",
            entity(EntityType::Project, "두루미 프로젝트"),
        ),
        ("tidy-harbor-lens", entity(EntityType::Person, "kaya")),
        (
            "bold-cinder-wharf",
            entity(EntityType::Person, "murat demir"),
        ),
        (
            "lunar-thistle-gate",
            entity(EntityType::Organization, "anadolu lojistik"),
        ),
        ("sage-pebble-arch", entity(EntityType::Domain, "x.example")),
    ]
    .into_iter()
    .map(|(a, v)| (a.to_string(), v))
    .collect();
    let input = "Dana Ruiz met Dana at ACME / Dana Ruiz again. 두루미 프로젝트는 박 팀장이 Kaya Hanım'a; Murat Demir Anadolu Lojistik ile, see https://x.example/a";
    let output = "amber-falcon-river met amber-falcon-river at copper-lantern-mist / amber-falcon-river again. quiet-maple-forge는 velvet-otter-canyon이 tidy-harbor-lens Hanım'a; bold-cinder-wharf lunar-thistle-gate ile, see link 1 (sage-pebble-arch)";
    let got: Vec<(String, String, String)> = replacements(input, output, &entities, &keys)
        .iter()
        .map(|r| {
            (
                r.label.clone(),
                r.kind.clone(),
                input[r.start..r.end].to_string(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("amber-falcon-river", "person", "Dana Ruiz"),
            ("amber-falcon-river", "person", "Dana"),
            ("copper-lantern-mist", "organization", "ACME"),
            ("amber-falcon-river", "person", "Dana Ruiz"),
            ("quiet-maple-forge", "project", "두루미 프로젝트"),
            ("velvet-otter-canyon", "person", "박 팀장"),
            ("tidy-harbor-lens", "person", "Kaya"),
            ("bold-cinder-wharf", "person", "Murat Demir"),
            ("lunar-thistle-gate", "organization", "Anadolu Lojistik"),
            ("link 1 (sage-pebble-arch)", "link", "https://x.example/a"),
        ]
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
    );
}
