# Training the tool-picker (step 5 of the low-memory brief)

Untuned 270M to 0.8B models pick the right tool among three candidates only about 40 to 55% of the time (docs/03-delivery/14). A short LoRA fine-tune on jitmo's own tool list is what should lift them past the gate (tool accuracy at least 95%, argument accuracy at least 90%, no wrong sends).

## What is here

| File | What it is |
|---|---|
| `router_train.jsonl` | 3,180 examples. System message: "Pick the tool number…" plus three numbered tool titles (the router's best two and the best word-overlap match, as the engine builds them). User: the request. Assistant: the number, `0` for none. |
| `router_train_args.jsonl` | 840 examples. System: the tool and its typed arguments. User: the request. Assistant: JSON with every argument (empty when not said). |
| `finetune_router.ipynb` | A Colab notebook: LoRA fine-tune, merge, convert to GGUF, quantise to Q4_K_M, save to Drive. Written without being run: if a cell fails, send the error text to the coding agent. |

The data is made by `cargo run --release -p jitmo-testset-runner --bin jitmo-gendata -- --out training/router_train.jsonl --worker target/release/jitmo-model-worker.exe` from hand-written templates (typos, Hinglish, casing, "please"). It never reads `testset/heldout.txt`, which must stay a fair test.

## How to run it (about 20 minutes on a free T4)

1. In Google Drive create `MyDrive/jitmo/` and upload `router_train.jsonl` and `router_train_args.jsonl`.
2. Open `finetune_router.ipynb` in Colab (File, Upload notebook, or open it from Drive). `Runtime`, `Change runtime type`, GPU.
3. In the first cell pick `BASE_MODEL` (start with `unsloth/gemma-3-270m-it`, then `Qwen/Qwen3.5-0.8B`). Run all cells. No password or token is needed for these models.
4. The tuned model lands in `MyDrive/jitmo/out/<name>-router-Q4_K_M.gguf`. Copy it to this PC, for example into `V:\jitmo-sandbox\models\`.
5. Test it (this is the real measurement):

```
target\release\jitmo-llmbench.exe --exe V:\jitmo-sandbox\llama\llama-server.exe --model V:\jitmo-sandbox\models\<name>-router-Q4_K_M.gguf --compact --mix 21 --worker target\release\jitmo-model-worker.exe --threads 3 --ctx 768 --batch 128
```

It prints tool accuracy, argument accuracy, wrong actions, load, reading and writing time and peak memory. Only a model that passes the gates in doc 15 becomes the default.
