//! The one model for names in free text (RFC Q23, M5.2): Otter, a span
//! scorer, as an ONNX file and a tokenizer run in this process by `tract`.
//! The model is configuration, not code: the two files, pinned by SHA-256,
//! the labels and the threshold, below. A text is cut into windows under
//! the model's token limit, each window is scored over every candidate
//! span, and the spans above the threshold come back as mentions in byte
//! offsets of the text. The detector fails closed (R13): the files must
//! match their hashes, and the model must find the names of a fixed
//! sentence before it serves anything.

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Instant;

use anyhow::{Context as _, Result, anyhow, bail};
use tokenizers::{Encoding, Tokenizer};
use tract_onnx::prelude::*;

use super::{Detector, Form, Kind, Mention};
use crate::digest::Sha256;
use crate::privacy::ident::EntityType;

/// `otter.onnx`: the span scorer `scripts/otter-export.py` writes from
/// `whoisjones/otter-cross-mmbert` at commit `8729188` (opset 17).
pub const ONNX_SHA256: &str = "461f1a71935b1ea840d668fe8f4d2bd90d9d1a265e533853d32a5961663769a0";
/// `tokenizer.json` from the same checkpoint.
pub const TOKENIZER_SHA256: &str =
    "1cd61f03e5def45b8097b3ec02a89b3f5aff7c40b44b87bf0583765e2a112d6d";
/// The entity types the model is asked for, in the prompt's order, and
/// the type each becomes.
pub const LABELS: [(&str, EntityType); 5] = [
    ("person", EntityType::Person),
    ("organization", EntityType::Organization),
    ("project", EntityType::Project),
    ("product", EntityType::Product),
    ("location", EntityType::Location),
];
/// A span is a mention when its sigmoid score is above this, set on the
/// evaluation corpus (`src/privacy/eval.rs`, `threshold_sweep`).
pub const THRESHOLD: f32 = 0.2;
/// `tract`'s threads, fixed in code (Q23: ONNX Runtime slowed past four).
/// Measured 2026-10-07 on an M2 Pro with 16 GB, a 20,040-character page of
/// 5,522 tokens in 256-token windows: 7.5 s on 5 threads and the same
/// within noise on 4, so the smaller count, which leaves a core to the
/// host.
pub const THREADS: usize = 4;
/// The model's sequence limit (`max_seq_length`), prompt and text together.
const MAX_TOKENS: usize = 1024;
/// A span is at most this many tokens (`max_span_length`).
const MAX_SPAN: usize = 30;
/// Text tokens per window, and how many each window shares with the
/// next, so a name cut by a window's edge is whole in the other. Smaller
/// windows are faster per token (the encoder's global attention grows
/// with the square of the window): the same page took 10.3 s in windows
/// of 896, 9.0 s in 512 and 7.5 s in 256, with the overlap counted; a
/// plan compiled for one fixed shape was no faster than the symbolic one
/// (648 against 662 ms on a 531-token window) and would hold a second
/// copy of the weights.
const WINDOW: usize = 256;
const OVERLAP: usize = 64;
/// The prompt's control token, whose positions the graph reads labels at.
const LABEL_TOKEN: &str = "[LABEL]";
/// Not an added token of the tokenizer, but the prompt's delimiter, so
/// text may not hold it either.
const SEP: &str = "[SEP]";
/// The sentence the detector must find names in before it serves (R13).
const SELF_TEST: &str = "Hi Dana, I spoke with Kenji Watanabe about the Falcon rewrite at Acme.";
const SELF_TEST_NAMES: [(&str, EntityType); 2] = [
    ("Kenji Watanabe", EntityType::Person),
    ("Acme", EntityType::Organization),
];

/// The folder that holds `otter.onnx` and `tokenizer.json`:
/// `~/Library/Application Support/protonctl/model` on macOS,
/// `$XDG_DATA_HOME/protonctl/model` on Linux (default
/// `~/.local/share/protonctl/model`).
pub fn dir() -> PathBuf {
    crate::platform::data_dir().join("model")
}

