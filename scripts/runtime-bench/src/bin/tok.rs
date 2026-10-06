//! Does the tokenizers crate, without its HTTP features, give Python's token
//! IDs and offsets for every corpus text?
use tokenizers::Tokenizer;

#[derive(serde::Deserialize)]
struct Ref {
    text: String,
    ids: Vec<u32>,
    offsets: Vec<(usize, usize)>,
}

fn main() -> anyhow::Result<()> {
    let dir = runtime_bench::dir();
    let tok = Tokenizer::from_file(format!("{dir}/tokenizer.json")).map_err(anyhow::Error::msg)?;
    let refs: Vec<Ref> = serde_json::from_slice(&std::fs::read(format!("{dir}/tokens.json"))?)?;
    let (mut ids_ok, mut off_ok) = (0, 0);
    for r in &refs {
        let e = tok
            .encode(r.text.as_str(), true)
            .map_err(anyhow::Error::msg)?;
        let same_ids = e.get_ids() == r.ids.as_slice();
        // Python reports offsets in characters, Rust in bytes.
        let chars: Vec<(usize, usize)> = e
            .get_offsets()
            .iter()
            .map(|&(a, b)| (r.text[..a].chars().count(), r.text[..b].chars().count()))
            .collect();
        ids_ok += usize::from(same_ids);
        off_ok += usize::from(chars == r.offsets);
        if !same_ids {
            println!(
                "ids differ: {:?}",
                r.text.chars().take(80).collect::<String>()
            );
        }
    }
    println!(
        "tokenizer: ids match {ids_ok}/{n}, offsets match {off_ok}/{n}",
        n = refs.len()
    );
    Ok(())
}
