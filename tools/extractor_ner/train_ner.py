"""Fine-tune a small encoder as a BIO token classifier on the synthetic corpus (CPU).

    python -I train_ner.py --base sentence-transformers/all-MiniLM-L6-v2 --train data/synth_train.txt \
        --out models_tmp/minilm_l6 --epochs 5

Prints only aggregate scores for the blind-set DEV/TEST halves (see ner_common).
"""
import argparse
import random
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import numpy as np  # noqa: E402
import torch  # noqa: E402
from transformers import AutoModelForTokenClassification, AutoTokenizer, get_linear_schedule_with_warmup  # noqa: E402

import ner_common as C  # noqa: E402


def encode_rows(tok, rows, max_len=64):
    items = []
    for _, text, spans in rows:
        enc = tok(text, return_offsets_mapping=True, truncation=True, max_length=max_len)
        items.append((enc["input_ids"], C.bio_labels(enc["offset_mapping"], spans)))
    return items


def collate(batch, pad_id):
    n = max(len(i) for i, _ in batch)
    ids = torch.full((len(batch), n), pad_id, dtype=torch.long)
    lab = torch.full((len(batch), n), -100, dtype=torch.long)
    mask = torch.zeros((len(batch), n), dtype=torch.long)
    for k, (i, l) in enumerate(batch):
        ids[k, : len(i)] = torch.tensor(i)
        lab[k, : len(l)] = torch.tensor(l)
        mask[k, : len(i)] = 1
    return ids, mask, lab


class Predictor:
    def __init__(self, model, tok):
        self.model, self.tok = model, tok

    @torch.no_grad()
    def __call__(self, text):
        enc = self.tok(text, return_offsets_mapping=True, truncation=True, max_length=64, return_tensors="pt")
        off = enc.pop("offset_mapping")[0].tolist()
        logits = self.model(**enc).logits[0]
        probs = logits.softmax(-1)
        ids = probs.argmax(-1)
        sc = probs.max(-1).values
        return [(t, text[a:b]) for t, a, b, _ in C.decode(text, off, ids.tolist(), sc.tolist())]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--tokenizer", default=None, help="tokenizer repo if the base has none (bert-tiny: bert-base-uncased)")
    ap.add_argument("--train", nargs="+", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--epochs", type=int, default=5)
    ap.add_argument("--lr", type=float, default=1e-4)
    ap.add_argument("--bs", type=int, default=32)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--threads", type=int, default=6)
    ap.add_argument("--freeze-encoder", action="store_true")
    ap.add_argument("--layers", type=int, default=0, help="keep only the first N encoder layers (0 = all)")
    a = ap.parse_args()
    random.seed(a.seed)
    torch.manual_seed(a.seed)
    torch.set_num_threads(a.threads)

    tok = AutoTokenizer.from_pretrained(a.tokenizer or a.base)
    model = AutoModelForTokenClassification.from_pretrained(
        a.base, num_labels=len(C.LABELS), id2label=dict(enumerate(C.LABELS)), label2id=C.L2I
    )
    if a.layers:
        enc = model.base_model.encoder
        enc.layer = enc.layer[: a.layers]
        model.config.num_hidden_layers = a.layers
    if a.freeze_encoder:
        for p in model.base_model.parameters():
            p.requires_grad = False
    rows = []
    for f in a.train:
        rows += C.load_rows(f)
    items = encode_rows(tok, rows)
    print(f"{len(items)} training sentences, {sum(p.numel() for p in model.parameters()) / 1e6:.1f}M params")
    opt = torch.optim.AdamW([p for p in model.parameters() if p.requires_grad], lr=a.lr, weight_decay=0.01)
    steps = a.epochs * ((len(items) + a.bs - 1) // a.bs)
    sched = get_linear_schedule_with_warmup(opt, int(0.06 * steps), steps)
    pad = tok.pad_token_id
    t0 = time.time()
    model.train()
    for ep in range(a.epochs):
        random.shuffle(items)
        tot = 0.0
        for i in range(0, len(items), a.bs):
            ids, mask, lab = collate(items[i : i + a.bs], pad)
            out = model(input_ids=ids, attention_mask=mask, labels=lab)
            out.loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()
            sched.step()
            opt.zero_grad()
            tot += out.loss.item() * len(ids)
        print(f"epoch {ep + 1}/{a.epochs} loss {tot / len(items):.4f}  ({time.time() - t0:.0f}s)", flush=True)
    train_s = time.time() - t0
    model.eval()
    Path(a.out).mkdir(parents=True, exist_ok=True)
    model.save_pretrained(a.out)
    tok.save_pretrained(a.out)
    pred = Predictor(model, tok)
    dev, test = C.fresh_split()
    print(C.fmt("DEV  (fp32 torch, model alone)", C.score(dev, pred)))
    print(C.fmt("TEST (fp32 torch, model alone)", C.score(test, pred)))
    print(f"training time {train_s:.0f} s")


if __name__ == "__main__":
    main()
