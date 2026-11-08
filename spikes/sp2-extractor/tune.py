"""Tune loop: errors shown for doc4, val, val2 only. test.txt (burned) is numbers only; extra sets via argv are numbers only unless --show."""
import sys
from extract2 import *
ex = Extractor("urchade/gliner_small-v2.1", LABELS["a"], 0.5)
for f in ("doc4.txt", "val.txt", "val2.txt"):
    print(fmt(f, score(ex, load(f), show="--quiet" not in sys.argv)))
print(fmt("test.txt (burned, regression)", score(ex, load("test.txt"))))
