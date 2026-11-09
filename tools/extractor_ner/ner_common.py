"""Shared helpers: bracket-format parsing, BIO labelling over word-piece tokens, span decoding, and the
scoring convention of the SP2 spike (sentence-exact + span F1, articles / 's / case ignored).

Blind-set discipline: `fresh_split()` returns DEV (odd-numbered data lines) and TEST (even-numbered).
`score()` only returns aggregates; `show_errors()` refuses to run on anything but DEV rows.
"""
import re
from collections import Counter
from pathlib import Path

BR = re.compile(r"\[(\w+)\|(.*?)\]")
TYPES = ["app", "contact", "date_time", "duration", "event", "file", "folder", "media", "message", "number", "place", "query", "setting", "website"]
LABELS = ["O"] + [f"{p}-{t}" for t in TYPES for p in ("B", "I")]
L2I = {l: i for i, l in enumerate(LABELS)}

REPO = Path(__file__).resolve().parents[2]
FRESH = REPO / "spikes/sp2-extractor/data/fresh.txt"
if not FRESH.exists():  # worktree without the untracked spike files: fall back to the main checkout
    FRESH = Path("V:/Code/jitmo/spikes/sp2-extractor/data/fresh.txt")


def parse_line(line):
    """'ID|text with [t|span]' -> (id, plain text, [(type, start, end)])."""
    rid, ann = line.split("|", 1)
    text, spans, pos = "", [], 0
    for m in BR.finditer(ann):
        text += ann[pos:m.start()]
        s = len(text)
        text += m.group(2)
        spans.append((m.group(1), s, len(text)))
        pos = m.end()
    text += ann[pos:]
    return rid, text, spans


def load_rows(path):
    return [parse_line(l) for l in Path(path).read_text(encoding="utf-8").splitlines() if "|" in l]


def fresh_split():
    """DEV = odd-numbered data lines (1st, 3rd, ...), TEST = even-numbered."""
    rows = load_rows(FRESH)
    dev = [r for i, r in enumerate(rows) if i % 2 == 0]
    test = [r for i, r in enumerate(rows) if i % 2 == 1]
    return dev, test


def norm(s):
    s = s.lower().strip(" .?!,")
    s = re.sub(r"'s$", "", s)
    for _ in range(2):
        s = re.sub(r"^(?:at|on|the|my|this|these|that|a|an|some|for)\s+", "", s)
    return s.strip()


def bag(items):
    return Counter((k, norm(t)) for k, t in items)


def score(rows, predict):
    """predict(text) -> [(type, span text)]. Returns aggregate numbers only."""
    tp = fp = fn = perfect = 0
    for _, text, gold in rows:
        pred = bag(predict(text))
        want = bag((k, text[s:e]) for k, s, e in gold)
        hit = pred & want
        a, b, c = sum(hit.values()), sum((pred - want).values()), sum((want - pred).values())
        tp, fp, fn = tp + a, fp + b, fn + c
        perfect += b == 0 and c == 0
    p, r = tp / max(tp + fp, 1), tp / max(tp + fn, 1)
    f1 = 2 * p * r / max(p + r, 1e-9)
    return dict(acc=perfect / len(rows), perfect=perfect, n=len(rows), f1=f1, p=p, r=r)


def show_errors(rows, predict, dev_rows):
    """Print misses, but only for DEV rows (identity-checked)."""
    ids = {r[0] for r in dev_rows}
    for rid, text, gold in rows:
        assert rid in ids, "show_errors is for DEV only"
        pred = bag(predict(text))
        want = bag((k, text[s:e]) for k, s, e in gold)
        if pred != want:
            print(f"  {rid} {text!r}\n     missed {sorted((want - pred).elements())} extra {sorted((pred - want).elements())}")


def fmt(name, s):
    return f"{name:34s} sentence-exact {s['perfect']:3d}/{s['n']:3d} = {100 * s['acc']:5.1f}%  span F1 {100 * s['f1']:5.1f}% (P {100 * s['p']:.1f} / R {100 * s['r']:.1f})"


# ---------------------------------------------------------------- BIO over word pieces
def bio_labels(offsets, spans):
    """offsets: [(a, b)] per token ((0,0) for special tokens). Returns a label id per token (-100 for special)."""
    out = []
    started = set()
    for a, b in offsets:
        if a == b:
            out.append(-100)
            continue
        lab = "O"
        for si, (t, s, e) in enumerate(spans):
            if s <= a < e or (a < s < b):
                if si in started:
                    lab = f"I-{t}"
                else:
                    lab = f"B-{t}"
                    started.add(si)
                break
        out.append(L2I[lab])
    return out


def decode(text, offsets, label_ids, scores=None, min_score=0.0):
    """Token labels -> [(type, start, end, score)] (char offsets, snapped to whole words)."""
    spans = []
    cur = None
    for i, ((a, b), li) in enumerate(zip(offsets, label_ids)):
        if a == b:
            continue
        lab = LABELS[li]
        sc = 1.0 if scores is None else float(scores[i])
        if lab == "O":
            if cur:
                spans.append(cur)
                cur = None
            continue
        pre, t = lab.split("-", 1)
        if pre == "B" or cur is None or cur[0] != t:
            if cur:
                spans.append(cur)
            cur = [t, a, b, [sc]]
        else:
            cur[2] = b
            cur[3].append(sc)
    if cur:
        spans.append(cur)
    out = []
    for t, a, b, scs in spans:
        while a > 0 and text[a - 1].isalnum() and text[a].isalnum():
            a -= 1
        while b < len(text) and text[b].isalnum() and text[b - 1].isalnum():
            b += 1
        s = sum(scs) / len(scs)
        if s >= min_score:
            out.append((t, a, b, s))
    return out