/// Why the model could not be loaded. `doctor` prints it; a tool call
/// gets a fixed fault instead (R13).
#[derive(Debug)]
pub enum LoadError {
    /// A file is missing from the folder.
    NotInstalled(PathBuf),
    /// A file is there but is not the one pinned.
    Hash(PathBuf),
    /// The files match but do not load or run.
    Broken(anyhow::Error),
    /// The model runs but did not find the fixed sentence's names.
    SelfTest(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled(dir) => write!(
                f,
                "the name model is not installed: {} must hold otter.onnx and tokenizer.json",
                dir.display()
            ),
            Self::Hash(path) => write!(
                f,
                "the name model is not installed: {} does not match its pinned SHA-256",
                path.display()
            ),
            Self::Broken(e) => write!(f, "the name model does not load: {e:#}"),
            Self::SelfTest(found) => write!(
                f,
                "the name model does not find the names of its test sentence (found: {found}), so it will not serve"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why `find` gave up: the call's deadline passed between windows, or a
/// window could not be tokenized or scored, which fails the call (R13)
/// rather than leave its names raw.
#[derive(Debug)]
pub enum Halt {
    Deadline,
    Failed(anyhow::Error),
}

/// A token index as the graph takes it.
fn index(i: usize) -> i64 {
    i64::try_from(i).expect("a token index is under 1,024")
}

/// The loaded model: `tract`'s plan, the tokenizer, and what both need.
pub struct Model {
    plan: std::sync::Arc<TypedRunnableModel>,
    tokenizer: Tokenizer,
    /// The prompt the model was trained with, before the text.
    prefix: String,
    label_id: u32,
    /// Every special token of the tokenizer, and `[SEP]`: replaced in text
    /// before tokenizing, so text never adds to the prompt.
    specials: Vec<String>,
    threshold: f32,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("threshold", &self.threshold)
            .finish_non_exhaustive()
    }
}

/// `tract`'s thread pool, set once per process.
fn set_threads() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        tract_linalg::multithread::set_default_executor(
            tract_linalg::multithread::Executor::multithread(THREADS),
        );
    });
}

/// `path`'s bytes, if its SHA-256 is `pinned`.
fn pinned_bytes(path: &Path, pinned: &str) -> Result<Vec<u8>, LoadError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let dir = path.parent().unwrap_or(path).to_path_buf();
            return Err(LoadError::NotInstalled(dir));
        }
        Err(e) => {
            return Err(LoadError::Broken(
                anyhow!(e).context(format!("cannot read {}", path.display())),
            ));
        }
    };
    let want: Sha256 = pinned.parse().expect("a pinned SHA-256 is 64 hex digits");
    if Sha256::of(&bytes) != want {
        return Err(LoadError::Hash(path.to_path_buf()));
    }
    Ok(bytes)
}

impl Model {
    /// Load from `dir()`, check both files' hashes, and run the test
    /// sentence.
    pub fn load() -> Result<Self, LoadError> {
        Self::load_from(&dir())
    }

    pub fn load_from(dir: &Path) -> Result<Self, LoadError> {
        let onnx = pinned_bytes(&dir.join("otter.onnx"), ONNX_SHA256)?;
        let tok = pinned_bytes(&dir.join("tokenizer.json"), TOKENIZER_SHA256)?;
        let model = Self::from_bytes(&onnx, &tok).map_err(LoadError::Broken)?;
        drop((onnx, tok));
        let mut found = Vec::new();
        model.find(SELF_TEST, None, &mut found).map_err(|h| match h {
            Halt::Deadline => LoadError::Broken(anyhow!("the test sentence was cut off")),
            Halt::Failed(e) => LoadError::Broken(e),
        })?;
        let names: Vec<(&str, EntityType)> = found
            .iter()
            .filter_map(|m| match m.kind {
                Kind::Entity(t) => Some((m.value.as_str(), t)),
                _ => None,
            })
            .collect();
        if SELF_TEST_NAMES.iter().all(|want| names.contains(want)) {
            Ok(model)
        } else {
            let found = names
                .iter()
                .map(|(v, t)| format!("{v:?} as {}", <&str>::from(*t)))
                .collect::<Vec<_>>()
                .join(", ");
            Err(LoadError::SelfTest(found))
        }
    }

