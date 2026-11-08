"""Latency + RSS + accuracy for GLiNER-small, PyTorch vs ONNX int8.
Usage: python bench2.py torch|onnx
Latency: model call + rules, one request at a time, no cache, after 3 warm-up calls.
Accuracy: raw outputs (threshold 0.3) are dumped to data/raw_<tag>.pkl and re-scored with the frozen rules."""
import os, sys, time, pickle
mode = sys.argv[1]
threads = int(os.environ.get("THREADS", "4"))
os.environ["OMP_NUM_THREADS"] = str(threads)
import psutil
proc = psutil.Process()
import extract2
from extract2 import *

if mode == "torch":
    import torch
    torch.set_num_threads(threads)
    from gliner import GLiNER
    t0 = time.perf_counter()
    model = GLiNER.from_pretrained("urchade/gliner_small-v2.1")
    tag, name = "torch", "urchade/gliner_small-v2.1"
else:
    import onnxruntime as ort
    from gliner import GLiNER
    so = ort.SessionOptions()
    so.intra_op_num_threads = threads
    t0 = time.perf_counter()
    model = GLiNER.from_pretrained(str(ROOT / "models" / "gliner_small_onnx"), load_onnx_model=True, load_tokenizer=True,
                                   onnx_model_file="onnx/model_int8.onnx", session_options=so)
    tag, name = "onnx_int8", "x/gliner_small_onnx_int8"
load_s = time.perf_counter() - t0
labels = list(LABELS["a"].values())
for _ in range(3):
    model.predict_entities("warm up the engine please", labels, threshold=0.3)
rss_loaded = proc.memory_info().rss / 1e6

texts = [t for f in ("doc4.txt", "fresh.txt") for _, t, _ in load(f)]
extract2._CACHE_LOADED[name] = {}
ex = Extractor(name, LABELS["a"], 0.5)
ex.model = model
psutil.cpu_percent(None)
lat_model, lat_full = [], []
for t in texts:
    t0 = time.perf_counter()
    model.predict_entities(t, labels, threshold=0.5)
    lat_model.append((time.perf_counter() - t0) * 1000)
for t in texts:
    t0 = time.perf_counter()
    ex.extract(t)
    lat_full.append((time.perf_counter() - t0) * 1000)
cpu = psutil.cpu_percent(None)


def stat(a):
    a = sorted(a)
    return f"mean {sum(a)/len(a):.0f} ms, median {a[len(a)//2]:.0f}, p95 {a[int(len(a)*.95)]:.0f}, max {a[-1]:.0f}"


print(f"[{tag}] threads={threads} load {load_s:.1f}s; RSS after load+warmup {rss_loaded:.0f} MB; RSS peak-ish now {proc.memory_info().rss/1e6:.0f} MB")
print(f"[{tag}] model call only (n={len(texts)}): {stat(lat_model)}")
print(f"[{tag}] model + rules  (n={len(texts)}): {stat(lat_full)}")
print(f"[{tag}] system CPU% during timing (includes other processes): {cpu:.0f}")

# accuracy: dump raw outputs at 0.3 for all sets, then score with the frozen rules
raw = {}
for f in ("val.txt", "doc4.txt", "val2.txt", "test.txt", "fresh.txt"):
    for _, t, _ in load(f):
        raw[(name, t)] = model.predict_entities(t, labels, threshold=0.3)
pickle.dump(raw, open(ROOT / "data" / f"raw_{tag}.pkl", "wb"))
extract2._CACHE_LOADED[name] = raw
ex2 = Extractor(name, LABELS["a"], 0.5)
for f in ("doc4.txt", "val.txt", "val2.txt", "test.txt", "fresh.txt"):
    print(fmt(f"[{tag}] {f}", score(ex2, load(f))))
