"""Fine-tune the tool-picker on this PC's CPU (same recipe as finetune_router.ipynb, no GPU needed).

    python training/train_local.py --base unsloth/gemma-3-270m-it --out V:/jitmo-sandbox/train/run-gemma

Writes progress to <out>/train.log (one line per few steps) and the merged model to <out>/merged.
Never reads testset/heldout.txt.
"""
import argparse, json, os, random, sys, time

import torch
from datasets import Dataset
from peft import LoraConfig
from transformers import AutoModelForCausalLM, AutoTokenizer, TrainerCallback
from trl import SFTConfig, SFTTrainer

HERE = os.path.dirname(os.path.abspath(__file__))

p = argparse.ArgumentParser()
p.add_argument("--base", default="unsloth/gemma-3-270m-it")
p.add_argument("--out", required=True)
p.add_argument("--epochs", type=float, default=3)
p.add_argument("--max-steps", type=int, default=-1, help="stop early (timing trial)")
p.add_argument("--lr", type=float, default=2e-4)
p.add_argument("--rank", type=int, default=16)
p.add_argument("--batch", type=int, default=4)
p.add_argument("--accum", type=int, default=4, help="batch x accum = 16, as in the notebook")
p.add_argument("--threads", type=int, default=os.cpu_count() // 2 or 1)
p.add_argument("--no-eval", action="store_true")
a = p.parse_args()

torch.set_num_threads(a.threads)
os.makedirs(a.out, exist_ok=True)
LOG = os.path.join(a.out, "train.log")


def log(msg):
    line = f"{time.strftime('%H:%M:%S')}  {msg}"
    print(line, flush=True)
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(line + "\n")


files = [os.path.join(HERE, "router_train.jsonl"), os.path.join(HERE, "router_train_args.jsonl")]
rows = [json.loads(l) for f in files for l in open(f, encoding="utf-8") if l.strip()]
random.Random(1).shuffle(rows)
data = [{"prompt": r["messages"][:2], "completion": r["messages"][2:]} for r in rows]
split = int(len(data) * 0.95)
train_ds, eval_ds = Dataset.from_list(data[:split]), Dataset.from_list(data[split:])
log(f"base {a.base}; {len(train_ds)} train, {len(eval_ds)} eval; {a.threads} threads")

tok = AutoTokenizer.from_pretrained(a.base)
model = AutoModelForCausalLM.from_pretrained(a.base, dtype=torch.float32)
lora = LoraConfig(r=a.rank, lora_alpha=a.rank * 2, lora_dropout=0.05, task_type="CAUSAL_LM",
                  target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"])
steps_per_epoch = -(-len(train_ds) // (a.batch * a.accum))


class FileLog(TrainerCallback):
    def __init__(self):
        self.t0 = time.time()

    def on_log(self, args, state, control, logs=None, **kw):
        done, total = state.global_step, state.max_steps
        el = time.time() - self.t0
        eta = el / done * (total - done) if done else 0
        log(f"step {done}/{total}  epoch {(state.epoch or 0):.2f}  {logs}  elapsed {el/60:.1f} min  left ~{eta/60:.0f} min")


cfg = SFTConfig(output_dir=os.path.join(a.out, "run"), num_train_epochs=a.epochs, max_steps=a.max_steps,
                learning_rate=a.lr, per_device_train_batch_size=a.batch, per_device_eval_batch_size=a.batch,
                gradient_accumulation_steps=a.accum, logging_steps=5,
                eval_strategy="no" if a.no_eval else "epoch", save_strategy="no", report_to="none",
                max_length=512, completion_only_loss=True, lr_scheduler_type="cosine",
                warmup_steps=int(0.05 * a.epochs * steps_per_epoch), use_cpu=True,
                dataloader_num_workers=0, bf16=False, fp16=False,
                # A 270M model fits easily in RAM; recomputing activations would only cost CPU time.
                gradient_checkpointing=False)
trainer = SFTTrainer(model=model, args=cfg, train_dataset=train_ds, eval_dataset=eval_ds,
                     processing_class=tok, peft_config=lora, callbacks=[FileLog()])
trainer.train()
log("training done")

if a.max_steps > 0:
    log("timing trial only: not saving")
    sys.exit(0)

# Quick check on held-back training examples (the real test is jitmo-llmbench on testset/heldout.txt).
m = trainer.model.eval()
ok, n = 0, min(60, len(eval_ds))
with torch.no_grad():
    for ex in eval_ds.select(range(n)):
        text = tok.apply_chat_template(ex["prompt"], tokenize=False, add_generation_prompt=True)
        ids = tok(text, return_tensors="pt", add_special_tokens=False)
        out = m.generate(**ids, max_new_tokens=12, do_sample=False)
        got = tok.decode(out[0][ids["input_ids"].shape[1]:], skip_special_tokens=True).strip()
        ok += got == ex["completion"][0]["content"].strip()
log(f"{ok}/{n} exact matches on held-back training examples")

merged = trainer.model.merge_and_unload()
merged.save_pretrained(os.path.join(a.out, "merged"), safe_serialization=True)
tok.save_pretrained(os.path.join(a.out, "merged"))
log(f"merged model saved to {os.path.join(a.out, 'merged')}")
