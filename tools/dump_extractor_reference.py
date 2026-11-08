"""Dump reference outputs of the Python GLiNER (ONNX int8) and of the SP2 rules pipeline.

Writes two JSON files next to this script:
  extractor_reference.json  raw GLiNER spans (threshold 0.3) for ~30 sentences.
                            The Rust test `gliner_matches_python` compares against it.
  extractor_parity.json     final spans of the Python rules+GLiNER pipeline (extract2.py) for
                            every sentence of doc4/val/val2/test/fresh, for the Rust `parity` test.

Usage (from the repo root, with the SP2 venv that has gliner + onnxruntime):
  spikes/sp2-extractor/.venv/Scripts/python.exe tools/dump_extractor_reference.py \
      [--spike spikes/sp2-extractor] [--model <dir with onnx/model_int8.onnx>]
"""
import json
import sys
from pathlib import Path

args = sys.argv[1:]


def opt(name, default):
    return Path(args[args.index(name) + 1]) if name in args else Path(default)


SPIKE = opt("--spike", "spikes/sp2-extractor").resolve()
MODEL = opt("--model", SPIKE / "models" / "gliner_small_onnx").resolve()
OUT = Path(__file__).parent
sys.path.insert(0, str(SPIKE))

import extract2  # noqa: E402
from extract2 import LABELS, Extractor, load  # noqa: E402
from gliner import GLiNER  # noqa: E402

model = GLiNER.from_pretrained(str(MODEL), load_onnx_model=True, load_tokenizer=True,
                               onnx_model_file="onnx/model_int8.onnx")
labels = list(LABELS["a"].values())


class Recorder:
    """Wraps the model so the raw entities of the last call can be read back."""

    def __init__(self, m):
        self.m, self.last = m, None

    def predict_entities(self, text, labs, threshold):
        self.last = self.m.predict_entities(text, labs, threshold=threshold)
        return self.last


rec = Recorder(model)
extract2._CACHE_LOADED["x/onnx"] = {}
ex = Extractor("x/onnx", LABELS["a"], 0.3)
ex.model = rec

sets = ["doc4.txt", "val.txt", "val2.txt", "test.txt", "fresh.txt"]
parity = []
raw_by_text = {}
for name in sets:
    for rid, text, gold in load(name):
        spans = ex.extract(text)
        raw_by_text[text] = [
            {"label": e["label"], "start": e["start"], "end": e["end"], "text": e["text"], "score": round(e["score"], 4)}
            for e in rec.last
        ]
        parity.append({"set": name, "id": rid, "text": text, "spans": [[k, t] for k, t in spans]})

# ~30 sentences for the model test: every 12th sentence of the pooled sets
texts = [p["text"] for p in parity]
pick = texts[::12][:30]
ref = {"labels": labels, "threshold": 0.3,
       "sentences": [{"text": t, "entities": raw_by_text[t]} for t in pick]}
(OUT / "extractor_reference.json").write_text(json.dumps(ref, indent=1, ensure_ascii=False), encoding="utf-8")
(OUT / "extractor_parity.json").write_text(json.dumps(parity, indent=0, ensure_ascii=False), encoding="utf-8")
print(f"{len(ref['sentences'])} reference sentences, {len(parity)} parity sentences")
