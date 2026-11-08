"""SP3 evaluation. Usage:
  python eval.py --model models/<file>.gguf --set val            (tuning set, rerun freely)
  python eval.py --model models/<file>.gguf --set test --final   (T75-T82, refuses to run twice per model)
"""
import argparse
import json
import os
import re
import resource
import statistics
import sys
from collections import Counter, defaultdict

import plan as P

S = lambda skill, **slots: (skill, slots)  # noqa: E731

# ---- Validation set: written by me, used for prompt tuning ----
VAL = [
    ("Turn the volume up and then play some jazz.",
     [S("system.volume", direction="up"), S("music.play", query="jazz")]),
    ("Search YouTube for guitar tutorials, then open Arijit Singh's latest video.",
     [S("web.search", query="guitar tutorials", site="youtube"), S("youtube.latest", creator="Arijit Singh")]),
    ("Check the weather in Pune and remind me to carry an umbrella at 8am.",
     [S("info.weather", place="Pune"), S("reminder.set", text="carry an umbrella", when="8am")]),
    ("Find last week's invoice PDF and email it to Anita.",
     [S("files.find", query="invoice", type="pdf", date="last week"), S("email.send", to="Anita", file="$step1.result")]),
    ("Take a screenshot, then lock my laptop.",
     [S("system.screenshot"), S("system.lock")]),
    ("What's on my calendar this week? Then block two hours tomorrow morning for focus.",
     [S("calendar.show", range="this week"), S("calendar.block_focus", duration="two hours", when="tomorrow morning")]),
    ("Find Meera's latest email, summarize it and draft a reply saying I'll review it tonight.",
     [S("email.find", person="Meera"), S("email.summarize", target="$step1.result"),
      S("email.draft_reply", target="$step1.result", text="I'll review it tonight")]),
    ("Convert 5 miles to km and message Ved on WhatsApp with the answer.",
     [S("info.math", expression="5 miles to km"), S("whatsapp.send", contact="Ved", text="$step1.result")]),
    ("Switch off the Wi-Fi and set a 25 minute timer.",
     [S("system.toggle", device="wifi", state="off"), S("timer.set", duration="25 minutes")]),
    ("Any new WhatsApp messages? Also, which emails are waiting for my reply?",
     [S("whatsapp.read"), S("email.awaiting_reply")]),
    ("Rename the report to Final Report and move it to the Clients folder.",
     [S("files.rename", file="report", new_name="Final Report"), S("files.move", file="$step1.result", dest="Clients folder")]),
    ("Find the Q3 deck and open it, then share the Marketing folder with the team.",
     [S("files.find", query="Q3 deck"), S("files.open", file="$step1.result"),
      S("files.share_link", item="Marketing folder", **{"with": "team"})]),
    ("Decline the Friday review and tell the team chat I can't make it.",
     [S("calendar.respond", event="Friday review", response="decline"), S("teams.post", channel="team chat", text="I can't make it")]),
    ("Save the attachments from Rahul's last email to the Finance folder, then tell the ops channel on Teams it's done.",
     [S("workflow.save_attachments", email="Rahul's last email", folder="Finance"), S("teams.post", channel="ops channel", text="it's done")]),
    ("When does the sun set today, and what time is it in London?",
     [S("info.astro", what="sunset", date="today"), S("info.time", place="London")]),
    ("Close Spotify and turn on Bluetooth.",
     [S("app.close", app="Spotify"), S("system.toggle", device="bluetooth", state="on")]),
    ("When are Priya and I both free on Wednesday? Then set up a meeting with her then.",
     [S("calendar.free_slot", people="Priya", range="Wednesday"), S("calendar.schedule", people="Priya", when="$step1.result")]),
    ("Give me my morning brief and play my wake-up playlist.",
     [S("workflow.morning_brief"), S("music.play", query="wake-up playlist")]),
    ("Skip this song and make the screen brighter.",
     [S("music.control", action="skip"), S("system.brightness", direction="up")]),
    ("Show what I downloaded today and send the newest one to Rahul on Teams.",
     [S("files.recent", kind="downloads", date="today"), S("files.send", file="$step1.result", to="Rahul", via="teams")]),
    ("Cancel my 3pm and email Rahul that I'm sorry I missed it.",
     [S("calendar.cancel", event="3pm"), S("email.send", to="Rahul", body="I'm sorry I missed it")]),
    ("Add dentist on Friday at 4pm to my calendar and remind me to call mom at 6pm.",
     [S("calendar.add", title="dentist", when="Friday at 4pm"), S("reminder.set", text="call mom", when="6pm")]),
    ("Turn the volume down, take a screenshot, then lock the laptop.",
     [S("system.volume", direction="down"), S("system.screenshot"), S("system.lock")]),
    ("Find Rahul's emails about the budget and archive them.",
     [S("email.find", person="Rahul", topic="budget"), S("email.organize", query="$step1.result", action="archive")]),
]

