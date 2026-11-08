"""SP3 extras: fresh chained requests (not in VAL/TEST/few-shot) + timing breakdown.
Usage: python extra.py MODEL [threads]
Reports per-request plan correctness, wall time, process CPU seconds, peak RSS,
and a grammar-vs-no-grammar decode-speed comparison (same prompt) to find where time goes.
"""
import json, sys, time, statistics
import psutil, types
# eval.py (unchanged from WSL) imports the Unix-only `resource`; stub it so we can reuse S/score_item on Windows.
sys.modules.setdefault("resource", types.ModuleType("resource"))
import plan as P
from eval import S, score_item


def peak_mb():
    mi = psutil.Process().memory_info()
    return getattr(mi, "peak_wset", mi.rss) / 2**20

EXTRA = [
    ("Open Excel and play some Arijit Singh.",
     [S("app.open", app="Excel"), S("music.play", query="Arijit Singh")]),
    ("What's the weather in Delhi tomorrow and remind me to take a jacket at 7am.",
     [S("info.weather", place="Delhi", date="tomorrow"), S("reminder.set", text="take a jacket", when="7am")]),
    ("Find the rent receipt and share it with Kavya on Teams.",
     [S("files.find", query="rent receipt"), S("files.send", file="$step1.result", to="Kavya", via="teams")]),
    ("Show urgent emails, then summarize the first one.",
     [S("email.urgent"), S("email.summarize", target="$step1.result")]),
    ("Mute the volume, turn off Wi-Fi and lock the screen.",
     [S("system.volume", direction="mute"), S("system.toggle", device="wifi", state="off"), S("system.lock")]),
    ("Check my schedule for today and message Anita on WhatsApp that I'm in meetings till 5.",
     [S("calendar.show", range="today"), S("whatsapp.send", contact="Anita", text="I'm in meetings till 5")]),
    ("Pause the music, take a screenshot and send it to Meera by email.",
     [S("music.control", action="pause"), S("system.screenshot"), S("email.send", to="Meera", file="$step1.result")]),
    ("Start focus mode for 45 minutes and close Slack.",
     [S("workflow.focus_mode", duration="45 minutes"), S("app.close", app="Slack")]),
    ("Open YouTube and search for sourdough recipes.",
     [S("web.open", site="YouTube"), S("web.search", query="sourdough recipes", site="youtube")]),
    ("Find the quarterly report, rename it to Q2 Report and email it to Rahul.",
     [S("files.find", query="quarterly report"), S("files.rename", file="$step1.result", new_name="Q2 Report"),
      S("email.send", to="Rahul", file="$step2.result")]),
]


def main():
    path = sys.argv[1]
    th = int(sys.argv[2]) if len(sys.argv) > 2 else 4
    proc = psutil.Process()
    llm = P.load_model(path, n_threads=th)
    rss_load = proc.memory_info().rss / 2**20
    rows = []
    for req, gold in EXTRA:
        c0 = proc.cpu_times(); t0 = time.perf_counter()
        plan, raw, st = P.make_plan(llm, req)
        wall = time.perf_counter() - t0; c1 = proc.cpu_times()
        sc, pred = score_item(gold, plan)
        cpu = (c1.user + c1.system) - (c0.user + c0.system)
        rows.append({"request": req, "exact": sc["exact"], "wall_s": wall, "cpu_s": cpu, "out_tokens": st["out_tokens"],
                     "prefill_s": st["prefill_seconds"], "tok_per_sec": st["tok_per_sec"],
                     "gold": [g[0] for g in gold], "pred": [p[0] for p in pred], "raw": raw})
        print(f"{'OK ' if sc['exact'] else 'BAD'} wall={wall:5.1f}s cpu={cpu:5.1f}s tok={st['out_tokens']:3d} {req}", flush=True)
        if not sc["exact"]:
            print("    gold", [g[0] for g in gold], "pred", [p[0] for p in pred], flush=True)

    # grammar vs no-grammar decode speed on one request (prefix is cached so mostly decode)
    msgs = P.messages_for(EXTRA[0][0])
    def run(grammar):
        t0 = time.perf_counter(); n = 0; first = None
        for ch in llm.create_chat_completion(messages=msgs, grammar=grammar, temperature=0.0, max_tokens=60, stream=True):
            if ch["choices"][0]["delta"].get("content"):
                n += 1; first = first or time.perf_counter()
        t1 = time.perf_counter()
        return n, (n - 1) / max(t1 - first, 1e-9)
    gram = {}
    for label, g in (("grammar", P.get_grammar()), ("no_grammar", None), ("grammar", P.get_grammar()), ("no_grammar", None)):
        gram.setdefault(label, []).append(run(g))
    out = {"model": path, "threads": th, "rss_after_load_mb": rss_load, "peak_mb": peak_mb(),
           "n_correct": sum(r["exact"] for r in rows), "n": len(rows),
           "wall_median_s": statistics.median(r["wall_s"] for r in rows),
           "cpu_median_s": statistics.median(r["cpu_s"] for r in rows),
           "decode_tok_per_s": gram, "rows": rows,
           "system_cpu_percent_sample": psutil.cpu_percent(interval=1)}
    json.dump(out, open("results/extra_%dthreads.json" % th, "w"), indent=1)
    print(json.dumps({k: v for k, v in out.items() if k != "rows"}, indent=1))


if __name__ == "__main__":
    main()
