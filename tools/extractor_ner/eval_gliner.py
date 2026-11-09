"""Zero-shot GLiNER-family baselines on DEV/TEST (model alone), with latency and memory, and a raw-span dump
for the Rust hybrid evaluation (`ner_eval.rs eval_hybrid_from_json`).

    python -I eval_gliner.py --model <dir or hf id> [--onnx onnx/model_int8.onnx] [--dump out.json] [--th 0.3]

Label wording is the one shipped in `crates/model-worker/src/gliner.rs` (chosen in SP2).
"""
import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import numpy as np  # noqa: E402
import psutil  # noqa: E402

import ner_common as C  # noqa: E402

LABELS = {
    "application": "app", "person or group": "contact", "file or document": "file", "folder": "folder",
    "song, artist or video creator": "media", "website": "website", "search query": "query", "message text": "message",
    "place": "place", "meeting or event": "event", "system setting": "setting",
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--onnx", default=None)
    ap.add_argument("--dump", default=None)
    ap.add_argument("--th", type=float, default=0.3)
    ap.add_argument("--threads", type=int, default=1)
    a = ap.parse_args()
    import onnxruntime as ort
    from gliner import GLiNER

    proc = psutil.Process()
    rss0 = proc.memory_info().rss
    so = ort.SessionOptions()
    so.intra_op_num_threads = a.threads
    if a.onnx:
        model = GLiNER.from_pretrained(a.model, load_onnx_model=True, load_tokenizer=True, onnx_model_file=a.onnx, session_options=so)
    else:
        model = GLiNER.from_pretrained(a.model)
    rss1 = proc.memory_info().rss
    labs = list(LABELS)

    def raw(text):
        return model.predict_entities(text, labs, threshold=a.th)

    def predict(text):
        return [(LABELS[e["label"]], text[e["start"]:e["end"]]) for e in raw(text)]

    dev, test = C.fresh_split()
    print(C.fmt(f"DEV  {a.model.split('/')[-1]} (zero-shot, alone)", C.score(dev, predict)))
    print(C.fmt(f"TEST {a.model.split('/')[-1]} (zero-shot, alone)", C.score(test, predict)))
    ms = []
    for _, t, _ in dev * 2:
        t0 = time.perf_counter()
        raw(t)
        ms.append((time.perf_counter() - t0) * 1000)
    print(f"latency {np.mean(ms):.1f} ms mean, {np.percentile(ms, 95):.1f} ms p95 ({a.threads} thread)")
    print(f"memory: +load {(rss1 - rss0) / 1e6:.0f} MB, +after requests {(proc.memory_info().rss - rss0) / 1e6:.0f} MB (includes torch/transformers imports)")
    if a.dump:
        out = {}
        for _, text, _ in C.load_rows(C.FRESH):
            out[text] = [{"kind": LABELS[e["label"]], "start": len(text[: e["start"]].encode()), "end": len(text[: e["end"]].encode()),
                          "text": text[e["start"]:e["end"]], "score": float(e["score"])} for e in raw(text)]
        Path(a.dump).write_text(json.dumps(out), encoding="utf-8")


if __name__ == "__main__":
    main()
