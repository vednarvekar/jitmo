import sys
from extract2 import *
th=float(sys.argv[1]) if len(sys.argv)>1 else 0.5
ex=Extractor("urchade/gliner_small-v2.1",LABELS["a"],th)
val=load("val.txt")
s=score(ex,val,show=True); print(fmt("val",s))
for k,c in sorted(s["per"].items()):
    if sum(c.values()): print(k,dict(c))
