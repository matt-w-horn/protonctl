#!/usr/bin/env python3
"""Proof of concept for RFC Q23: Otter over the labeled names corpus.

Reads tests/fixtures/names.jsonl, where each mention is marked
{type:form|surface}, runs Otter on the plain text, and reports per form
and per language how many mentions were covered (every character of the
mention inside a predicted span, of any label) or partly covered, and how
many predicted spans touch no mention. Prints timing and peak memory.

Usage: otter-poc.py MODEL_DIR [--threshold 0.3] [--labels person,organization,...]
Loads only a local directory, offline: the model's own Python code runs on
load (trust_remote_code), so it must be the copy that was read.
"""

import os

# Before transformers loads: never fetch a model or its code.
os.environ["HF_HUB_OFFLINE"] = "1"

import argparse
import json
import re
import resource
import sys
import time
from collections import defaultdict
from pathlib import Path

MARK = re.compile(r"\{(\w+):(\w+)\|([^}]*)\}")
CORPUS = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "names.jsonl"
LABELS = ["person", "organization", "project", "product", "location"]


def parse(text):
    """Plain text and gold mentions (type, form, start, end)."""
    out, gold, pos = [], [], 0
    for m in MARK.finditer(text):
        out.append(text[pos : m.start()])
        start = sum(map(len, out))
        out.append(m[3])
        gold.append((m[1], m[2], start, start + len(m[3])))
        pos = m.end()
    out.append(text[pos:])
    return "".join(out), gold


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("model_dir")
    ap.add_argument("--threshold", type=float, default=0.3)
    ap.add_argument("--labels", default=",".join(LABELS))
    args = ap.parse_args()
    labels = args.labels.split(",")

    from transformers import AutoModel

    t0 = time.perf_counter()
    model = AutoModel.from_pretrained(args.model_dir, trust_remote_code=True, local_files_only=True)
    model.eval()
    load = time.perf_counter() - t0

    items = [json.loads(line) for line in CORPUS.read_text(encoding="utf-8").splitlines()]
    rows = defaultdict(lambda: [0, 0, 0])  # n, covered, partly
    langs = defaultdict(lambda: [0, 0])
    predicted = touching = 0
    misses, spurious = [], []
    t0 = time.perf_counter()
    chars = 0
    for item in items:
        text, gold = parse(item["text"])
        chars += len(text)
        spans = model.predict(text, labels=labels, threshold=args.threshold)
        covered = [False] * len(text)
        for s in spans:
            for i in range(s["start"], s["end"]):
                covered[i] = True
        for typ, form, a, b in gold:
            inside = sum(covered[a:b])
            # Whitespace inside a name need not be covered.
            needed = sum(1 for c in text[a:b] if not c.isspace())
            got = sum(1 for i in range(a, b) if covered[i] and not text[i].isspace())
            row = rows[(typ, form)]
            row[0] += 1
            langs[item["lang"]][0] += 1
            if got == needed:
                row[1] += 1
                langs[item["lang"]][1] += 1
            else:
                if inside:
                    row[2] += 1
                misses.append(f'{item["id"]}: {text[a:b]!r} ({typ}/{form}){" partly" if inside else ""}')
        for s in spans:
            predicted += 1
            if any(s["start"] < b and a < s["end"] for _, _, a, b in gold):
                touching += 1
            else:
                spurious.append(f'{item["id"]}: {s["text"]!r} as {s["label"]} ({s["score"]:.2f})')
    run = time.perf_counter() - t0

    total = sum(r[0] for r in rows.values())
    hit = sum(r[1] for r in rows.values())
    print(f"threshold {args.threshold}, labels {labels}")
    print(f"recall {hit}/{total} = {hit / total:.3f}; precision {touching}/{predicted} = {touching / max(predicted, 1):.3f}")
    print("\ntype/form | n | covered | partly")
    for (typ, form), (n, c, p) in sorted(rows.items()):
        print(f"{typ}/{form} | {n} | {c} | {p}")
    print("\nlanguage | n | covered")
    for lang, (n, c) in sorted(langs.items()):
        print(f"{lang} | {n} | {c}")
    print("\nmissed:", *misses, sep="\n  ")
    print("\nspans touching no mention:", *spurious, sep="\n  ")
    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    rss_mb = rss / 1e6 if sys.platform == "darwin" else rss / 1e3
    print(f"\nload {load:.1f} s; {len(items)} items, {chars} characters in {run:.2f} s "
          f"({run / chars * 20000:.2f} s per 20,000 characters); peak RSS {rss_mb:.0f} MB")


if __name__ == "__main__":
    main()
