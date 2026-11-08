"""Does our model worker hurt a busy machine? A CPU-saturating 'dev workload' (one spinner per logical core) is timed
alone, then while a worker runs router inference flat out at different Windows priorities."""
import sys, os, time, ctypes, multiprocessing as mp, subprocess, json
PRIO = {"normal": 0x20, "below_normal": 0x4000, "idle": 0x40}
def spinner(q, secs):
    end = time.perf_counter() + secs; n = 0
    while time.perf_counter() < end:
        for _ in range(20000): n += 1
    q.put(n)
def worker(prio, ecoqos, secs):
    import numpy as np, onnxruntime as ort
    k = ctypes.windll.kernel32; h = k.GetCurrentProcess(); k.SetPriorityClass(ctypes.c_void_p(h), PRIO[prio])
    if ecoqos:  # PROCESS_POWER_THROTTLING_STATE {Version=1, ControlMask=1(EXECUTION_SPEED), StateMask=1}
        st = (ctypes.c_ulong * 3)(1, 1, 1); k.SetProcessInformation(ctypes.c_void_p(h), 4, st, 12)
    so = ort.SessionOptions(); so.intra_op_num_threads = 1; so.inter_op_num_threads = 1
    s = ort.InferenceSession(r"..\sp1-router\models\model_int8.onnx", so, providers=["CPUExecutionProvider"])
    feeds = {i.name: np.ones([1, 16], np.int64) for i in s.get_inputs()}
    end = time.perf_counter() + secs; n = 0
    while time.perf_counter() < end: s.run(None, feeds); n += 1
    print(json.dumps({"worker_runs_per_s": round(n / secs)}))
def competitor(secs):
    q = mp.Queue(); ps = [mp.Process(target=spinner, args=(q, secs)) for _ in range(os.cpu_count())]
    [p.start() for p in ps]; tot = sum(q.get() for _ in ps); [p.join() for p in ps]; return tot
if __name__ == "__main__":
    if len(sys.argv) > 1: worker(sys.argv[1], sys.argv[2] == "eco", float(sys.argv[3])); sys.exit()
    SECS = 8; print("logical cores:", os.cpu_count())
    base = competitor(SECS); print("competitor alone:", base, "(=100%)")
    for prio, eco in (("normal", False), ("below_normal", False), ("idle", False), ("idle", True)):
        w = subprocess.Popen([sys.executable, __file__, prio, "eco" if eco else "no", str(SECS)], stdout=subprocess.PIPE, text=True)
        time.sleep(1.5); got = competitor(SECS - 2); w.wait()
        # competitor ran SECS-2 seconds, scale to SECS
        pct = round(100 * got * SECS / (SECS - 2) / base)
        print(f"worker={prio:12s} eco={eco!s:5s} competitor keeps {pct}%   {w.stdout.read().strip()}")
