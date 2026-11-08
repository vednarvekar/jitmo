import pickle, extract2
from extract2 import *
raw = pickle.load(open(ROOT / "data" / "raw_onnx_int8.pkl", "rb"))
name = "x/gliner_small_onnx_int8"
extract2._CACHE_LOADED[name] = raw
for th in (0.3, 0.35, 0.4, 0.45, 0.5):
    ex = Extractor(name, LABELS["a"], th)
    r = {f: score(ex, load(f))["perfect"] for f in ("doc4.txt", "val.txt", "val2.txt")}
    tune = sum(r.values())
    r2 = {f: score(ex, load(f))["perfect"] for f in ("test.txt", "fresh.txt")}
    print(th, r, "tuning total", tune, "/164 |", r2)
