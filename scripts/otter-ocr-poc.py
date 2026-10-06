#!/usr/bin/env python3
"""Proof of concept for RFC Q23: Otter over OCR output.

Renders each document in tests/fixtures/ocr-docs.jsonl as a degraded image
(scan, fax or phone photo), reads it back with Apple Vision and Tesseract,
and runs Otter on the raw OCR text and on a cleaned copy. Each planted name
is located in the OCR text by approximate matching, every occurrence of it;
a name OCR damaged past a third of its letters counts as unreadable, not as
found or missed. Recall is over located occurrences.

Usage: otter-ocr-poc.py MODEL_DIR VISION_BINARY OUT_DIR [--threshold 0.2]
Loads only a local directory, offline: the model's own Python code runs on
load (trust_remote_code), so it must be the copy that was read.
"""

import os

# Before transformers loads: never fetch a model or its code.
os.environ["HF_HUB_OFFLINE"] = "1"

import argparse
import json
import random
import re
import subprocess
from collections import defaultdict
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "tests" / "fixtures" / "ocr-docs.jsonl"
FONTS = [Path("/System/Library/Fonts/Supplemental"), Path("/System/Library/Fonts")]
LABELS = ["person", "organization", "project", "product", "location"]
MARK = re.compile(r"\{(\w+):(\w+)\|([^}]*)\}")


def plain(text):
    """Text without markup, and the planted names (type, form, surface)."""
    return MARK.sub(lambda m: m[3], text), [(m[1], m[2], m[3]) for m in MARK.finditer(text)]


def font(name, size):
    for d in FONTS:
        if (d / name).exists():
            return ImageFont.truetype(str(d / name), size)
    raise FileNotFoundError(name)


