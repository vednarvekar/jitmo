"""Runtime-only footprint: ONNX session + tokenizer + numpy head (no sklearn/training data). Usage: bench_runtime.py fp32|int8"""
import sys, time, re
import numpy as np, psutil
import onnxruntime as ort
from tokenizers import Tokenizer
mode = sys.argv[1]
p = psutil.Process()
base = p.memory_info().rss / 1e6
so = ort.SessionOptions(); so.intra_op_num_threads = 4
s = ort.InferenceSession("models/model.onnx" if mode == "fp32" else "models/model_int8.onnx", so, providers=["CPUExecutionProvider"])
tok = Tokenizer.from_file("models/tokenizer.json"); tok.enable_truncation(128); tok.enable_padding()
h = np.load("models/head_%s.npz" % mode, allow_pickle=True)
names = {i.name for i in s.get_inputs()}
def emb(texts):
    e = tok.encode_batch(texts); ids = np.array([x.ids for x in e], dtype=np.int64); m = np.array([x.attention_mask for x in e], dtype=np.int64)
    f = {"input_ids": ids, "attention_mask": m}
    if "token_type_ids" in names: f["token_type_ids"] = np.zeros_like(ids)
    o = s.run(None, f)[0]; v = (o * m[..., None]).sum(1) / m.sum(1, keepdims=True); return v / np.linalg.norm(v, axis=1, keepdims=True)
def top(texts):
    z = emb(texts) @ h["coef"].T + h["b"]; z = np.exp(z - z.max(1, keepdims=True)); pr = z / z.sum(1, keepdims=True); i = pr.argmax(1)
    return [(h["classes"][k], pr[j, k]) for j, k in enumerate(i)]
SPLIT = re.compile(r"\s*(?:,\s*then\s+|\s+and\s+then\s+|\s+then\s+|\s+after that\s+|,\s+and\s+|,\s*|\s+and\s+|\s+&\s+)", re.I)
def route(t):
    parts = [x for x in SPLIT.split(t) if x.strip()]; whole = top([t])[0]
    if len(parts) > 1:
        r = top(parts)
        if all(c >= 0.8 for _, c in r) and len({l for l, _ in r}) > 1 and "OUT" not in {l for l, _ in r}: return {l for l, _ in r}
    return {whole[0]}
texts = [l.split("|", 1)[1].strip() for f in ("data/test.txt", "data/test2.txt") for l in open(f, encoding="utf-8")]
route("warm up")
lat = []
for t in texts:
    t0 = time.perf_counter(); route(t); lat.append((time.perf_counter() - t0) * 1000)
lat.sort()
print(f"{mode}: runtime RSS {p.memory_info().rss/1e6:.0f} MB (python+ort baseline before model {base:.0f} MB); latency n={len(lat)} mean {np.mean(lat):.1f} median {lat[len(lat)//2]:.1f} p95 {lat[int(len(lat)*.95)]:.1f} max {lat[-1]:.1f} ms; cpu load {psutil.cpu_percent(interval=1):.0f}%")
