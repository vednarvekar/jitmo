"""Early footprint numbers for SP8: RSS of the real ONNX models under default vs low-footprint ONNX Runtime
options, after load, after inference, after unload (+ EmptyWorkingSet). One fresh process per case."""
import sys, json, time, subprocess, os
CASES = {
 "router_int8": r"..\sp1-router\models\model_int8.onnx",
 "gliner_small_int8": r"..\sp2-extractor\models\gliner_small_onnx\onnx\model_int8.onnx",
}
def child(name, path, mode):
    import numpy as np, psutil, gc, ctypes, onnxruntime as ort
    p = psutil.Process(); mb = lambda: round(p.memory_info().rss / 2**20, 1)
    out = {"case": name, "mode": mode, "base": mb()}
    so = ort.SessionOptions()
    if mode == "low":
        so.enable_cpu_mem_arena = False; so.enable_mem_pattern = False
        so.intra_op_num_threads = 1; so.inter_op_num_threads = 1
        so.add_session_config_entry("session.use_env_allocators", "0")
    t0 = time.perf_counter(); sess = ort.InferenceSession(path, so, providers=["CPUExecutionProvider"]); out["load_ms"] = round((time.perf_counter() - t0) * 1000)
    out["loaded"] = mb()
    feeds = {}
    for i in sess.get_inputs():
        shape = [1 if not isinstance(d, int) else d for d in i.shape]
        shape = [16 if (k > 0 and not isinstance(i.shape[k], int)) else s for k, s in enumerate(shape)]
        dt = {"tensor(int64)": np.int64, "tensor(float)": np.float32, "tensor(bool)": np.bool_, "tensor(int32)": np.int32}.get(i.type, np.float32)
        feeds[i.name] = np.ones(shape, dt)
    try:
        t = []
        for _ in range(30):
            s = time.perf_counter(); sess.run(None, feeds); t.append((time.perf_counter() - s) * 1000)
        out["ms_median"] = round(sorted(t)[len(t)//2], 1)
    except Exception as e:  # noqa
        out["run_error"] = str(e)[:60]; e = None
    out["after_infer"] = mb(); out["peak"] = round(p.memory_info().peak_wset / 2**20, 1)
    del sess; gc.collect(); out["after_unload"] = mb()
    ctypes.windll.psapi.EmptyWorkingSet(ctypes.windll.kernel32.GetCurrentProcess())
    out["after_trim"] = mb()
    print(json.dumps(out))
if len(sys.argv) > 1:
    child(*sys.argv[1:4]); sys.exit()
for name, path in CASES.items():
    for mode in ("default", "low"):
        r = subprocess.run([sys.executable, __file__, name, path, mode], capture_output=True, text=True)
        print(r.stdout.strip() or r.stderr.strip()[-300:])