def render(doc, out):
    """A white page at about 200 dpi, then the document's degradation."""
    rng = random.Random(doc["id"])
    text, _ = plain(doc["text"])
    f = font(doc["font"], 30)
    page = Image.new("L", (1700, 1300), 255)
    ImageDraw.Draw(page).multiline_text((90, 90), text, font=f, fill=20, spacing=14)
    kind = doc["degrade"]
    if kind == "scan":
        page = page.rotate(rng.uniform(-0.8, 0.8), fillcolor=255, resample=Image.BICUBIC)
        page = page.filter(ImageFilter.GaussianBlur(0.7)).resize((1275, 975))
        page = page.point(lambda v: min(255, max(0, v + rng.randint(-18, 18))))
        quality = 55
    elif kind == "fax":
        page = page.rotate(rng.uniform(-1.2, 1.2), fillcolor=255, resample=Image.BICUBIC)
        page = page.resize((850, 325)).resize((850, 650))  # 200 x 100 dpi, like a fax
        page = page.point(lambda v: 0 if v < 150 else 255)
        px = page.load()
        for _ in range(2500):
            px[rng.randrange(850), rng.randrange(650)] = rng.choice((0, 255))
        quality = 60
    else:  # a phone photo: skew, uneven light, soft focus
        page = page.rotate(rng.uniform(-2.5, 2.5), fillcolor=235, resample=Image.BICUBIC)
        shade = Image.linear_gradient("L").resize(page.size).point(lambda v: v // 3)
        page = Image.composite(page, Image.new("L", page.size, 140), shade.point(lambda v: 255 - v))
        page = page.filter(ImageFilter.GaussianBlur(1.1)).resize((1190, 910))
        quality = 45
    page.convert("RGB").save(out, "JPEG", quality=quality)


def clean(text):
    """Join words hyphenated across lines, and lines into one run of text."""
    text = re.sub(r"(\w)-\n(\w)", r"\1\2", text)
    text = re.sub(r"[ \t]*\n[ \t]*", " ", text)
    return re.sub(r"[ \t]{2,}", " ", text).strip()


def locate(name, text):
    """Every approximate occurrence of `name` in `text` (Sellers' algorithm,
    case-folded), each within a third of its letters: [(start, end, edits)],
    best first among overlapping candidates."""
    # Fold per character: casefolding the whole string turns "ß" into "ss"
    # and shifts every offset after it.
    p, t = [c.casefold() for c in name], [c.casefold() for c in text]
    # A name of one or two letters must match exactly, or "TQ" would match
    # every "To" on the page.
    limit = len(p) // 3
    # Scripts that separate words: a match must start and end at a word edge.
    spaced = bool(name) and ord(name[0]) < 0x2E80

    def edge(i):
        return i <= 0 or i >= len(text) or not text[i].isalnum() or not text[i - 1].isalnum()

    prev = [(j, 0) for j in range(len(p) + 1)]  # (cost, start) per pattern prefix
    ends = []
    for i, ch in enumerate(t):
        cur = [(0, i + 1)]
        for j in range(1, len(p) + 1):
            cur.append(min(
                (prev[j - 1][0] + (p[j - 1] != ch), prev[j - 1][1] if j > 1 else i),
                (prev[j][0] + 1, prev[j][1]),
                (cur[j - 1][0] + 1, cur[j - 1][1]),
            ))
        cost, start = cur[-1]
        if len(p) and cost <= limit and (not spaced or (edge(start) and edge(i + 1))):
            ends.append((cost, start, i + 1))
        prev = cur
    taken, out = set(), []
    for cost, a, b in sorted(ends):
        if not taken.intersection(range(a, b)):
            taken.update(range(a, b))
            out.append((a, b, cost))
    return out


def gold_spans(names, text):
    """Located occurrences of the planted names, overlaps merged into the
    longer one, and which planted names OCR left unreadable."""
    spans, unreadable = [], 0
    for _, _, surface in names:
        found = locate(surface, text)
        unreadable += not found
        spans += [(a, b) for a, b, _ in found]
    spans.sort(key=lambda s: (s[0], -(s[1] - s[0])))
    merged = []
    for a, b in spans:
        if merged and a < merged[-1][1]:
            merged[-1] = (merged[-1][0], max(b, merged[-1][1]))
        else:
            merged.append((a, b))
    return merged, unreadable


def score(model, text, names, threshold):
    spans = model.predict(text, labels=LABELS, threshold=threshold)
    covered = [False] * len(text)
    for s in spans:
        for i in range(s["start"], s["end"]):
            covered[i] = True
    golds, unreadable = gold_spans(names, text)
    found = ["unreadable"] * unreadable
    for a, b in golds:
        need = [i for i in range(a, b) if not text[i].isspace()]
        if all(covered[i] for i in need):
            found.append("found")
        elif any(covered[i] for i in need):
            found.append("partly")
        else:
            found.append("missed")
    extra = [s for s in spans if not any(s["start"] < b and a < s["end"] for a, b in golds)]
    return found, golds, len(spans), len(spans) - len(extra), extra


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("model_dir")
    ap.add_argument("vision")
    ap.add_argument("out_dir")
    ap.add_argument("--threshold", type=float, default=0.2)
    args = ap.parse_args()
    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    docs = [json.loads(line) for line in DOCS.read_text(encoding="utf-8").splitlines()]
    for d in docs:
        render(d, out / f'{d["id"]}.jpg')

    vision = subprocess.run(
        [args.vision, *[str(out / f'{d["id"]}.jpg') for d in docs]],
        capture_output=True, text=True, check=True,
    ).stdout
    read = {}
    for block in vision.split("=== ")[1:]:
        path, _, body = block.partition("\n")
        read[("vision", Path(path).stem)] = body.rstrip("\n")
    for d in docs:
        if d["lang"] in ("ja", "zh"):
            continue  # Tesseract here has English data only
        read[("tesseract", d["id"])] = subprocess.run(
            ["tesseract", str(out / f'{d["id"]}.jpg'), "stdout", "-l", "eng"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    (out / "ocr.json").write_text(json.dumps({f"{e}/{i}": t for (e, i), t in read.items()}, ensure_ascii=False, indent=1))

    from transformers import AutoModel

    model = AutoModel.from_pretrained(args.model_dir, trust_remote_code=True, local_files_only=True)
    model.eval()
    totals = defaultdict(lambda: defaultdict(int))
    by_degrade = defaultdict(lambda: defaultdict(int))
    notes = []
    for d in docs:
        _, names = plain(d["text"])
        for engine in ("vision", "tesseract"):
            raw = read.get((engine, d["id"]))
            if raw is None:
                continue
            for variant, text in (("raw", raw), ("clean", clean(raw))):
                found, golds, predicted, touching, extra = score(model, text, names, args.threshold)
                key = f"{engine}/{variant}"
                for r in found:
                    totals[key][r] += 1
                    by_degrade[(key, d["degrade"])][r] += 1
                golds_found = found[found.count("unreadable"):]
                for (a, b), r in zip(golds, golds_found):
                    if r in ("missed", "partly"):
                        notes.append(f'{key} {d["id"]}: {text[a:b]!r} {r}')
                totals[key]["predicted"] += predicted
                totals[key]["touching"] += touching
                for s in extra:
                    notes.append(f'{key} {d["id"]}: extra {s["text"]!r} as {s["label"]} ({s["score"]:.2f})')

    print(f"threshold {args.threshold}")
    print("engine/text | occurrences | planted names unreadable | found | partly | missed | recall | precision")
    for key, t in sorted(totals.items()):
        readable = t["found"] + t["partly"] + t["missed"]
        print(f'{key} | {readable} | {t["unreadable"]} | {t["found"]} | {t["partly"]} | {t["missed"]} | '
              f'{t["found"] / max(readable, 1):.3f} | {t["touching"] / max(t["predicted"], 1):.3f}')
    print("\nrecall of readable names by degradation")
    for (key, kind), t in sorted(by_degrade.items()):
        readable = t["found"] + t["partly"] + t["missed"]
        print(f'{key} {kind}: {t["found"]}/{readable}, unreadable {t["unreadable"]}')
    print("\nmisses, partial finds and extra spans:", *sorted(set(notes)), sep="\n  ")


if __name__ == "__main__":
    main()
