import sys
from extract2 import *
name = sys.argv[1]
for th in (0.3, 0.5):
    ex = Extractor(name, LABELS["a"], th)
    for f in ("doc4.txt", "fresh.txt"):
        print(th, f, fmt(f, score(ex, load(f))), flush=True)
