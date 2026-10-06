//! What every runner shares: the cases `scripts/otter-export.py --cases` writes
//! into `$BENCH_DIR`, the check against PyTorch's logits, and the timing.
use std::time::Instant;

pub fn dir() -> String {
    std::env::var("BENCH_DIR").expect("BENCH_DIR names the folder otter-export.py --cases wrote")
}

pub fn threads() -> usize {
    std::env::var("THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(4)
}

#[derive(serde::Deserialize)]
pub struct Case {
    pub input_ids: Vec<i64>,
    pub attention_mask: Vec<i64>,
    pub span_start: Vec<i64>,
    pub span_end: Vec<i64>,
    pub span_len: Vec<i64>,
    pub label_pos: Vec<i64>,
    pub logits: Vec<f32>,
    pub logits_shape: Vec<usize>,
    pub tokens: usize,
}

pub const CASES: [&str; 2] = ["case", "case_long"];

pub fn case(name: &str) -> anyhow::Result<Case> {
    Ok(serde_json::from_slice(&std::fs::read(format!(
        "{}/{name}.json",
        dir()
    ))?)?)
}

/// Compare with PyTorch's logits, and with its keep-or-drop decision at a
/// threshold of 0.2.
pub fn check(runtime: &str, name: &str, c: &Case, got: &[f32]) {
    assert_eq!(got.len(), c.logits.len(), "{runtime} {name}: output size");
    let sig = |x: f32| 1.0 / (1.0 + (-x).exp());
    let max = c
        .logits
        .iter()
        .zip(got)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    let same = c
        .logits
        .iter()
        .zip(got)
        .all(|(a, b)| (sig(*a) > 0.2) == (sig(*b) > 0.2));
    println!(
        "{runtime} {name} ({} tokens): max |diff| {max:.2e}, same decisions at 0.2: {same}",
        c.tokens
    );
}

/// Run once to warm up, then report the best of three.
pub fn time<T>(runtime: &str, name: &str, mut f: impl FnMut() -> T) -> T {
    let mut out = f();
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        out = f();
        best = best.min(t.elapsed().as_secs_f64());
    }
    println!("{runtime} {name}: best of 3 {:.0} ms", best * 1000.0);
    out
}
