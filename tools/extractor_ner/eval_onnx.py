"""Score an exported ONNX token classifier (model alone) on DEV/TEST, measure memory and latency, and
optionally dump raw spans for the Rust hybrid evaluation (`tests/ner_eval.rs`).

    python -I eval_onnx.py --dir ../../models/ner [--dump preds.json] [--file model_int8.onnx]

Memory: working set (psutil rss) right before and right after creating the session and running 30 requests,
in a fresh process, 1 intra-op thread (as the worker runs it). Latency: per-request wall time after warm-up.
"""
import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import numpy as np  # noqa: E402
import psutil  # noqa: E402
from tokenizers import Tokenizer  # noqa: E402

import ner_common as C  # noqa: E402


def softmax(x):
    e = np.exp(x - x.max(-1, keepdims=True))
    return e / e.sum(-1, keepdims=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--file", default="model_int8.onnx")
    ap.add_argument("--dump", default=None)
    ap.add_argument("--threads", type=int, default=1)
    ap.add_argument("--max-len", type=int, default=64)
    a = ap.parse_args()
    d = Path(a.dir)
    proc = psutil.Process()
    rss0 = proc.memory_info().rss
    import onnxruntime as ort

    so = ort.SessionOptions()
    so.intra_op_num_threads = a.threads
    so.inter_op_num_threads = 1
    sess = ort.InferenceSession(str(d / a.file), so, providers=["CPUExecutionProvider"])
    tok = Tokenizer.from_file(str(d / "tokenizer.json"))
    tok.enable_truncation(max_length=a.max_len)
    names = {i.name for i in sess.get_inputs()}
    labels = json.loads((d / "labels.json").read_text())
    assert labels == C.LABELS, "label set differs from ner_common.LABELS"
    rss_load = proc.memory_info().rss

    def raw(text):
        enc = tok.encode(text)
        n = len(enc.ids)
        feed = {"input_ids": np.array([enc.ids], dtype=np.int64), "attention_mask": np.array([enc.attention_mask], dtype=np.int64)}
        if "token_type_ids" in names:
            feed["token_type_ids"] = np.zeros((1, n), dtype=np.int64)
        logits = sess.run(None, feed)[0][0]
        p = softmax(logits)
        return enc.offsets, p.argmax(-1).tolist(), p.max(-1).tolist()

    def predict(text):
        off, ids, sc = raw(text)
        return [(t, text[s:e]) for t, s, e, _ in C.decode(text, off, ids, sc)]

    dev, test = C.fresh_split()
    print(C.fmt(f"DEV  {a.file} (model alone)", C.score(dev, predict)))
    print(C.fmt(f"TEST {a.file} (model alone)", C.score(test, predict)))
    # latency on DEV sentences only (no peeking at TEST text is needed for timing)
    sents = [t for _, t, _ in dev] * 2
    for t in sents[:5]:
        raw(t)
    ms = []
    for t in sents:
        t0 = time.perf_counter()
        raw(t)
        ms.append((time.perf_counter() - t0) * 1000)
    rss_run = proc.memory_info().rss
    print(f"latency {np.mean(ms):.2f} ms mean, {np.percentile(ms, 95):.2f} ms p95 ({a.threads} thread)")
    print(f"memory: process before {rss0 / 1e6:.0f} MB, +session load {(rss_load - rss0) / 1e6:.0f} MB, +after 100 requests {(rss_run - rss0) / 1e6:.0f} MB "
          f"(python+onnxruntime baseline included in 'before' only through imports)")
    if a.dump:
        rows = C.load_rows(C.FRESH)
        out = {}
        for rid, text, _ in rows:
            off, ids, sc = raw(text)
            ents = []
            for t, s, e, score in C.decode(text, off, ids, sc):
                ents.append({"kind": t, "start": len(text[:s].encode()), "end": len(text[:e].encode()), "text": text[s:e], "score": score})
            out[text] = ents
        Path(a.dump).write_text(json.dumps(out), encoding="utf-8")
        print("dumped raw spans to", a.dump)


if __name__ == "__main__":
    main()
