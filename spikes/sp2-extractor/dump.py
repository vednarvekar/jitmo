"""Cache raw GLiNER predictions (threshold 0.3) for val/doc4/test so rule tuning is instant.
Only val is used during tuning; doc4/test raw outputs are cached but not inspected."""
import os, pickle, sys
os.environ["OMP_NUM_THREADS"] = "4"
from extract2 import *
m = sys.argv[1]
from gliner import GLiNER
g = GLiNER.from_pretrained(m)
out = {}
for f in ("val.txt", "doc4.txt", "test.txt"):
    for rid, text, _ in load(f):
        out[(m, text)] = g.predict_entities(text, list(LABELS["a"].values()), threshold=0.3)
pickle.dump(out, open(f"/tmp/raw_{m.split('/')[1]}.pkl", "wb"))
