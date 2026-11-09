"""Variant A1: a BIO head on top of the router's FROZEN int8 MiniLM encoder (shared encoder, near-zero extra memory).

The router runs `models/router/model_int8.onnx` for every request and already has the per-token hidden states
([1, n, 384]) in hand; this trains a small head on exactly those features (int8 outputs, not fp32 ones), so
what is trained is what would be deployed.

    python -I train_shared_head.py --router ../../models/router --train data/synth_train.txt --head gru \
        --out work/head_gru --epochs 12

Writes head.onnx (input `hidden` [1, n, 384] float32 -> `logits` [1, n, 29]) and labels.json. Prints DEV/TEST
aggregates only.
"""
import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import numpy as np  # noqa: E402
import onnxruntime as ort  # noqa: E402
import torch  # noqa: E402
from tokenizers import Tokenizer  # noqa: E402

import ner_common as C  # noqa: E402


class Head(torch.nn.Module):
    def __init__(self, kind, d=384, hid=128, n=len(C.LABELS)):
        super().__init__()
        self.kind = kind
        if kind == "linear":
            self.out = torch.nn.Linear(d, n)
        elif kind == "mlp":
            self.net = torch.nn.Sequential(torch.nn.Linear(d, 256), torch.nn.GELU(), torch.nn.Dropout(0.1), torch.nn.Linear(256, n))
        elif kind == "conv":  # window of 5 tokens
            self.conv = torch.nn.Conv1d(d, 192, 5, padding=2)
            self.out = torch.nn.Linear(192, n)
        elif kind == "gru":
            self.proj = torch.nn.Linear(d, 192)
            self.rnn = torch.nn.GRU(192, hid, batch_first=True, bidirectional=True)
            self.out = torch.nn.Linear(2 * hid, n)

    def forward(self, h):
        if self.kind == "linear":
            return self.out(h)
        if self.kind == "mlp":
            return self.net(h)
        if self.kind == "conv":
            x = torch.nn.functional.gelu(self.conv(h.transpose(1, 2))).transpose(1, 2)
            return self.out(x)
        x = torch.nn.functional.gelu(self.proj(h))
        x, _ = self.rnn(x)
        return self.out(x)


class Features:
    def __init__(self, router_dir):
        d = Path(router_dir)
        so = ort.SessionOptions()
        so.intra_op_num_threads = 4
        self.sess = ort.InferenceSession(str(d / "model_int8.onnx"), so, providers=["CPUExecutionProvider"])
        self.tok = Tokenizer.from_file(str(d / "tokenizer.json"))
        self.tok.enable_truncation(max_length=64)

    def __call__(self, text):
        enc = self.tok.encode(text)
        n = len(enc.ids)
        feed = {
            "input_ids": np.array([enc.ids], dtype=np.int64),
            "attention_mask": np.array([enc.attention_mask], dtype=np.int64),
            "token_type_ids": np.zeros((1, n), dtype=np.int64),
        }
        return self.sess.run(None, feed)[0][0].astype(np.float32), enc.offsets


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--router", required=True)
    ap.add_argument("--train", nargs="+", required=True)
    ap.add_argument("--head", default="gru", choices=["linear", "mlp", "conv", "gru"])
    ap.add_argument("--out", required=True)
    ap.add_argument("--epochs", type=int, default=12)
    ap.add_argument("--lr", type=float, default=2e-3)
    ap.add_argument("--bs", type=int, default=64)
    a = ap.parse_args()
    torch.manual_seed(0)
    feats = Features(a.router)
    rows = []
    for f in a.train:
        rows += C.load_rows(f)
    t0 = time.time()
    data = []
    for _, text, spans in rows:
        h, off = feats(text)
        data.append((torch.from_numpy(h), torch.tensor(C.bio_labels(off, spans))))
    print(f"features for {len(data)} sentences in {time.time() - t0:.0f}s", flush=True)
    head = Head(a.head)
    opt = torch.optim.AdamW(head.parameters(), lr=a.lr, weight_decay=0.01)
    steps = a.epochs * ((len(data) + a.bs - 1) // a.bs)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=a.lr, total_steps=steps)
    t0 = time.time()
    for ep in range(a.epochs):
        perm = torch.randperm(len(data)).tolist()
        tot = 0.0
        head.train()
        for i in range(0, len(perm), a.bs):
            batch = [data[j] for j in perm[i : i + a.bs]]
            n = max(len(h) for h, _ in batch)
            H = torch.zeros(len(batch), n, 384)
            Y = torch.full((len(batch), n), -100, dtype=torch.long)
            for k, (h, y) in enumerate(batch):
                H[k, : len(h)] = h
                Y[k, : len(y)] = y
            loss = torch.nn.functional.cross_entropy(head(H).reshape(-1, len(C.LABELS)), Y.reshape(-1), ignore_index=-100)
            loss.backward()
            opt.step()
            sched.step()
            opt.zero_grad()
            tot += loss.item() * len(batch)
        print(f"epoch {ep + 1}/{a.epochs} loss {tot / len(data):.4f}", flush=True)
    train_s = time.time() - t0
    head.eval()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    dummy = torch.zeros(1, 12, 384)
    torch.onnx.export(head, (dummy,), str(out / "head.onnx"), input_names=["hidden"], output_names=["logits"],
                      dynamic_axes={"hidden": {0: "b", 1: "n"}, "logits": {0: "b", 1: "n"}}, opset_version=17, dynamo=False)
    (out / "labels.json").write_text(json.dumps(C.LABELS))
    hs = ort.InferenceSession(str(out / "head.onnx"), providers=["CPUExecutionProvider"])

    def predict(text):
        h, off = feats(text)
        lg = hs.run(None, {"hidden": h[None]})[0][0]
        p = np.exp(lg - lg.max(-1, keepdims=True))
        p /= p.sum(-1, keepdims=True)
        return [(t, text[s:e]) for t, s, e, _ in C.decode(text, off, p.argmax(-1).tolist(), p.max(-1).tolist())]

    dev, test = C.fresh_split()
    print(C.fmt(f"DEV  shared encoder + {a.head} head", C.score(dev, predict)))
    print(C.fmt(f"TEST shared encoder + {a.head} head", C.score(test, predict)))
    print(f"head params {sum(p.numel() for p in head.parameters()) / 1e3:.0f}K, head file {(out / 'head.onnx').stat().st_size / 1e3:.0f} KB, training {train_s:.0f}s")


if __name__ == "__main__":
    main()
