"""SP1 final run (Windows): own ONNX embedder (fp32 and int8), all sets, latency + RSS.
Sets: val/test = the original user sets (recovered from WSL), val2/test2 = fresh sets written
for this re-run (test2 scored once, cutoff picked on val+val2), doc4 = seen during tuning.
Usage: python final2.py [fp32|int8]
"""
import json, os, sys, time, pickle
import numpy as np, psutil
import onnxruntime as ort
from tokenizers import Tokenizer
from sklearn.linear_model import LogisticRegression
import run
from final import load_txt, norm, evaluate, show

M = run.ROOT / "models"
MODE = sys.argv[1] if len(sys.argv) > 1 else "fp32"


class Enc:
    def __init__(self, path, threads=4):
        so = ort.SessionOptions(); so.intra_op_num_threads = threads
        self.s = ort.InferenceSession(str(path), so, providers=["CPUExecutionProvider"])
        self.tok = Tokenizer.from_file(str(M / "tokenizer.json"))
        self.tok.enable_truncation(128); self.tok.enable_padding()
        self.names = {i.name for i in self.s.get_inputs()}

    def embed(self, texts):
        out = []
        for i in range(0, len(texts), 64):
            enc = self.tok.encode_batch(texts[i:i + 64])
            ids = np.array([e.ids for e in enc], dtype=np.int64)
            mask = np.array([e.attention_mask for e in enc], dtype=np.int64)
            feeds = {"input_ids": ids, "attention_mask": mask}
            if "token_type_ids" in self.names: feeds["token_type_ids"] = np.zeros_like(ids)
            h = self.s.run(None, feeds)[0]
            m = mask[..., None]
            v = (h * m).sum(1) / m.sum(1)
            out.append(v / np.linalg.norm(v, axis=1, keepdims=True))
        return np.vstack(out)


class R:
    def __init__(self, enc, texts, labels):
        self.enc, self.cache = enc, {}
        self.clf = LogisticRegression(C=30, max_iter=3000).fit(self.embed(texts), labels)

    def embed(self, texts):
        todo = [t for t in dict.fromkeys(texts) if t not in self.cache]
        if todo:
            for t, v in zip(todo, self.enc.embed(todo)): self.cache[t] = v
        return np.array([self.cache[t] for t in texts])

    def top(self, texts):
        p = self.clf.predict_proba(self.embed(texts)); i = p.argmax(1)
        return [(self.clf.classes_[k], row[k]) for row, k in zip(p, i)]

    predict = run.Router.predict


def main():
    path = M / "model.onnx"
    if MODE == "int8":
        q = M / "model_int8.onnx"
        if not q.exists():
            from onnxruntime.quantization import quantize_dynamic, QuantType
            quantize_dynamic(str(path), str(q), weight_type=QuantType.QInt8)
        path = q
    print(MODE, "model file MB:", round(path.stat().st_size / 1e6, 1))
    proc = psutil.Process()
    train = [json.loads(l) for l in open(run.ROOT / "data" / "train.jsonl", encoding="utf-8")]
    val, test, val2, test2 = (load_txt(f) for f in ["val.txt", "test.txt", "val2.txt", "test2.txt"])
    doc4 = run.load_tests()
    held = {norm(t) for s in (val, test, val2, test2) for t, _ in s} | {norm(t[1]) for t in doc4}
    clean = [t for t in train if norm(t["text"]) not in held]
    print("train rows", len(clean))
    enc = Enc(path)
    r = R(enc, [t["text"] for t in clean], [t["labels"][0] for t in clean])
    cuts = [0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
    tune = val + val2
    for c in cuts:
        print(f"  cutoff {c}: val+val2 exact {evaluate(r, tune, c)[0]}/{len(tune)}  "
              f"val {evaluate(r, val, c)[0]}/{len(val)} val2 {evaluate(r, val2, c)[0]}/{len(val2)}")
    conf = max((evaluate(r, tune, c)[0], c) for c in cuts)[1]
    print("cutoff picked on val+val2:", conf)
    for name, rows in [("val", val), ("val2", val2)]:
        show(name, r, rows, conf)
    for name, rows in [("TEST (orig)", test), ("TEST2 (fresh)", test2), ("doc4", doc4)]:
        show(name, r, rows, conf, detail=(name != "doc4"))
    # threshold sweep on test sets: accuracy vs OUT-fallback / wrong-confident rate
    print("\nthreshold sweep (single-task requests only): conf cutoff applies to chain splitting; for single-task we report top-1 prob stats")
    allsets = test + test2 + [(t[1], t[2]) for t in doc4]
    singles = [(t, l) for t, l in allsets if len(l) == 1]
    tops = r.top([t for t, _ in singles])
    for th in [0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]:
        acc = [(str(p) == l[0]) for (p, c), (t, l) in zip(tops, singles) if c >= th]
        n = len(singles)
        print(f"  top1>={th}: answered {len(acc)}/{n} ({len(acc)/n:.0%}), correct among answered {sum(acc)}/{len(acc)} = {sum(acc)/max(len(acc),1):.1%}, wrong-but-answered {len(acc)-sum(acc)}")
    # latency on fresh text (cache cleared); warm up first
    r.predict("warm up", conf)
    lat = []
    for t, _ in test + test2:
        r.cache.clear(); t0 = time.perf_counter(); r.predict(t, conf); lat.append((time.perf_counter() - t0) * 1000)
    lat.sort()
    print(f"\nlatency per request incl. chain split (n={len(lat)}): mean {np.mean(lat):.1f} ms, median {lat[len(lat)//2]:.1f}, p95 {lat[int(len(lat)*.95)]:.1f}, max {lat[-1]:.1f}")
    # single encode only
    one = []
    for t, _ in test:
        t0 = time.perf_counter(); enc.embed([t]); one.append((time.perf_counter() - t0) * 1000)
    print(f"single-utterance encode: mean {np.mean(one):.1f} ms median {np.median(one):.1f}")
    print(f"process RSS now {proc.memory_info().rss/1e6:.0f} MB (includes sklearn/numpy + {len(r.cache)} cached embeddings + train matrix); load% {psutil.cpu_percent(interval=1)}")
    pickle.dump(r.clf, open(M / f"clf_{MODE}.pkl", "wb"))


if __name__ == "__main__":
    main()
