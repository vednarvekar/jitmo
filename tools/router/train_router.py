"""Reproducible router training: regenerate data, embed, train the head, write head.json + metrics.

Run from the repo root with the SP1 venv (it has onnxruntime, tokenizers, scikit-learn):

    spikes/sp1-router/.venv/Scripts/python.exe tools/router/train_router.py --regen
    spikes/sp1-router/.venv/Scripts/python.exe tools/router/train_router.py --regen --score-heldout

Steps: (--regen) gen_data.py -> data/train.jsonl; drop any training row that equals a
sentence of the four frozen SP1 sets (test, test2, val, val2) or the Phase 2 held-out file;
embed with the int8 ONNX MiniLM (same math as crates/model-worker/src/router.rs: mean pool
over real tokens, L2 normalise); fit LogisticRegression(C=30); write models-src/router/head.json
(classes, coef, b; the committed copy; also models/router/head.json when that folder exists)
and tools/router/metrics_<tag>.md / .json.

The held-out Phase 2 file is only scored with --score-heldout (score it once, after the
training data is frozen). `--eval-head PATH` scores an existing head.json instead of training,
which is how the "before" numbers were produced.
"""
import argparse
import json
import sys
from collections import Counter
from pathlib import Path

import numpy as np
import onnxruntime as ort
from sklearn.linear_model import LogisticRegression
from tokenizers import Tokenizer

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
SP1 = REPO / "spikes" / "sp1-router" / "data"
CUTOFF = 0.8  # chain-part confidence, the SP1 value also used in router.rs
SPLIT = __import__("re").compile(
    r"\s*(?:,\s*then\s+|\s+and\s+then\s+|\s+then\s+|\s+after that\s+|,\s+and\s+|,\s*|\s+and\s+|\s+&\s+)",
    __import__("re").I,
)
FROZEN = ["test.txt", "test2.txt", "val.txt", "val2.txt"]
PAIRS = {
    "S1 vs A3 (volume vs media)": ("S1", "A3"),
    "E5 vs F5 (reply vs send file)": ("E5", "F5"),
    "E7 vs M1 (email vs whatsapp)": ("E7", "M1"),
    "E7 vs E5 (new mail vs reply)": ("E7", "E5"),
    "A2 vs A3 (play X vs pause/skip)": ("A2", "A3"),
}


def norm(s):
    return s.lower().strip(" .?!")


def load_txt(path):
    rows = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        if "|" in line:
            lab, text = line.split("|", 1)
            rows.append((text, lab.split("+")))
    return rows


def find_encoder_dir(arg):
    cands = [Path(arg)] if arg else []
    cands += [REPO / "models" / "router", REPO / "spikes" / "sp1-router" / "models"]
    for c in cands:
        if (c / "model_int8.onnx").exists() and (c / "tokenizer.json").exists():
            return c
    sys.exit("encoder not found: need model_int8.onnx + tokenizer.json (see models-src/router/README.md; "
             "pass --encoder-dir)")


class Encoder:
    def __init__(self, d):
        so = ort.SessionOptions()
        so.intra_op_num_threads = 4
        self.s = ort.InferenceSession(str(d / "model_int8.onnx"), so, providers=["CPUExecutionProvider"])
        self.tok = Tokenizer.from_file(str(d / "tokenizer.json"))
        self.tok.enable_truncation(128)
        self.tok.enable_padding()
        self.names = {i.name for i in self.s.get_inputs()}
        self.cache = {}

    def embed(self, texts):
        todo = [t for t in dict.fromkeys(texts) if t not in self.cache]
        for i in range(0, len(todo), 64):
            batch = todo[i:i + 64]
            enc = self.tok.encode_batch(batch)
            ids = np.array([e.ids for e in enc], dtype=np.int64)
            mask = np.array([e.attention_mask for e in enc], dtype=np.int64)
            feeds = {"input_ids": ids, "attention_mask": mask}
            if "token_type_ids" in self.names:
                feeds["token_type_ids"] = np.zeros_like(ids)
            h = self.s.run(None, feeds)[0]
            m = mask[..., None]
            v = (h * m).sum(1) / m.sum(1)
            v = v / np.linalg.norm(v, axis=1, keepdims=True)
            for t, x in zip(batch, v):
                self.cache[t] = x
        return np.array([self.cache[t] for t in texts])


