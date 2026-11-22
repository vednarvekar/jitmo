"""Fine-tune the MiniLM encoder to choose the tool, and export it in the worker's own format.

Unlike train_router.py (a frozen encoder plus a head over the spec's 50 task ids), this trains
the encoder itself, and its classes are the skill ids, so a skill needs no task id to be
chosen. Measured on testset/heldout.txt (14 Nov 2026): 84% right tools against 61% for the rules
plus the old head and 70% for a fine-tuned Gemma 3 270M.

    PY=spikes/sp2-extractor/.venv/Scripts/python.exe        # torch, transformers, onnx, onnxruntime
    $PY -I tools/router/train_tool_encoder.py --out models/tools --score-heldout

Writes into --out exactly what crates/model-worker/src/router.rs loads:
  model_int8.onnx   the fine-tuned encoder (output 0 = token vectors), int8
  tokenizer.json
  head.json         {"classes": [skill ids..., "OUT"], "coef": [[...]], "b": [...]}
The worker mean-pools the token vectors, L2-normalises and applies the head: the same as here.

Training data: training/router_train.jsonl (the request and the right tool of each row) plus
every skill's own examples. testset/heldout.txt is NEVER read for training; --score-heldout only
scores the exported int8 model on it, once, at the end.
"""
import argparse, glob, json, os, random, re, sys, time
from pathlib import Path

import numpy as np
import torch
from transformers import AutoModel, AutoTokenizer

ROOT = Path(__file__).resolve().parents[2]
OUT_CLASS = "OUT"
SCALE = 20.0  # the pooled vector has length 1; this gives the head room to be sure

ap = argparse.ArgumentParser()
ap.add_argument("--base", default="sentence-transformers/all-MiniLM-L6-v2")
ap.add_argument("--out", default=str(ROOT / "models" / "tools"))
ap.add_argument("--epochs", type=int, default=4)
ap.add_argument("--seed", type=int, default=1)
ap.add_argument("--threads", type=int, default=3)
ap.add_argument("--score-heldout", action="store_true")
ap.add_argument("--no-older", action="store_true", help="leave out tools/router/data/train.jsonl")
a = ap.parse_args()
torch.set_num_threads(a.threads)
torch.manual_seed(a.seed)
random.seed(a.seed)


def load_skills():
    skills = {}
    for f in sorted(glob.glob(str(ROOT / "skills" / "*.yaml"))):
        t = open(f, encoding="utf-8").read()
        sid = re.search(r"^id:\s*(\S+)", t, re.M).group(1)
        # Account set-up and the test stub are not requests a user makes.
        if sid.startswith("account.") or sid == "stub.send":
            continue
        title = re.search(r'^title:\s*"([^"]+)"', t, re.M)
        ex = re.search(r"^examples:\n((?:\s+-.*\n)+)", t, re.M)
        exs = [re.sub(r'^\s*-\s*"?|"?\s*$', "", l) for l in ex.group(1).splitlines()] if ex else []
        skills[sid] = (title.group(1) if title else sid, exs)
    return skills


skills = load_skills()
by_title = {v[0]: k for k, v in skills.items()}
assert len(by_title) == len(skills), "two skills share a title"

# A row of the picker's training file: numbered tool titles, the request, the right number.
# "0" means no tool fits (or the right one was left out on purpose): kept as OUT only when the
# same request never appears with a tool.
train, zero = {}, set()
for l in open(ROOT / "training" / "router_train.jsonl", encoding="utf-8"):
    m = json.loads(l)["messages"]
    listed = dict(re.findall(r"^(\d): (.+)$", m[0]["content"], re.M))
    text, ans = m[1]["content"], m[2]["content"].strip()
    if ans == "0":
        zero.add(text)
    elif listed.get(ans) in by_title:
        train[text] = by_title[listed[ans]]
for sid, (_, exs) in skills.items():
    for e in exs:
        train.setdefault(re.sub(r"[{}]", "", e), sid)
# The older router's training file: one spec task id (or OUT) a request, in other templates. A
# task id is the skill that declares it; requests with several labels (chains) are skipped.
by_task = {}
for f in sorted(glob.glob(str(ROOT / "skills" / "*.yaml"))):
    t = open(f, encoding="utf-8").read()
    task = re.search(r"^task:\s*(\S+)", t, re.M)
    sid = re.search(r"^id:\s*(\S+)", t, re.M).group(1)
    if task and sid in skills:
        by_task[task.group(1)] = sid
