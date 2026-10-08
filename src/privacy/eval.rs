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

use super::detect;
use super::detect::dict::Names;
use super::detect::model::{self, Model};
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

/// The words of a text, in byte offsets: runs of letters and digits, each
/// character of a script without spaces on its own, and each other
/// character that is not whitespace, so that two names with only
/// punctuation between them stay apart.
fn words(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut run: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c.is_alphanumeric() && !detect::unspaced(c) {
            run.get_or_insert(i);
            continue;
        }
        if let Some(start) = run.take() {
            out.push((start, i));
        }
        if !c.is_whitespace() {
            out.push((i, i + c.len_utf8()));
        }
    }
    if let Some(start) = run {
        out.push((start, s.len()));
    }
    out
}

/// A replacement the pipeline made: the alias (or `link N`), the type its
/// entity has, and the words of the input it stands for.
struct Replaced {
    label: String,
    kind: String,
    /// Word indices of the input, `lo..hi`; empty when the alias stands
    /// for nothing the alignment can see.
    lo: usize,
    hi: usize,
}

/// The replacements in `output`, found by aligning its words with the
/// `input`'s: an alias is a key of `entities` or `link N`; the input words
/// between the words matched on either side of it are what it replaced.
fn replacements(input: &str, output: &str, entities: &Map<String, Value>) -> Vec<Replaced> {
    static ALIAS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[a-z]+(?:-[a-z]+){2,}|\blink [0-9]+\b").expect("fixed"));
    let aliases: Vec<(usize, usize)> = ALIAS
        .find_iter(output)
        .filter(|m| m.as_str().starts_with("link ") || entities.contains_key(m.as_str()))
        .map(|m| (m.start(), m.end()))
        .collect();
    let plain: Vec<(usize, usize)> = words(output)
        .into_iter()
        .filter(|&(s, e)| !aliases.iter().any(|&(a, b)| a <= s && e <= b))
        .collect();
    let input_words = words(input);
    // Longest common subsequence of the words, by text.
    let (n, m) = (input_words.len(), plain.len());
    let same = |i: usize, j: usize| {
        input[input_words[i].0..input_words[i].1] == output[plain[j].0..plain[j].1]
    };
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if same(i, j) {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut matched = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(i, j) {
            matched.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let mut out = Vec::new();
    let mut taken_until = 0;
    for &(a, b) in &aliases {
        let label = output[a..b].to_string();
        let kind = if label.starts_with("link ") {
            "link".to_string()
        } else {
            entities[&label]["type"]
                .as_str()
                .unwrap_or("?")
                .to_string()
        };
        // Plain output words before this alias.
        let before = plain.iter().take_while(|&&(s, _)| s < a).count();
        let lo = matched
            .iter()
            .filter(|&&(_, oj)| oj < before)
            .map(|&(ii, _)| ii + 1)
            .max()
            .unwrap_or(0)
            .max(taken_until);
        let hi = matched
            .iter()
            .filter(|&&(_, oj)| oj >= before)
            .map(|&(ii, _)| ii)
            .min()
            .unwrap_or(n)
            .max(lo);
        taken_until = hi;
        out.push(Replaced {
            label,
            kind,
            lo,
            hi,
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
        let input_words = words(&item.text);
        let replaced = replacements(&item.text, text, entities);
        // The alias the From header's name has in this result.
        let from_alias = out["message"]["from"]
            .as_str()
            .and_then(|f| f.split_once(" <"))
            .map(|(p, _)| p.to_string());
        let mut touched = vec![false; replaced.len()];
        for p in &item.planted {
            let mine: Vec<usize> = (0..input_words.len())
                .filter(|&i| p.start <= input_words[i].0 && input_words[i].1 <= p.end)
                .collect();
            let (Some(&first), Some(&last)) = (mine.first(), mine.last()) else {
                continue;
            };
            let covering: Vec<usize> = (0..replaced.len())
                .filter(|&k| replaced[k].lo < replaced[k].hi && replaced[k].lo <= last && first < replaced[k].hi)
                .collect();
            for &k in &covering {
                touched[k] = true;
            }
            let covered = |i: usize| covering.iter().any(|&k| replaced[k].lo <= i && i < replaced[k].hi);
            let came = if covering.is_empty() {
                Came::Raw
            } else if !mine.iter().all(|&i| covered(i)) {
                Came::Part
            } else if covering.len() == 1 {
                Came::Whole
            } else {
                Came::Pieces
            };
            let row = r.forms.entry((p.form.clone(), p.expect.clone())).or_default();
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
        for (k, rep) in replaced.iter().enumerate() {
            let by_type = r.types.entry(rep.kind.clone()).or_default();
            by_type.aliases += 1;
            r.langs.entry(item.lang.clone()).or_default().aliases += 1;
            if !touched[k] {
                by_type.spurious += 1;
                r.langs.entry(item.lang.clone()).or_default().spurious += 1;
                let stood_for = if rep.lo < rep.hi {
                    &item.text[input_words[rep.lo].0..input_words[rep.hi - 1].1]
                } else {
                    ""
                };
                r.spurious
                    .push(format!("{}: {stood_for:?} as {}", item.id, rep.kind));
            }
        }
    }
    r
}

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
    insta::assert_snapshot!(text(&evaluate(Some(model))));
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

/// The alignment sees what the pipeline replaced, by word, through
/// repeated words, punctuation between names and names in pieces.
#[test]
fn the_alignment_finds_what_was_replaced() {
    let entities: Map<String, Value> = [
        ("amber-falcon-river", "person"),
        ("copper-lantern-mist", "organization"),
    ]
    .into_iter()
    .map(|(a, t)| (a.to_string(), json!({ "type": t })))
    .collect();
    let input = "Dana Ruiz met Dana at Acme / Dana Ruiz again, see https://x.example/a";
    let output =
        "amber-falcon-river met amber-falcon-river at copper-lantern-mist / amber-falcon-river again, see link 1";
    let got: Vec<(String, String, String)> = replacements(input, output, &entities)
        .iter()
        .map(|r| {
            let w = words(input);
            (
                r.label.clone(),
                r.kind.clone(),
                input[w[r.lo].0..w[r.hi - 1].1].to_string(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("amber-falcon-river", "person", "Dana Ruiz"),
            ("amber-falcon-river", "person", "Dana"),
            ("copper-lantern-mist", "organization", "Acme"),
            ("amber-falcon-river", "person", "Dana Ruiz"),
            ("link 1", "link", "https://x.example/a"),
        ]
        .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
    );
    assert_eq!(words("与王小明开会 a-b"), vec![(0, 3), (3, 6), (6, 9), (9, 12), (12, 15), (15, 18), (19, 20), (20, 21), (21, 22)]);
}
