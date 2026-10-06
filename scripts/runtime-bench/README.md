# runtime-bench

The runtime comparison behind RFC-0001 Q23 ([section 10](../../docs/rfc-0001/10-open-questions.md)): for Otter's span scorer, does each Rust runtime give the logits PyTorch gives, and how fast. Not part of protonctl: it is its own Cargo workspace, outside the root build, clippy and `cargo-deny`.

1. Export the model and write the cases (Otter's checkpoint at commit `8729188` in `MODEL_DIR`; the Python environment needs torch, transformers, onnx and onnxscript):

   ```sh
   python3 scripts/otter-export.py "$MODEL_DIR" "$BENCH_DIR/otter.onnx" --cases "$BENCH_DIR"
   python3 scripts/otter-export.py "$MODEL_DIR" "$BENCH_DIR/otter-dynamo.onnx" --dynamo
   ```

2. Run each runner from this folder, with `BENCH_DIR` set and `THREADS` for `ort` and `tract` (default 4):

   ```sh
   cargo run --release --features tok --bin tok
   cargo run --release --features tract --bin tract
   cargo run --release --features ort --bin ort
   cargo run --release --features burn --bin burn -- flex
   cargo run --release --features burn --bin burn -- metal
   ```

`ort`'s default build downloads ONNX Runtime; `burn` generates about 370 KB of Rust and a 1.2 GB weights file into `target/` at build time.