older = ROOT / "tools" / "router" / "data" / "train.jsonl"
added = 0
if older.exists() and not a.no_older:
    for l in open(older, encoding="utf-8"):
        r = json.loads(l)
        if len(r["labels"]) != 1 or r["text"] in train or r["text"] in zero:
            continue
        lab = r["labels"][0]
        if lab == "OUT":
            zero.add(r["text"])
            added += 1
        elif lab in by_task:
            train[r["text"]] = by_task[lab]
            added += 1
    print(f"{added} more requests from tools/router/data/train.jsonl", flush=True)
rows = [(t, s) for t, s in train.items()] + [(t, OUT_CLASS) for t in sorted(zero) if t not in train]
classes = sorted(skills) + [OUT_CLASS]
index = {c: i for i, c in enumerate(classes)}
random.shuffle(rows)


def corpus(name):
    """(request, right class) for each plain run/decline line of a testset corpus."""
    out = []
    for l in open(ROOT / "testset" / name, encoding="utf-8"):
        p = l.rstrip("\n").split("|")
        if not l.strip() or l.startswith("#") or len(p) < 4 or "&&" in p[2] or p[2].startswith("<") or not p[2].strip():
            continue
        first = p[3].split(";")[0]
        if first.startswith("run:") and first[4:].split("/")[0] in skills:
            out.append((p[2], first[4:].split("/")[0]))
        elif first == "decline":
            out.append((p[2], OUT_CLASS))
    return out


# Messy, hand-written requests (not the held-out set, and not training data): used only to set
# how sure the model says it is.
calib = [(t, s) for t, s in corpus("audit_corpus.txt") if t not in train]
# No held-out sentence may be trained on, even by accident (the files are only compared).
unseen = {t.lower() for t, _ in corpus("heldout.txt")}
fit = [(t, s) for t, s in rows if t.lower() not in unseen]
if len(fit) != len(rows):
    print(f"dropped {len(rows) - len(fit)} training requests equal to a held-out request", flush=True)
print(f"{len(fit)} training requests, {len(calib)} audit-corpus requests to set the confidence, {len(classes)} classes", flush=True)

tok = AutoTokenizer.from_pretrained(a.base)
model = AutoModel.from_pretrained(a.base)
head = torch.nn.Linear(model.config.hidden_size, len(classes))


def pooled(texts, grad):
    out = []
    for i in range(0, len(texts), 64):
        b = tok(texts[i:i + 64], padding=True, truncation=True, max_length=64, return_tensors="pt")
        with torch.set_grad_enabled(grad):
            h = model(**b).last_hidden_state
        m = b["attention_mask"].unsqueeze(-1).float()
        out.append(torch.nn.functional.normalize((h * m).sum(1) / m.sum(1), dim=1))
    return torch.cat(out)


opt = torch.optim.AdamW(
    [{"params": model.parameters(), "lr": 5e-5}, {"params": head.parameters(), "lr": 2e-3}], weight_decay=0.01
)
steps = a.epochs * (len(fit) // 32 + 1)
sched = torch.optim.lr_scheduler.LinearLR(opt, 1.0, 0.05, steps)
t0 = time.time()
for ep in range(a.epochs):
    model.train()
    random.shuffle(fit)
    total = 0.0
    for i in range(0, len(fit), 32):
        b = fit[i:i + 32]
        logits = head(pooled([t for t, _ in b], True) * SCALE)
        loss = torch.nn.functional.cross_entropy(logits, torch.tensor([index[s] for _, s in b]), label_smoothing=0.1)
        opt.zero_grad()
        loss.backward()
        opt.step()
        sched.step()
        total += loss.item()
    print(f"epoch {ep + 1}/{a.epochs}  loss {total / (len(fit) / 32):.3f}  {time.time() - t0:.0f}s", flush=True)
model.eval()

# One temperature, so that "0.8 sure" means about 80% right on messy requests. It is fitted on
# the audit corpus, not on the training templates: those look far easier than real requests, and
# a temperature fitted on them made the model much too sure of itself.
with torch.no_grad():
    cal_logits = head(pooled([t for t, _ in calib], False) * SCALE)
cal_y = torch.tensor([index[s] for _, s in calib])
temp = min(
    (x / 20 for x in range(8, 61)),
    key=lambda T: torch.nn.functional.cross_entropy(cal_logits / T, cal_y).item(),
)
cal_acc = (cal_logits.argmax(1) == cal_y).float().mean().item()
print(f"audit corpus: {100 * cal_acc:.1f}% right, temperature {temp:.2f}", flush=True)

out = Path(a.out)
out.mkdir(parents=True, exist_ok=True)
coef = (head.weight.detach() * SCALE / temp).tolist()
bias = (head.bias.detach() / temp).tolist()
json.dump(
    {"classes": classes, "coef": [[round(x, 5) for x in r] for r in coef], "b": [round(x, 5) for x in bias]},
    open(out / "head.json", "w"),
)
bt = tok.backend_tokenizer
bt.no_padding()
bt.no_truncation()
bt.save(str(out / "tokenizer.json"))


class Tokens(torch.nn.Module):
    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, input_ids, attention_mask, token_type_ids):
        return self.m(input_ids=input_ids, attention_mask=attention_mask, token_type_ids=token_type_ids).last_hidden_state


