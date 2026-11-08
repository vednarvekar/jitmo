"""Score the frozen rules ONCE on the fresh held-out set (live GLiNER, no cache)."""
import sys
from extract2 import *
ex = Extractor("urchade/gliner_small-v2.1", LABELS["a"], 0.5)
rows = load("fresh.txt")
s = score(ex, rows, show=True)
print(fmt("FRESH held-out (rules + GLiNER)", s))
for k, c in sorted(s["per"].items()):
    if sum(c.values()): print(f"  {k:10s} TP {c['tp']:2d} FP {c['fp']:2d} FN {c['fn']:2d}")
g = score(Extractor("urchade/gliner_small-v2.1", LABELS["a"], 0.5, rules=False), rows)
print(fmt("FRESH held-out (GLiNER only)", g))
