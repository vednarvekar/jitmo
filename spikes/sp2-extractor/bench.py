"""Clean live latency + RSS: one process, one model, 4 threads, no cache."""
import os, resource, sys, time
os.environ["OMP_NUM_THREADS"] = "4"
import torch
torch.set_num_threads(4)
sys.argv.append("--live")
from extract2 import *
ex = Extractor("urchade/gliner_small-v2.1", LABELS["a"], 0.5)
rows = load("doc4.txt")
ex.extract("warm up")
rss0 = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
lat = []
for _, t, _ in rows:
    t0 = time.perf_counter(); ex.extract(t); lat.append((time.perf_counter() - t0) * 1000)
lat.sort()
print(f"load+warm RSS {rss0:.0f} MB; mean {sum(lat)/len(lat):.0f} ms median {lat[len(lat)//2]:.0f} p95 {lat[int(len(lat)*.95)]:.0f}; peak RSS {resource.getrusage(resource.RUSAGE_SELF).ru_maxrss/1024:.0f} MB; load avg {os.getloadavg()[0]:.1f}")