# ---- Final test: doc 4 T75-T82, gold written from the Expected column + task IDs. Never tuned on. ----
# T79: doc lists A6 (open a website) then A5 (search), so gold uses web.open even though
# "Open Chrome" could also be read as app.open (A1).
TEST = [
    ("T75", "Play lo-fi and turn on Do Not Disturb.",
     [S("music.play", query="lo-fi"), S("system.dnd")]),
    ("T76", "Find the budget sheet and send it to Rahul on WhatsApp.",
     [S("files.find", query="budget sheet"), S("whatsapp.send_file", file="$step1.result", contact="Rahul")]),
    ("T77", "Check my calendar and tell Ved on WhatsApp when I'm free tomorrow.",
     [S("calendar.show", range="tomorrow"), S("whatsapp.send", contact="Ved", text="$step1.result")]),
    ("T78", "Archive the newsletters and show what's urgent.",
     [S("email.organize", query="newsletters", action="archive"), S("email.urgent")]),
    ("T79", "Open Chrome and search for flights to Goa.",
     [S("web.open"), S("web.search", query="flights to Goa")]),
    ("T80", "Take a screenshot and email it to Rahul.",
     [S("system.screenshot"), S("email.send", to="Rahul", file="$step1.result")]),
    ("T81", "Mute notifications, dim the screen and play my focus playlist.",
     [S("system.dnd"), S("system.brightness", direction="down"), S("music.play", query="focus playlist")]),
    ("T82", "Turn Rahul's email into a meeting and invite Priya.",
     [S("workflow.email_to_event", email="Rahul's email"), S("calendar.schedule", people="Priya")]),
]


def norm(v):
    v = str(v).lower().strip()
    v = re.sub(r"[^a-z0-9$.' ]", " ", v)
    v = re.sub(r"\b(the|a|an|my)\b", " ", v)
    return re.sub(r"\s+", " ", v).strip()


def slot_match(gold, pred):
    """Returns (strict, lenient)."""
    if pred is None:
        return False, False
    if "$step" in gold:
        ref = re.search(r"\$step\d+\.result", gold).group(0)
        ok = ref in str(pred)
        return ok, ok
    g, p = norm(gold), norm(pred)
    if g == p:
        return True, True
    if g and p and (g in p or p in g):
        return False, True
    gt, pt = set(g.split()), set(p.split())
    if gt and pt and len(gt & pt) / len(gt | pt) >= 0.5:
        return False, True
    return False, False


