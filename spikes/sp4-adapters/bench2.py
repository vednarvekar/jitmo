"""SP4 round 2 (Windows). Usage: python bench2.py BASE.gguf [N_SWAPS] [OUT.json]
 1. 500 hot swaps (a/b/c rotating) with a 1-token decode after each (forces the adapter into the graph);
    records swap-call time, USS (private memory, not distorted by mmap paging) and RSS every swap.
 2. Interleaved A/B timing of 8-token generation: no adapter vs adapter, ABAB... so machine noise hits both equally.
 3. 100 cold cycles: load adapter from disk, apply, 1-token decode, detach, free; USS drift.
"""
import ctypes, gc, json, statistics, sys, time
import psutil
import llama_cpp as L

base = sys.argv[1]; N = int(sys.argv[2]) if len(sys.argv) > 2 else 500
outp = sys.argv[3] if len(sys.argv) > 3 else "result_r2.json"
proc = psutil.Process()
mb = lambda b: b / 2**20
uss = lambda: mb(proc.memory_full_info().uss)
rss = lambda: mb(proc.memory_info().rss)
llm = L.Llama(model_path=base, n_ctx=512, n_threads=4, verbose=False, seed=0)
ctx, model = llm._ctx.ctx, llm._model.model
PROMPT = "Plan: open the budget sheet and send it to Rahul."


def gen(n):
    llm.reset()
    return llm.create_completion(PROMPT, max_tokens=n, temperature=0.0)["choices"][0]["text"]


def set_adapters(ads):
    n = len(ads)
    arr = (L.llama_adapter_lora_p_ctypes * max(n, 1))(*ads) if n else None
    sc = (ctypes.c_float * max(n, 1))(*([1.0] * n)) if n else None
    r = L.llama_set_adapters_lora(ctx, arr, n, sc); assert r == 0, r


def st(x):
    s = sorted(x)
    return {"n": len(x), "mean": statistics.mean(x), "median": statistics.median(x),
            "p95": s[max(int(.95 * len(s)) - 1, 0)], "max": max(x)}


out = {"base": base, "n_swaps": N, "load_avg_note": "other agents' ML job running; see cpu_percent"}
gen(8); gen(8)  # warm
out["uss_after_load_mb"] = uss(); out["rss_after_load_mb"] = rss()
names = ["a", "b", "c"]; ads = {}; loadms = {}
for n in names:
    t = time.perf_counter(); ads[n] = L.llama_adapter_lora_init(model, f"adapters/lora_{n}.gguf".encode())
    loadms[n] = (time.perf_counter() - t) * 1e3; assert ads[n]
out["adapter_load_ms"] = loadms; out["uss_after_adapters_mb"] = uss()

# 1. hot swaps
sw, uss_s, rss_s = [], [], []
for i in range(N):
    n = names[i % 3]
    t = time.perf_counter(); set_adapters([ads[n]]); sw.append((time.perf_counter() - t) * 1e3)
    gen(1)
    if i % 10 == 0 or i == N - 1:
        uss_s.append(uss()); rss_s.append(rss())
out["hot_swap_call_ms"] = st(sw)
out["hot_uss_mb"] = {"first": uss_s[0], "last": uss_s[-1], "min": min(uss_s), "max": max(uss_s), "drift": uss_s[-1] - uss_s[0]}
out["hot_rss_mb"] = {"first": rss_s[0], "last": rss_s[-1], "min": min(rss_s), "max": max(rss_s)}
print("hot done", flush=True)

# 2. per-token overhead, interleaved
base_t, lora_t = [], []
K = 8
for rep in range(8):
    set_adapters([]); t = time.perf_counter(); gen(K); base_t.append((time.perf_counter() - t) / K * 1e3)
    set_adapters([ads["a"]]); t = time.perf_counter(); gen(K); lora_t.append((time.perf_counter() - t) / K * 1e3)
    set_adapters([ads["c"]]); t = time.perf_counter(); gen(K); lora_t.append((time.perf_counter() - t) / K * 1e3)
out["ms_per_token_no_adapter"] = st(base_t); out["ms_per_token_with_adapter"] = st(lora_t)
out["lora_overhead_ratio_of_medians"] = statistics.median(lora_t) / statistics.median(base_t)
print("overhead done", flush=True)

# 3. cold load/apply/free cycles
for n in names: L.llama_adapter_lora_free(ads[n])
set_adapters([])
gc.collect(); u0 = uss(); lm, us = [], []
for i in range(100):
    n = names[i % 3]
    t = time.perf_counter(); a = L.llama_adapter_lora_init(model, f"adapters/lora_{n}.gguf".encode()); set_adapters([a])
    lm.append((time.perf_counter() - t) * 1e3)
    gen(1); set_adapters([]); L.llama_adapter_lora_free(a)
    if i % 10 == 0 or i == 99: us.append(uss())
out["cold_load_apply_ms"] = st(lm)
out["cold_uss_mb"] = {"first": us[0], "last": us[-1], "min": min(us), "max": max(us), "drift": us[-1] - us[0]}
out["cpu_percent_at_end"] = psutil.cpu_percent(interval=1)
print(json.dumps(out, indent=1)); json.dump(out, open(outp, "w"), indent=1)