class Head:
    def __init__(self, classes, coef, b):
        self.classes, self.coef, self.b = list(classes), np.array(coef), np.array(b)

    @classmethod
    def load(cls, path):
        j = json.loads(Path(path).read_text(encoding="utf-8"))
        return cls(j["classes"], j["coef"], j["b"])

    def proba(self, v):
        z = v @ self.coef.T + self.b
        z = z - z.max(1, keepdims=True)
        e = np.exp(z)
        return e / e.sum(1, keepdims=True)


class Router:
    def __init__(self, enc, head):
        self.enc, self.head = enc, head

    def top(self, texts):
        p = self.head.proba(self.enc.embed(texts))
        i = p.argmax(1)
        return [(self.head.classes[k], float(row[k])) for row, k in zip(p, i)]

    def predict(self, text):
        parts = [p for p in SPLIT.split(text) if p.strip()]
        whole = self.top([text])[0][0]
        if len(parts) > 1:
            res = self.top(parts)
            labs = {l for l, _ in res}
            if all(c >= CUTOFF for _, c in res) and "OUT" not in labs and len(labs) > 1:
                return labs
        return {whole}


def evaluate(r, rows):
    out = {"n": len(rows), "exact": 0, "single": [0, 0], "chain": [0, 0], "out": [0, 0], "errors": []}
    for text, labels in rows:
        want = set(labels)
        got = r.predict(text)
        hit = got == want
        out["exact"] += hit
        b = out["chain"] if len(want) > 1 else out["single"]
        b[0] += hit
        b[1] += 1
        if want == {"OUT"}:
            out["out"][0] += hit
            out["out"][1] += 1
        if not hit:
            out["errors"].append((text, sorted(want), sorted(got)))
    return out


def pct(a, b):
    return f"{a}/{b} = {100 * a / b:.1f}%" if b else "n/a"


def pair_errors(res):
    """For each confusable pair count single-task errors in both directions."""
    table = {}
    for name, (a, b) in PAIRS.items():
        ab = sum(1 for _, w, g in res["errors"] if w == [a] and g == [b])
        ba = sum(1 for _, w, g in res["errors"] if w == [b] and g == [a])
        table[name] = (ab, ba)
    out_as_task = sum(1 for _, w, g in res["errors"] if w == ["OUT"] and len(g) == 1 and g != ["OUT"])
    task_as_out = sum(1 for _, w, g in res["errors"] if len(w) == 1 and w != ["OUT"] and g == ["OUT"])
    table["OUT looks in scope (OUT -> task)"] = (out_as_task, 0)
    table["in-scope refused (task -> OUT)"] = (task_as_out, 0)
    return table


