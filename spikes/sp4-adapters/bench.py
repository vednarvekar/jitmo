import sys, time, ctypes, json, statistics, psutil, gc
import llama_cpp as L
base = sys.argv[1]; N = int(sys.argv[2]) if len(sys.argv) > 2 else 60
proc = psutil.Process(); rss = lambda: proc.memory_info().rss / 2**20
llm = L.Llama(model_path=base, n_ctx=512, n_threads=4, verbose=False, seed=0)
ctx, model = llm._ctx.ctx, llm._model.model
PROMPT = "Plan: open the budget sheet and send it to Rahul."
def gen():
    llm.reset(); llm._ctx.kv_cache_clear() if hasattr(llm._ctx, "kv_cache_clear") else None
    return llm.create_completion(PROMPT, max_tokens=2, temperature=0.0)["choices"][0]["text"]
def set_adapters(ads):
    n = len(ads)
    arr = (L.llama_adapter_lora_p_ctypes * max(n, 1))(*ads) if n else None
    sc = (ctypes.c_float * max(n, 1))(*([1.0] * n)) if n else None
    r = L.llama_set_adapters_lora(ctx, arr, n, sc); assert r == 0, r
out = {"base": base, "rss_after_base_load_mb": rss()}
gen()  # warm
bl=[]
for _ in range(8):
    t = time.perf_counter(); o0 = gen(); bl.append((time.perf_counter() - t) * 1e3)
out["gen_no_adapter_ms_median"] = statistics.median(bl); out["gen_no_adapter_ms_all"] = bl
r0 = rss()
# strategy A: load all adapters once (disk read + weight tensor alloc), then hot-swap by pointer
names = ["a", "b", "c"]; ads = {}; loadt = {}
for n in names:
    t = time.perf_counter(); ads[n] = L.llama_adapter_lora_init(model, f"adapters/lora_{n}.gguf".encode()); loadt[n] = (time.perf_counter() - t) * 1e3
    assert ads[n]
out["adapter_load_ms"] = loadt; out["rss_after_adapters_loaded_mb"] = rss()
outs = {}
swap_ms, swap_gen_ms, rss_series = [], [], []
seq = [names[i % 3] for i in range(N)]
for i, n in enumerate(seq):
    t = time.perf_counter(); set_adapters([ads[n]]); t1 = time.perf_counter()
    o = gen(); t2 = time.perf_counter()
    outs.setdefault(n, o)
    swap_ms.append((t1 - t) * 1e3); swap_gen_ms.append((t2 - t1) * 1e3); rss_series.append(rss())
out["outputs_differ_by_adapter"] = {"none": o0, **outs}
def st(x): s = sorted(x); return {"mean": statistics.mean(x), "median": statistics.median(x), "p95": s[int(.95 * len(s)) - 1], "max": max(x)}
out["A_hot_swap_call_ms"] = st(swap_ms)
out["A_first_gen_after_swap_ms (2 tok)"] = st(swap_gen_ms)
out["A_rss_first_mb"] = rss_series[0]; out["A_rss_last_mb"] = rss_series[-1]; out["A_rss_max_mb"] = max(rss_series)
out["A_rss_drift_mb"] = rss_series[-1] - rss_series[0]; out["n_swaps"] = N
# swap to none and back
set_adapters([]); t = time.perf_counter(); gen(); out["gen_after_detach_ms"] = (time.perf_counter() - t) * 1e3
# strategy B: lazy load+free per swap (no resident adapters)
for n in names: L.llama_adapter_lora_free(ads[n])
set_adapters([])
lm, rs = [], []
for i in range(N // 2):
    n = names[i % 3]
    t = time.perf_counter(); a = L.llama_adapter_lora_init(model, f"adapters/lora_{n}.gguf".encode()); set_adapters([a]); lm.append((time.perf_counter() - t) * 1e3)
    gen(); set_adapters([]); L.llama_adapter_lora_free(a); rs.append(rss())
out["B_load_from_disk_and_apply_ms"] = st(lm); out["B_rss_first_mb"] = rs[0]; out["B_rss_last_mb"] = rs[-1]; out["B_rss_drift_mb"] = rs[-1] - rs[0]
print(json.dumps(out, indent=1)); json.dump(out, open(sys.argv[3] if len(sys.argv) > 3 else "result.json", "w"), indent=1)