def score_item(gold, plan):
    pred = []
    if plan:
        for st in plan["steps"]:
            if isinstance(st, dict):
                pred.append((st.get("skill"), st.get("slots") if isinstance(st.get("slots"), dict) else {}))
    gs = [g[0] for g in gold]
    ps = [p[0] for p in pred]
    gc, pc = Counter(gs), Counter(ps)
    tp = sum((gc & pc).values())
    res = {
        "exact": gs == ps,
        "tp": tp, "fp": sum(pc.values()) - tp, "fn": sum(gc.values()) - tp,
        "slots_total": 0, "slots_strict": 0, "slots_lenient": 0,
        "extra_slots": 0,
        "pairs": [],
    }
    for i in range(max(len(gold), len(pred))):
        g = gold[i][0] if i < len(gold) else "<none>"
        p = pred[i][0] if i < len(pred) else "<missing>"
        res["pairs"].append((g, p))
    for i, (gskill, gslots) in enumerate(gold):
        for k, gv in gslots.items():
            res["slots_total"] += 1
            if i < len(pred) and pred[i][0] == gskill:
                st, le = slot_match(gv, pred[i][1].get(k))
                res["slots_strict"] += st
                res["slots_lenient"] += le
    for i, (pskill, pslots) in enumerate(pred):
        if i < len(gold) and gold[i][0] == pskill:
            res["extra_slots"] += len([k for k in pslots if k not in gold[i][1]])
    return res, pred


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--set", choices=["val", "test"], required=True)
    ap.add_argument("--final", action="store_true")
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--tag", default="")
    a = ap.parse_args()

    name = os.path.basename(a.model).replace(".gguf", "")
    os.makedirs("results", exist_ok=True)
    out = f"results/{a.set}_{name}{a.tag}.json"
    if a.set == "test":
        if not a.final:
            sys.exit("test set needs --final")
        if os.path.exists(out):
            sys.exit(f"{out} already exists: the final test is one-shot per model")

    items = [(f"v{i+1}", r, g) for i, (r, g) in enumerate(VAL)] if a.set == "val" else TEST
    llm = P.load_model(a.model, n_threads=a.threads)
    rss_after_load = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024

    rows, agg = [], defaultdict(float)
    confusion = Counter()
    for iid, req, gold in items:
        plan, raw, st = P.make_plan(llm, req)
        sc, pred = score_item(gold, plan)
        for g, p in sc["pairs"]:
            confusion[(g, p)] += 1
        rows.append({"id": iid, "request": req, "gold": gold, "pred": pred, "raw": raw, **st,
                     **{k: v for k, v in sc.items() if k != "pairs"}})
        print(f"{iid:4} {'OK ' if sc['exact'] else 'BAD'} {st['seconds']:5.1f}s  {req}")
        if not sc["exact"]:
            print("     gold:", [g[0] for g in gold])
            print("     pred:", [p[0] for p in pred])

    n = len(rows)
    secs = [r["seconds"] for r in rows]
    warm = secs[1:] if n > 1 else secs
    peak_rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
    tp, fp, fn = (sum(r[k] for r in rows) for k in ("tp", "fp", "fn"))
    st_tot = sum(r["slots_total"] for r in rows)
    summary = {
        "model": name, "set": a.set, "n": n, "threads": a.threads,
        "exact_match": sum(r["exact"] for r in rows) / n,
        "exact_match_count": sum(r["exact"] for r in rows),
        "step_tp": tp, "step_fp": fp, "step_fn": fn,
        "step_precision": tp / (tp + fp) if tp + fp else 0,
        "step_recall": tp / (tp + fn) if tp + fn else 0,
        "slot_acc_strict": sum(r["slots_strict"] for r in rows) / st_tot,
        "slot_acc_lenient": sum(r["slots_lenient"] for r in rows) / st_tot,
        "extra_slots_total": sum(r["extra_slots"] for r in rows),
        "json_valid": sum(r["json_valid"] for r in rows) / n,
        "tok_per_sec_mean": statistics.mean(r["tok_per_sec"] for r in rows),
        "sec_per_plan_mean": statistics.mean(secs),
        "sec_per_plan_median": statistics.median(secs),
        "sec_per_plan_max": max(secs),
        "sec_per_plan_first_cold": secs[0],
        "sec_per_plan_mean_excl_first": statistics.mean(warm),
        "prefill_sec_mean_excl_first": statistics.mean(r["prefill_seconds"] for r in rows[1:]) if n > 1 else None,
        "plans_under_5s": sum(s < 5 for s in secs),
        "peak_rss_mb": peak_rss, "rss_after_load_mb": rss_after_load,
        "timing_note": "measured while another job was running, re-measure later",
    }
    print(json.dumps(summary, indent=1))
    print("\nConfusion (gold -> predicted, errors only):")
    for (g, p), c in sorted(confusion.items(), key=lambda kv: -kv[1]):
        if g != p:
            print(f"  {g:28} -> {p:28} x{c}")
    print(f"  (correct skill positions: {sum(c for (g, p), c in confusion.items() if g == p)})")
    json.dump({"summary": summary, "rows": rows,
               "confusion": [[g, p, c] for (g, p), c in confusion.items()]},
              open(out, "w"), indent=1)


if __name__ == "__main__":
    main()