    /// The same model at another threshold, for the threshold sweep.
    #[cfg(test)]
    pub fn at_threshold(&self, threshold: f32) -> Self {
        Self {
            plan: std::sync::Arc::clone(&self.plan),
            tokenizer: self.tokenizer.clone(),
            prefix: self.prefix.clone(),
            label_id: self.label_id,
            specials: self.specials.clone(),
            threshold,
        }
    }

    /// The model from the two files' bytes, with no hash check and no test
    /// sentence; `load_from` does both.
    fn from_bytes(onnx: &[u8], tokenizer: &[u8]) -> Result<Self> {
        set_threads();
        let tokenizer = Tokenizer::from_bytes(tokenizer)
            .map_err(|e| anyhow!("{e}"))
            .context("tokenizer.json")?;
        let label_id = tokenizer
            .token_to_id(LABEL_TOKEN)
            .context("the tokenizer has no [LABEL] token")?;
        let mut specials: Vec<String> = tokenizer
            .get_added_tokens_decoder()
            .values()
            .filter(|t| t.special)
            .map(|t| t.content.clone())
            .chain(std::iter::once(SEP.to_string()))
            .collect();
        // Longest first, so a token that holds another is replaced whole.
        specials.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        specials.dedup();
        let plan = tract_onnx::onnx()
            .model_for_read(&mut &onnx[..])
            .context("otter.onnx")?
            .into_optimized()
            .context("otter.onnx cannot be optimized")?
            .into_runnable()
            .context("otter.onnx cannot be planned")?;
        let labels: Vec<&str> = LABELS.iter().map(|(name, _)| *name).collect();
        Ok(Self {
            plan,
            tokenizer,
            prefix: prompt(&labels),
            label_id,
            specials,
            threshold: THRESHOLD,
        })
    }

    /// Every mention the model finds in `text`, in byte offsets of `text`,
    /// added to `out`. Stops between windows once `deadline` has passed.
    pub fn find(
        &self,
        text: &str,
        deadline: Option<Instant>,
        out: &mut Vec<Mention>,
    ) -> Result<(), Halt> {
        if text.trim().is_empty() {
            return Ok(());
        }
        let escaped = escape(text, &self.specials);
        let whole = self
            .tokenizer
            .encode(escaped.as_str(), false)
            .map_err(|e| Halt::Failed(anyhow!("{e}")))?;
        for w in windows(whole.get_offsets(), escaped.len()) {
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Err(Halt::Deadline);
            }
            let lo = escaped.floor_char_boundary(w.lo);
            let hi = escaped.ceil_char_boundary(w.hi);
            let mut found = Vec::new();
            self.window(&escaped[lo..hi], &mut found)
                .map_err(Halt::Failed)?;
            out.extend(found.into_iter().filter_map(|(start, end, t)| {
                let (start, end) = (start + lo, end + lo);
                (w.core_lo <= start && start < w.core_hi).then(|| Mention {
                    start,
                    end,
                    kind: Kind::Entity(t),
                    value: text[start..end].to_string(),
                    source: Detector::Model,
                    form: Form::Whole,
                })
            }));
        }
        Ok(())
    }

    /// One window: byte spans of `window` and their types.
    fn window(&self, window: &str, out: &mut Vec<(usize, usize, EntityType)>) -> Result<()> {
        let input = format!("{}{window}", self.prefix);
        let enc = self
            .tokenizer
            .encode(input.as_str(), true)
            .map_err(|e| anyhow!("{e}"))?;
        let n = enc.len();
        if n > MAX_TOKENS {
            bail!("a window of {n} tokens is over the model's {MAX_TOKENS}");
        }
        let ids = enc.get_ids();
        let offsets = enc.get_offsets();
        let label_pos: Vec<i64> = ids
            .iter()
            .enumerate()
            .filter(|&(_, &id)| id == self.label_id)
            .map(|(i, _)| index(i))
            .collect();
        if label_pos.len() != LABELS.len() {
            bail!("the prompt holds {} labels, not {}", label_pos.len(), LABELS.len());
        }
        let Some(first) = offsets.iter().position(|o| o.1 > self.prefix.len()) else {
            return Ok(());
        };
        let last = last_text_token(&enc);
        let spans = spans(first, last, n);
        if spans.is_empty() {
            return Ok(());
        }
        let logits = self.score(ids, &spans, &label_pos)?;
        for (label, span, _) in decode(&logits, &spans, self.threshold) {
            let (s, e) = spans[span];
            if let Some((start, end)) = byte_span(window, offsets, s, e, self.prefix.len()) {
                out.push((start, end, LABELS[label].1));
            }
        }
        Ok(())
    }

    /// The graph's logits, `LABELS.len()` rows of one per span.
    fn score(&self, ids: &[u32], spans: &[(usize, usize)], label_pos: &[i64]) -> Result<Vec<Vec<f32>>> {
        let n = ids.len();
        let input_ids: Vec<i64> = ids.iter().map(|&i| i64::from(i)).collect();
        let mask = vec![1i64; n];
        let as_i64 = |v: Vec<usize>| -> Vec<i64> { v.into_iter().map(index).collect() };
        let starts = as_i64(spans.iter().map(|s| s.0).collect());
        let ends = as_i64(spans.iter().map(|s| s.1).collect());
        let lens = as_i64(spans.iter().map(|s| s.1 - s.0 + 1).collect());
        let tensor2 = |v: Vec<i64>| -> Result<TValue> {
            Ok(tract_ndarray::Array2::from_shape_vec((1, n), v)?
                .into_tensor()
                .into())
        };
        let tensor1 = |v: Vec<i64>| -> TValue { tract_ndarray::Array1::from_vec(v).into_tensor().into() };
        let out = self.plan.run(tvec!(
            tensor2(input_ids)?,
            tensor2(mask)?,
            tensor1(starts),
            tensor1(ends),
            tensor1(lens),
            tensor1(label_pos.to_vec()),
        ))?;
        let view = out[0].to_plain_array_view::<f32>()?;
        let shape = view.shape();
        if shape != [LABELS.len(), spans.len()] {
            bail!("the graph gave logits of shape {shape:?}");
        }
        Ok(view
            .outer_iter()
            .map(|row| row.iter().copied().collect())
            .collect())
    }
}

