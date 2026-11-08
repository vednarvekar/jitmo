"""Fine-tune GLiNER-small on generated + hand data. Eval is done separately with score_ft.py."""
import re, random, sys
from extract2 import load, LABELS
from gliner import GLiNER
random.seed(1)
LAB = LABELS["a"]                                  # kind -> label text (GLiNER kinds only)
TOK = re.compile(r"\w+(?:[-_']\w+)*|\S")
def build(rows):
    data = []
    for rid, text, gold in rows:
        toks = [(m.group(), m.start(), m.end()) for m in TOK.finditer(text)]
        used, ner = [], []
        for kind, span in gold:
            if kind not in LAB: continue
            i = text.lower().find(span.lower())
            while i >= 0 and any(i < b and i + len(span) > a for a, b in used):
                i = text.lower().find(span.lower(), i + 1)
            if i < 0: continue
            used.append((i, i + len(span)))
            idx = [k for k, (_, s, e) in enumerate(toks) if s >= i and e <= i + len(span)]
            if idx: ner.append([idx[0], idx[-1], LAB[kind]])
        data.append({"tokenized_text": [t for t, _, _ in toks], "ner": ner})
    return data
train = []
for f in ["train_gen.txt", "val.txt", "val2.txt", "test.txt"]:    # doc4 + fresh stay unseen
    train += build(load(f))
random.shuffle(train)
print("train examples", len(train), flush=True)
model = GLiNER.from_pretrained("urchade/gliner_small-v2.1")
model.train_model(train, train[:50], output_dir="ft_out", learning_rate=2e-5, others_lr=5e-5,
    weight_decay=0.01, others_weight_decay=0.01, lr_scheduler_type="linear", warmup_ratio=0.1,
    per_device_train_batch_size=8, per_device_eval_batch_size=8, num_train_epochs=3,
    save_steps=10**6, save_total_limit=1, logging_steps=50, use_cpu=True, report_to="none", dataloader_num_workers=0, max_steps=750)
model.save_pretrained("models/gliner_small_ft")
print("saved", flush=True)
