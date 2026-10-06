#!/usr/bin/env python3
"""Proof of concept for RFC Q23: export Otter's span scorer to ONNX for tract.

The graph takes the tokens of "[LABEL] a [LABEL] b ... [SEP] <text>" and the
candidate spans as absolute token positions, and returns one logit per label
and span. Tokenizing, listing spans and decoding stay outside it, in the
caller, as Otter's own predict() does them. Uses the TorchScript exporter
(opset 17) by default: tract 0.23.8 rejects the dynamo exporter's graph on a
symbolic Range. burn 0.21 needs the dynamo exporter's graph instead
(--dynamo): from the TorchScript one it generates Rust that does not compile.
Checks the graph against predict() on one text before writing it.

--cases DIR writes what scripts/runtime-bench compares a runtime against:
two inputs with PyTorch's logits (a short text and one of about 600 tokens,
from tests/fixtures/names.jsonl), the token IDs and offsets Python gives for
every corpus text, and a copy of the tokenizer.

Usage: otter-export.py MODEL_DIR OUT.onnx [--dynamo] [--cases DIR]
Loads only a local directory, offline: the model's own Python code runs on
load (trust_remote_code), so it must be the copy that was read.
"""

import os

# Before transformers loads: never fetch a model or its code.
os.environ["HF_HUB_OFFLINE"] = "1"

import argparse
import json
import re
import shutil
from pathlib import Path

import torch
import torch.nn.functional as F
from transformers import AutoModel

LABELS = ["person", "organization", "project", "product", "location"]
TEXT = "Hi Dana, I spoke with Kenji Watanabe about the Falcon rewrite at Acme."
CORPUS = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "names.jsonl"
NAMES = ["input_ids", "attention_mask", "span_start", "span_end", "span_len", "label_pos"]


class Scorer(torch.nn.Module):
    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, input_ids, attention_mask, span_start, span_end, span_len, label_pos):
        m = self.m
        h = m.token_encoder(input_ids=input_ids, attention_mask=attention_mask).last_hidden_state[0]
        start = F.normalize(m.token_start_linear(h), dim=-1)
        end = F.normalize(m.token_end_linear(h), dim=-1)
        types = F.normalize(m.type_linear(h[label_pos]), dim=-1)
        spans = torch.cat([start[span_start], end[span_end], m.width_embedding(span_len)], dim=-1)
        spans = F.normalize(m.token_span_linear(spans), dim=-1)
        return m.span_logit_scale.exp() * (types @ spans.T)


def inputs(m, text):
    """The scorer's inputs for one text, built as Otter's predict() builds them."""
    tok = m.tokenizer
    prefix = m.build_prompt(LABELS)
    enc = tok(prefix + text, return_offsets_mapping=True, return_tensors="pt",
              truncation=True, max_length=m.config.max_seq_length)
    offsets = enc.pop("offset_mapping")[0].tolist()
    ids = enc["input_ids"]
    label = tok.convert_tokens_to_ids("[LABEL]")
    label_pos = [i for i, t in enumerate(ids[0].tolist()) if t == label]
    first = next(i for i, o in enumerate(offsets) if o[1] > len(prefix))
    last = max(i for i, s in enumerate(enc.sequence_ids(0)) if s is not None)
    n = len(offsets) - first
    spans = [(i, i + j) for i in range(n) for j in range(m.config.max_span_length)
             if i + j < n and i + j + first <= last]
    return (ids, enc["attention_mask"],
            torch.tensor([a + first for a, _ in spans]), torch.tensor([b + first for _, b in spans]),
            torch.tensor([b - a + 1 for a, b in spans]), torch.tensor(label_pos)), spans, offsets, first, len(prefix)


def write_cases(m, scorer, model_dir, out):
    out.mkdir(parents=True, exist_ok=True)
    texts = [re.sub(r"\{\w+:\w+\|([^}]*)\}", r"\1", json.loads(line)["text"])
             for line in CORPUS.read_text(encoding="utf-8").splitlines()]
    long = " ".join(texts)[:2400]
    long = long[: long.rfind(" ")]
    for name, text in (("case", texts[0]), ("case_long", long)):
        x, *_ = inputs(m, text)
        with torch.no_grad():
            logits = scorer(*x)
        case = {k: v.reshape(-1).tolist() for k, v in zip(NAMES, x)}
        case |= {"logits": logits.reshape(-1).tolist(), "logits_shape": list(logits.shape),
                 "tokens": int(x[0].shape[1])}
        (out / f"{name}.json").write_text(json.dumps(case))
    prefix = m.build_prompt(LABELS)
    tokens = []
    for text in texts:
        enc = m.tokenizer(prefix + text, return_offsets_mapping=True)
        tokens.append({"text": prefix + text, "ids": enc["input_ids"],
                       "offsets": [list(o) for o in enc["offset_mapping"]]})
    (out / "tokens.json").write_text(json.dumps(tokens, ensure_ascii=False))
    shutil.copy(Path(model_dir) / "tokenizer.json", out / "tokenizer.json")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("model_dir")
    ap.add_argument("out")
    ap.add_argument("--dynamo", action="store_true")
    ap.add_argument("--cases", type=Path)
    args = ap.parse_args()
    m = AutoModel.from_pretrained(args.model_dir, trust_remote_code=True, local_files_only=True).eval()
    scorer = Scorer(m).eval()
    x, spans, offsets, first, shift = inputs(m, TEXT)
    with torch.no_grad():
        probs = torch.sigmoid(scorer(*x))
    mine = sorted((TEXT[max(0, offsets[a + first][0] - shift):offsets[b + first][1] - shift].strip(), LABELS[c])
                  for c, k in zip(*torch.nonzero(probs > 0.2, as_tuple=True)) for a, b in [spans[k]])
    theirs = sorted((e["text"], e["label"]) for e in m.predict(TEXT, labels=LABELS, threshold=0.2))
    # predict() also drops overlapping spans; every span it keeps must be among ours.
    assert set(theirs) <= set(mine), (theirs, mine)
    if args.dynamo:
        t, s, c = (torch.export.Dim("t", max=m.config.max_seq_length), torch.export.Dim("s"),
                   torch.export.Dim("c"))
        torch.onnx.export(
            scorer, x, args.out, dynamo=True, opset_version=18, input_names=NAMES,
            output_names=["logits"], external_data=False,
            dynamic_shapes={"input_ids": {1: t}, "attention_mask": {1: t}, "span_start": {0: s},
                            "span_end": {0: s}, "span_len": {0: s}, "label_pos": {0: c}},
        )
    else:
        torch.onnx.export(
            scorer, x, args.out, dynamo=False, opset_version=17, input_names=NAMES,
            output_names=["logits"],
            dynamic_axes={"input_ids": {1: "t"}, "attention_mask": {1: "t"}, "span_start": {0: "s"},
                          "span_end": {0: "s"}, "span_len": {0: "s"}, "label_pos": {0: "c"},
                          "logits": {0: "c", 1: "s"}},
        )
    print(f"wrote {args.out}; predict() kept {theirs}")
    if args.cases:
        write_cases(m, scorer, args.model_dir, args.cases)
        print(f"wrote the comparison cases to {args.cases}")


if __name__ == "__main__":
    main()
