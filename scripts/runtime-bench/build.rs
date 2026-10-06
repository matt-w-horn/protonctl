//! With the `burn` feature, generate Rust for Otter from the dynamo export in
//! `$BENCH_DIR/otter-dynamo.onnx` (burn-onnx's ModelGen).
fn main() {
    println!("cargo::rerun-if-env-changed=BENCH_DIR");
    #[cfg(feature = "burn")]
    {
        let dir = std::env::var("BENCH_DIR")
            .expect("BENCH_DIR names the folder otter-export.py --cases wrote");
        burn_onnx::ModelGen::new()
            .input(&format!("{dir}/otter-dynamo.onnx"))
            .out_dir("model/")
            .run_from_script();
    }
}