def report(name, res, lines):
    lines.append(
        f"| {name} | {pct(res['exact'], res['n'])} | {pct(*res['single'])} | {pct(*res['chain'])} | {pct(*res['out'])} |"
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--regen", action="store_true", help="regenerate data/train.jsonl first")
    ap.add_argument("--encoder-dir")
    ap.add_argument("--out", default=str(REPO / "models-src" / "router" / "head.json"))
    ap.add_argument("--eval-head", help="score this head.json instead of training")
    ap.add_argument("--score-heldout", action="store_true", help="also score data/heldout_phase2.txt")
    ap.add_argument("--no-write", action="store_true", help="do not write head.json / metrics")
    ap.add_argument("--errors", action="store_true", help="print every error")
    ap.add_argument("--C", type=float, default=30.0)
    ap.add_argument("--tag", default="after")
    a = ap.parse_args()

    enc = Encoder(find_encoder_dir(a.encoder_dir))
    sets = {f[:-4]: load_txt(SP1 / f) for f in FROZEN}
    held_path = HERE / "data" / "heldout_phase2.txt"
    heldout = load_txt(held_path) if held_path.exists() else []

    if a.eval_head:
        head = Head.load(a.eval_head)
        n_train = None
    else:
        if a.regen:
            sys.path.insert(0, str(HERE))
            import gen_data

            gen_data.main()
        rows = [json.loads(l) for l in (HERE / "data" / "train.jsonl").read_text(encoding="utf-8").splitlines()]
        blocked = {norm(t) for s in sets.values() for t, _ in s} | {norm(t) for t, _ in heldout}
        clean = [r for r in rows if norm(r["text"]) not in blocked]
        n_train = (len(clean), len(rows) - len(clean))
        print(f"train rows {n_train[0]} ({n_train[1]} removed as exact copies of held-out sentences)")
        X = enc.embed([r["text"] for r in clean])
        y = [r["labels"][0] for r in clean]
        clf = LogisticRegression(C=a.C, max_iter=3000).fit(X, y)
        head = Head(clf.classes_, clf.coef_, clf.intercept_)
        if not a.no_write:
            out = Path(a.out)
            out.parent.mkdir(parents=True, exist_ok=True)
            payload = {
                "classes": [str(c) for c in clf.classes_],
                "coef": [[round(float(x), 6) for x in row] for row in clf.coef_],
                "b": [round(float(x), 6) for x in clf.intercept_],
            }
            out.write_text(json.dumps(payload), encoding="utf-8")
            print("wrote", out, f"{out.stat().st_size / 1e3:.0f} KB")
            live = REPO / "models" / "router"
            if live.is_dir() and live.resolve() != out.parent.resolve():
                (live / "head.json").write_text(json.dumps(payload), encoding="utf-8")
                print("also updated", live / "head.json")
            head = Head(payload["classes"], payload["coef"], payload["b"])  # score what was written

    r = Router(enc, head)
    results = {}
    for name in ["test", "test2", "val", "val2"]:
        results[name] = evaluate(r, sets[name])
    pooled = {"n": 0, "exact": 0, "single": [0, 0], "chain": [0, 0], "out": [0, 0], "errors": []}
    for name in ["test", "test2"]:
        x = results[name]
        pooled["n"] += x["n"]
        pooled["exact"] += x["exact"]
        for k in ("single", "chain", "out"):
            pooled[k] = [pooled[k][0] + x[k][0], pooled[k][1] + x[k][1]]
        pooled["errors"] += x["errors"]
    results["test+test2"] = pooled
    if a.score_heldout:
        if not heldout:
            sys.exit("no data/heldout_phase2.txt")
        results["heldout_phase2"] = evaluate(r, heldout)

    lines = [
        f"### {a.tag}" + (f" (train rows {n_train[0]}, head C={a.C})" if n_train else f" (head {a.eval_head})"),
        "",
        "| Set | All (exact) | Single-task | Chains | OUT only |",
        "|---|---|---|---|---|",
    ]
    for name, res in results.items():
        report(name, res, lines)
    lines += ["", "Confusable pairs, single-task errors (first -> second / second -> first), test+test2"
              + (" and heldout_phase2:" if "heldout_phase2" in results else ":"), ""]
    lines += ["| Pair | test+test2 | heldout_phase2 |", "|---|---|---|"]
    pe = pair_errors(pooled)
    ph = pair_errors(results["heldout_phase2"]) if "heldout_phase2" in results else None
    for k, v in pe.items():
        h = f"{ph[k][0]} / {ph[k][1]}" if ph else "-"
        lines.append(f"| {k} | {v[0]} / {v[1]} | {h} |")
    text = "\n".join(lines)
    print(text)
    if a.errors:
        for name, res in results.items():
            if name in ("test+test2",):
                continue
            print(f"\n-- errors {name}")
            for t, w, g in res["errors"]:
                print(f"  {w} -> {g}  {t}")
    if not a.no_write:
        (HERE / f"metrics_{a.tag}.md").write_text(text + "\n", encoding="utf-8")
        summary = {
            name: {k: v for k, v in res.items() if k != "errors"} | {"errors": res["errors"]}
            for name, res in results.items()
        }
        (HERE / f"metrics_{a.tag}.json").write_text(json.dumps(summary, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