from onnxruntime.quantization import QuantType, quantize_dynamic

ids = torch.tensor([tok("remind me to call mom at 6pm")["input_ids"]])
fp32 = out / "model.onnx"
torch.onnx.export(
    Tokens(model), (ids, torch.ones_like(ids), torch.zeros_like(ids)), str(fp32),
    input_names=["input_ids", "attention_mask", "token_type_ids"], output_names=["tokens"],
    dynamic_axes={k: {0: "b", 1: "n"} for k in ["input_ids", "attention_mask", "token_type_ids", "tokens"]},
    opset_version=17, dynamo=False,
)
quantize_dynamic(str(fp32), str(out / "model_int8.onnx"), weight_type=QuantType.QInt8)
fp32.unlink()
print("wrote", ", ".join(f"{p.name} {p.stat().st_size / 1e6:.1f} MB" for p in sorted(out.iterdir())), flush=True)

if a.score_heldout:
    # The exported int8 model, scored the way the worker runs it (one request at a time).
    import onnxruntime as ort
    from tokenizers import Tokenizer

    sess = ort.InferenceSession(str(out / "model_int8.onnx"), providers=["CPUExecutionProvider"])
    rt = Tokenizer.from_file(str(out / "tokenizer.json"))
    H = json.load(open(out / "head.json"))
    W, B = np.array(H["coef"], dtype=np.float32), np.array(H["b"], dtype=np.float32)

    def probs(text):
        e = rt.encode(text)
        ids = np.array([e.ids[:128]], dtype=np.int64)
        h = sess.run(None, {"input_ids": ids, "attention_mask": np.ones_like(ids), "token_type_ids": np.zeros_like(ids)})[0][0]
        v = h.mean(0)
        v = v / max(np.linalg.norm(v), 1e-12)
        z = W @ v + B
        z = np.exp(z - z.max())
        return z / z.sum()

    cases = corpus("heldout.txt")
    t0 = time.time()
    P = [probs(t) for t, _ in cases]
    ms = 1000 * (time.time() - t0) / len(cases)
    top = [classes[int(p.argmax())] for p in P]
    conf = [float(p.max()) for p in P]
    n = len(cases)
    right = sum(t == g for t, (_, g) in zip(top, cases))
    tool = [(p, g) for p, (_, g) in zip(P, cases) if g != OUT_CLASS]
    in3 = sum(g in [classes[i] for i in np.argsort(-p) if classes[i] != OUT_CLASS][:3] for p, g in tool)
    wrong = sum(t != g and t != OUT_CLASS for t, (_, g) in zip(top, cases))
    print(f"held-out, int8: {right}/{n} = {100 * right / n:.1f}% right; right tool among the best 3: {in3}/{len(tool)} = "
          f"{100 * in3 / len(tool):.1f}%; wrong action {wrong} ({100 * wrong / n:.1f}%); {ms:.1f} ms a request")
    for th in (0.5, 0.8, 0.9):
        run = [(t, g) for t, (_, g), c in zip(top, cases, conf) if c >= th]
        ok = sum(t == g for t, g in run)
        print(f"  sure >= {th}: acts on {len(run)} ({100 * len(run) / n:.0f}%), right {100 * ok / max(1, len(run)):.1f}%, wrong {len(run) - ok}")
