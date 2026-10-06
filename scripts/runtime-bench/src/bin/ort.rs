//! ONNX Runtime through the ort crate, on the TorchScript export.
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;

fn main() -> anyhow::Result<()> {
    let threads = runtime_bench::threads();
    let e = |e: ort::Error<_>| anyhow::anyhow!("{e}");
    let mut s = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(e)?
        .with_intra_threads(threads)
        .map_err(e)?
        .commit_from_file(format!("{}/otter.onnx", runtime_bench::dir()))?;
    for name in runtime_bench::CASES {
        let c = runtime_bench::case(name)?;
        let (n, s_len) = (c.tokens, c.span_len.len());
        let mut run = || -> anyhow::Result<Vec<f32>> {
            let out = s.run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, n], c.input_ids.clone()))?,
                "attention_mask" => Tensor::from_array(([1usize, n], c.attention_mask.clone()))?,
                "span_start" => Tensor::from_array(([s_len], c.span_start.clone()))?,
                "span_end" => Tensor::from_array(([s_len], c.span_end.clone()))?,
                "span_len" => Tensor::from_array(([s_len], c.span_len.clone()))?,
                "label_pos" => Tensor::from_array(([c.label_pos.len()], c.label_pos.clone()))?,
            ])?;
            let (_, data) = out["logits"].try_extract_tensor::<f32>()?;
            Ok(data.to_vec())
        };
        let got = runtime_bench::time(&format!("ort/{threads} threads"), name, &mut run)?;
        runtime_bench::check("ort", name, &c, &got);
    }
    Ok(())
}