/// The label prefix Otter was trained with: `[LABEL] a [LABEL] b [SEP] `.
fn prompt(labels: &[&str]) -> String {
    format!("[LABEL] {} [SEP] ", labels.join(" [LABEL] "))
}

/// `text` with every special token replaced by the same number of `*`,
/// so byte offsets hold and no text reads as part of the prompt.
fn escape(text: &str, specials: &[String]) -> String {
    let mut out = text.to_string();
    for s in specials {
        if out.contains(s.as_str()) {
            out = out.replace(s.as_str(), &"*".repeat(s.len()));
        }
    }
    out
}

/// The index of the last text token: the one before the closing `<eos>`.
fn last_text_token(enc: &Encoding) -> usize {
    enc.get_sequence_ids()
        .iter()
        .rposition(Option::is_some)
        .unwrap_or(0)
}

/// Every candidate span, as Otter lists them: from each text token, up to
/// `MAX_SPAN` long, ending at or before `last`, in `(start, end)` token
/// indices of the whole input.
fn spans(first: usize, last: usize, n: usize) -> Vec<(usize, usize)> {
    let count = n.saturating_sub(first);
    (0..count)
        .flat_map(|i| (0..MAX_SPAN).map(move |j| (i, i + j)))
        .filter(|&(_, e)| e < count && e + first <= last)
        .map(|(i, e)| (i + first, e + first))
        .collect()
}

/// Spans above `threshold`, best first, none sharing a token with a better
/// one: `(label, span index, score)`.
fn decode(logits: &[Vec<f32>], spans: &[(usize, usize)], threshold: f32) -> Vec<(usize, usize, f32)> {
    let sigmoid = |x: f32| 1.0 / (1.0 + (-x).exp());
    let mut above: Vec<(usize, usize, f32)> = logits
        .iter()
        .enumerate()
        .flat_map(|(label, row)| {
            row.iter()
                .enumerate()
                .map(move |(span, &x)| (label, span, sigmoid(x)))
        })
        .filter(|&(_, _, p)| p > threshold)
        .collect();
    above.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0)));
    let mut used: Vec<bool> = Vec::new();
    let mut kept = Vec::new();
    for (label, span, p) in above {
        let (s, e) = spans[span];
        if used.len() <= e {
            used.resize(e + 1, false);
        }
        if used[s..=e].iter().any(|&u| u) {
            continue;
        }
        used[s..=e].fill(true);
        kept.push((label, span, p));
    }
    kept
}

