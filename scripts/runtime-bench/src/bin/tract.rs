//! tract, pure Rust, on the TorchScript export, with each case's shapes fixed
//! before the graph is optimized.
use tract_onnx::prelude::*;

fn main() -> anyhow::Result<()> {
    let threads = runtime_bench::threads();
    if threads > 1 {
        tract_linalg::multithread::set_default_executor(
            tract_linalg::multithread::Executor::multithread(threads),
        );
    }
    for name in runtime_bench::CASES {
        let c = runtime_bench::case(name)?;
        let (n, s_len) = (c.tokens, c.span_len.len());
        let model = tract_onnx::onnx()
            .model_for_path(format!("{}/otter.onnx", runtime_bench::dir()))?
            .with_input_fact(0, i64::fact([1, n]).into())?
            .with_input_fact(1, i64::fact([1, n]).into())?
            .with_input_fact(2, i64::fact([s_len]).into())?
            .with_input_fact(3, i64::fact([s_len]).into())?
            .with_input_fact(4, i64::fact([s_len]).into())?
            .with_input_fact(5, i64::fact([c.label_pos.len()]).into())?
            .into_optimized()?
            .into_runnable()?;
        let inputs = || -> anyhow::Result<TVec<TValue>> {
            Ok(tvec!(
                tract_ndarray::Array2::from_shape_vec((1, n), c.input_ids.clone())?
                    .into_tensor()
                    .into(),
                tract_ndarray::Array2::from_shape_vec((1, n), c.attention_mask.clone())?
                    .into_tensor()
                    .into(),
                tract_ndarray::Array1::from_vec(c.span_start.clone())
                    .into_tensor()
                    .into(),
                tract_ndarray::Array1::from_vec(c.span_end.clone())
                    .into_tensor()
                    .into(),
                tract_ndarray::Array1::from_vec(c.span_len.clone())
                    .into_tensor()
                    .into(),
                tract_ndarray::Array1::from_vec(c.label_pos.clone())
                    .into_tensor()
                    .into(),
            ))
        };
        let got = runtime_bench::time(
            &format!("tract/{threads} threads"),
            name,
            || -> anyhow::Result<Vec<f32>> {
                let out = model.run(inputs()?)?;
                Ok(out[0]
                    .to_plain_array_view::<f32>()?
                    .iter()
                    .copied()
                    .collect())
            },
        )?;
        runtime_bench::check("tract", name, &c, &got);
    }
    Ok(())
}