/// Token span `s..=e` as bytes of `window`, as Otter's `_char_span` does
/// it: `shift` (the prefix) taken off, clamped at 0, whitespace trimmed,
/// and quotes and brackets around the name left out. `None` when nothing
/// is left.
fn byte_span(
    window: &str,
    offsets: &[(usize, usize)],
    s: usize,
    e: usize,
    shift: usize,
) -> Option<(usize, usize)> {
    let start = offsets.get(s)?.0.saturating_sub(shift);
    let end = offsets.get(e)?.1.saturating_sub(shift).max(start);
    let start = window.floor_char_boundary(start.min(window.len()));
    let end = window.ceil_char_boundary(end.min(window.len()));
    let around = |c: char| c.is_whitespace() || "\"'«»“”‘’‹›()[]{},;:!?".contains(c);
    let inner = window[start..end].trim_matches(around);
    if inner.is_empty() || !inner.contains(char::is_alphanumeric) {
        return None;
    }
    let lead = window[start..end].len() - window[start..end].trim_start_matches(around).len();
    let start = start + lead;
    Some((start, start + inner.len()))
}

/// One window of a text: the bytes it covers, and the bytes whose
/// mentions it reports (`core`), which stop short of the overlap it shares
/// with its neighbours, each of which reports that part instead.
#[derive(Debug, PartialEq, Eq)]
struct Window {
    lo: usize,
    hi: usize,
    core_lo: usize,
    core_hi: usize,
}

/// The windows of a text tokenized whole, from its tokens' byte offsets:
/// `WINDOW` tokens each, every one but the last starting `OVERLAP` tokens
/// before the previous one ends.
fn windows(offsets: &[(usize, usize)], len: usize) -> Vec<Window> {
    let n = offsets.len();
    if n == 0 {
        return Vec::new();
    }
    let stride = WINDOW - OVERLAP;
    let mut out = Vec::new();
    let mut a = 0;
    loop {
        let b = (a + WINDOW).min(n);
        let core_lo = if a == 0 { 0 } else { offsets[a + OVERLAP / 2].0 };
        let core_hi = if b == n {
            len + 1
        } else {
            offsets[b - OVERLAP / 2].0
        };
        out.push(Window {
            lo: if a == 0 { 0 } else { offsets[a].0 },
            hi: if b == n { len } else { offsets[b - 1].1 },
            core_lo,
            core_hi,
        });
        if b == n {
            return out;
        }
        a += stride;
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::OnceLock;

    /// The model, loaded once for the whole test run, or `None` with a
    /// printed notice when its folder is absent (CI has no model). A folder
    /// that is there but does not load is a failure, not a skip.
    pub fn shared() -> Option<&'static Model> {
        static MODEL: OnceLock<Option<Model>> = OnceLock::new();
        MODEL
            .get_or_init(|| {
                let dir = dir();
                if !dir.is_dir() {
                    eprintln!(
                        "notice: no name model at {}; the model tests check nothing",
                        dir.display()
                    );
                    return None;
                }
                Some(Model::load().expect("the installed model loads"))
            })
            .as_ref()
    }

    fn found(model: &Model, text: &str) -> Vec<(String, EntityType)> {
        let mut out = Vec::new();
        model.find(text, None, &mut out).unwrap();
        out.into_iter()
            .filter_map(|m| match m.kind {
                Kind::Entity(t) => Some((m.value, t)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_prompt_is_otter_s() {
        assert_eq!(
            prompt(&["person", "location"]),
            "[LABEL] person [LABEL] location [SEP] "
        );
    }

    /// M5.2: a special token in text is replaced by as many `*`, so the
    /// bytes around it keep their offsets and the prompt gains nothing.
    #[test]
    fn special_tokens_are_escaped_to_the_same_length() {
        let specials = ["[LABEL]".to_string(), "<bos>".to_string(), "[SEP]".to_string()];
        let text = "Dana [LABEL] person <bos> Kenji [SEP] Acme";
        let escaped = escape(text, &specials);
        assert_eq!(escaped, "Dana ******* person ***** Kenji ***** Acme");
        assert_eq!(escaped.len(), text.len());
        assert_eq!(escape("plain", &specials), "plain");
    }

    /// Otter's span listing: every span of up to `MAX_SPAN` tokens over
    /// the text tokens, none reaching the closing token.
    #[test]
    fn spans_are_listed_as_otter_lists_them() {
        // Prompt tokens 0..15, text tokens 15..=39, <eos> at 40: 25 tokens
        // give 25 * 26 / 2 spans, as the exported case does.
        let s = spans(15, 39, 41);
        assert_eq!(s.len(), 325);
        assert_eq!(s[0], (15, 15));
        assert_eq!(s[1], (15, 16));
        assert!(s.iter().all(|&(a, b)| a >= 15 && b <= 39 && b - a < MAX_SPAN));
        // Long text: 30 per start token, fewer at the end.
        let s = spans(19, 603, 605);
        assert_eq!(s.len(), 17115);
        assert_eq!(spans(5, 4, 6), Vec::new());
    }

    /// The best span wins its tokens; a weaker one that shares a token is
    /// dropped, and one that touches nothing kept.
    #[test]
    fn decoding_keeps_the_best_non_overlapping_spans() {
        let spans = [(0, 1), (1, 2), (3, 3), (0, 0)];
        let logits = vec![
            vec![2.0, 1.0, -5.0, 0.5], // person
            vec![-5.0, 3.0, 1.5, -5.0], // organization
        ];
        let kept = decode(&logits, &spans, 0.5);
        let picked: Vec<(usize, usize)> = kept.iter().map(|&(l, s, _)| (l, s)).collect();
        // organization (1,2) at 3.0 first; person (0,1) shares token 1;
        // organization (3,3); person (0,0) at 0.5 is sigmoid 0.62 > 0.5.
        assert_eq!(picked, vec![(1, 1), (1, 2), (0, 3)]);
        assert_eq!(decode(&logits, &spans, 0.99), Vec::new());
    }

    /// A token span maps to the bytes of the window, with the prompt taken
    /// off, the leading space of the first token dropped, and quotes
    /// around a name left out.
    #[test]
    fn token_spans_map_to_trimmed_bytes() {
        let window = "Hi «Dana», ok";
        // Offsets as if a 10-byte prefix stood before the window; the first
        // token folds the space before it, as the tokenizer does.
        let offsets = [(0, 0), (0, 10), (9, 12), (12, 15), (15, 19), (19, 21), (21, 23), (0, 0)];
        assert_eq!(byte_span(window, &offsets, 2, 2, 10), Some((0, 2)));
        // "«Dana»" is bytes 3..11; the quotes go.
        assert_eq!(byte_span(window, &offsets, 3, 5, 10), Some((5, 9)));
        assert_eq!(&window[5..9], "Dana");
        assert_eq!(byte_span(window, &offsets, 6, 6, 10), None, "a comma alone");
        assert_eq!(byte_span(window, &offsets, 9, 9, 10), None, "no such token");
    }

    /// Windows of `WINDOW` tokens, each after the first starting `OVERLAP`
    /// tokens back, with cores that meet and cover the whole text.
    #[test]
    fn windows_overlap_and_their_cores_tile_the_text() {
        let offsets: Vec<(usize, usize)> = (0..2000).map(|i| (i * 2, i * 2 + 2)).collect();
        let w = windows(&offsets, 4000);
        assert_eq!(w.len(), 1 + (2000 - WINDOW).div_ceil(WINDOW - OVERLAP));
        assert_eq!(w[0].lo, 0);
        assert_eq!(w[0].hi, offsets[WINDOW - 1].1);
        assert_eq!(w[1].lo, offsets[WINDOW - OVERLAP].0);
        let last = w.last().unwrap();
        assert_eq!(last.hi, 4000);
        for pair in w.windows(2) {
            assert!(pair[0].hi > pair[1].lo, "the windows overlap");
            assert_eq!(pair[0].core_hi, pair[1].core_lo, "the cores meet");
            assert_eq!(pair[1].lo - pair[0].lo, 2 * (WINDOW - OVERLAP), "one stride apart");
        }
        assert_eq!(w[0].core_lo, 0);
        assert_eq!(last.core_hi, 4001, "the last core takes the end");
        assert_eq!(windows(&offsets[..10], 20).len(), 1);
        assert_eq!(windows(&[], 0), Vec::new());
    }

    /// The pinned hashes are checked: a folder with the wrong bytes or a
    /// missing file refuses to load, naming the folder.
    #[test]
    fn a_missing_or_altered_file_refuses_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let e = Model::load_from(dir.path()).unwrap_err();
        assert!(matches!(&e, LoadError::NotInstalled(d) if d == dir.path()), "{e:?}");
        assert!(e.to_string().contains("not installed"), "{e}");
        std::fs::write(dir.path().join("otter.onnx"), b"not the model").unwrap();
        std::fs::write(dir.path().join("tokenizer.json"), b"{}").unwrap();
        let e = Model::load_from(dir.path()).unwrap_err();
        assert!(matches!(&e, LoadError::Hash(p) if p.ends_with("otter.onnx")), "{e:?}");
        assert!(e.to_string().contains("SHA-256"), "{e}");
    }

    /// Q23: the runtime gives `PyTorch`'s logits for the exported case, and
    /// the same decisions at the threshold.
    #[test]
    fn the_runtime_gives_pytorch_s_logits() {
        #[derive(serde::Deserialize)]
        struct Case {
            input_ids: Vec<u32>,
            span_start: Vec<usize>,
            span_end: Vec<usize>,
            label_pos: Vec<i64>,
            logits: Vec<f32>,
            logits_shape: Vec<usize>,
        }
        let Some(model) = shared() else { return };
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/otter-case.json");
        let c: Case = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let spans: Vec<(usize, usize)> = c.span_start.iter().copied().zip(c.span_end.iter().copied()).collect();
        let got = model.score(&c.input_ids, &spans, &c.label_pos).unwrap();
        assert_eq!(c.logits_shape, [got.len(), got[0].len()]);
        let flat: Vec<f32> = got.into_iter().flatten().collect();
        let max = c
            .logits
            .iter()
            .zip(&flat)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max < 1e-3, "largest difference {max}");
        let sig = |x: f32| 1.0 / (1.0 + (-x).exp());
        assert!(
            c.logits
                .iter()
                .zip(&flat)
                .all(|(a, b)| (sig(*a) > THRESHOLD) == (sig(*b) > THRESHOLD)),
            "a decision differs"
        );
    }

    /// Q23: the tokenizer gives Python's token IDs and offsets for every
    /// corpus text of the proof of concept.
    #[test]
    fn the_tokenizer_gives_python_s_ids_and_offsets() {
        #[derive(serde::Deserialize)]
        struct Ref {
            text: String,
            ids: Vec<u32>,
            offsets: Vec<(usize, usize)>,
        }
        let Some(model) = shared() else { return };
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/otter-tokens.json");
        let refs: Vec<Ref> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(refs.len() >= 50);
        for r in &refs {
            let e = model.tokenizer.encode(r.text.as_str(), true).unwrap();
            assert_eq!(e.get_ids(), r.ids.as_slice(), "{}", r.text);
            // Python counts offsets in characters, Rust in bytes.
            let chars: Vec<(usize, usize)> = e
                .get_offsets()
                .iter()
                .map(|&(a, b)| (r.text[..a].chars().count(), r.text[..b].chars().count()))
                .collect();
            assert_eq!(chars, r.offsets, "{}", r.text);
        }
    }

    /// M5.2: names stand in text beside each special token of the
    /// tokenizer, `[LABEL]`, `<bos>` and the rest, and are still found.
    #[test]
    fn names_around_each_special_token_are_found() {
        let Some(model) = shared() else { return };
        assert!(model.specials.iter().any(|s| s == "[LABEL]"));
        assert!(model.specials.iter().any(|s| s == "<bos>"));
        assert!(model.specials.iter().any(|s| s == "[SEP]"));
        for special in &model.specials {
            let text = format!("Hi Dana, {special} person {special} I spoke with Kenji Watanabe about Acme.");
            let names = found(model, &text);
            assert!(
                names.contains(&("Kenji Watanabe".to_string(), EntityType::Person))
                    && names.contains(&("Acme".to_string(), EntityType::Organization)),
                "{special}: {names:?}"
            );
            assert!(
                names.iter().all(|(v, _)| !v.contains(special.as_str())),
                "{special} inside a name: {names:?}"
            );
        }
    }

    /// M5.2: a text over one window is cut into overlapping windows, a
    /// name on the cut is still found, once, and every mention's offsets
    /// are those of the original text.
    #[test]
    fn a_name_on_a_window_cut_is_found_once_with_its_offsets() {
        let Some(model) = shared() else { return };
        let filler = "The quarterly figures were discussed at length and nothing was decided. ";
        let mut text = String::new();
        let mut planted = Vec::new();
        for i in 0..150 {
            if i % 7 == 3 {
                planted.push(text.len());
                text.push_str("Then Kenji Watanabe asked about Acme. ");
            }
            text.push_str(filler);
        }
        let enc = model.tokenizer.encode(text.as_str(), false).unwrap();
        assert!(enc.len() > WINDOW, "{} tokens", enc.len());
        let mut out = Vec::new();
        model.find(&text, None, &mut out).unwrap();
        let kenji: Vec<usize> = out
            .iter()
            .filter(|m| m.value == "Kenji Watanabe")
            .map(|m| m.start)
            .collect();
        for at in &planted {
            assert!(kenji.contains(&(at + 5)), "the name planted at {at} was not found: {kenji:?}");
        }
        assert_eq!(kenji.len(), planted.len(), "each name found once");
        for m in &out {
            assert_eq!(&text[m.start..m.end], m.value, "offsets are the text's");
        }
    }

    /// M5.2: inference stops between windows once the deadline has passed.
    #[test]
    fn inference_stops_at_the_deadline() {
        let Some(model) = shared() else { return };
        // A deadline taken now has passed by the time the first window
        // checks it.
        let past = Some(Instant::now());
        let mut out = Vec::new();
        assert!(matches!(
            model.find(SELF_TEST, past, &mut out),
            Err(Halt::Deadline)
        ));
        assert_eq!(out, Vec::new());
        let future = Some(Instant::now() + std::time::Duration::from_secs(60));
        assert!(model.find(SELF_TEST, future, &mut out).is_ok());
        assert_ne!(out, Vec::new());
    }

    /// Time per window at 4 and 5 threads, and the RSS with the model
    /// loaded: `cargo test --release model::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand in release mode"]
    fn measure() {
        let Some(model) = shared() else { return };
        let mut page = String::new();
        let mut i = 0;
        while page.chars().count() < 20_000 {
            page.push_str(match i % 4 {
                0 => "Dear team, the review with Priya Raman and Tomás Herrera at Globex Corporation moved to Thursday. ",
                1 => "Die Rechnung von Brandt & Söhne GmbH für das Projekt Kranich liegt bei; Lena Hoffmann prüft sie. ",
                2 => "Nothing else changed: the smoke detectors were tested and the caulk clear is on order. ",
                _ => "請與王小明確認華信科技的合同，李靜已把青鳥項目的預算發給陳建國了。",
            });
            i += 1;
        }
        let tokens = model.tokenizer.encode(page.as_str(), false).unwrap().len();
        let t = Instant::now();
        let mut out = Vec::new();
        model.find(&page, None, &mut out).unwrap();
        let took = t.elapsed();
        let t = Instant::now();
        for _ in 0..10 {
            let mut out = Vec::new();
            model.find(SELF_TEST, None, &mut out).unwrap();
        }
        eprintln!("measure: a sentence of 44 tokens takes {:.0} ms", t.elapsed().as_secs_f64() * 100.0);
        let rss = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        eprintln!(
            "measure: {} characters, {tokens} tokens, {} mentions, {:.2} s on {THREADS} threads, windows of {WINDOW}; RSS {rss} KB",
            page.chars().count(),
            out.len(),
            took.as_secs_f64()
        );
    }
}
