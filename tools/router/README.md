# Router training (reproducible)

The router is a frozen int8 `all-MiniLM-L6-v2` encoder plus a logistic-regression head over 50
classes (A1..S5 and OUT). Only the head is trained here.

Run from the repo root with the existing SP1 venv (onnxruntime, tokenizers, scikit-learn, numpy):

```powershell
spikes\sp1-router\.venv\Scripts\python.exe tools\router\train_router.py --regen --tag after
```

- `--regen` runs `gen_data.py` (templates in `gen_data.py` and `gen_extra.py`, seed 7) and rewrites `data/train.jsonl`.
- Training rows equal to any sentence in `spikes/sp1-router/data/{test,test2,val,val2}.txt` or `data/heldout_phase2.txt` are dropped before fitting, so those files stay unseen.
- Encoder files are looked up in `--encoder-dir`, else `models/router/`, else `spikes/sp1-router/models/` (see `models-src/router/README.md`).
- Output: `models-src/router/head.json` (committed; also copied to `models/router/head.json` when that folder exists) and `metrics_<tag>.md/.json`.
- `--score-heldout` also scores `data/heldout_phase2.txt` (write it only after the training data is frozen, score once).
- `--eval-head PATH --encoder-dir DIR --no-write` scores an existing head (used for the "before" numbers).
- `--errors` prints every misrouted sentence.

Scoring uses the same rules as `crates/model-worker/src/router.rs`: split at "and / then / commas", keep the split only if every part has top-1 probability at least 0.8, no part is OUT and the parts differ.

Label decisions for the Phase 2 data: system volume and brightness (including "louder", "turn it up", "mute") are S1; A3 is pause, resume, skip, previous, stop playback; naming something to play is A2; a file sent by mail is F5; a plain email is E7; a reply is E5.

To use a new head in the app: `pwsh tools\router\fetch_models.ps1`.

## The fine-tuned tool encoder (`models/tools`)

`train_tool_encoder.py` trains the encoder itself (not just a head) and its classes are the skill
ids, so a skill needs no spec task id to be chosen. The worker loads it with the same code as the
older router (same three files: `model_int8.onnx`, `tokenizer.json`, `head.json`), and prefers
`models/tools` over `models/router` when both exist; delete or rename `models/tools` to go back.

```powershell
$env:HF_HUB_OFFLINE = "1"   # the base model is already in the Hugging Face cache
spikes\sp2-extractor\.venv\Scripts\python.exe -I tools\router\train_tool_encoder.py --out models\tools --score-heldout
```

- Training data: `training/router_train.jsonl` (request and right tool of each row), every
  skill's own examples, and the single-label rows of `data/train.jsonl` (task id mapped to the
  skill that declares it, `OUT` kept as out of scope). `--no-older` leaves the last one out.
- A training request equal to a held-out request is dropped. `testset/heldout.txt` is otherwise
  only scored, at the end, on the exported int8 file.
- Confidence: one temperature fitted on `testset/audit_corpus.txt` (messy hand-written requests
  that are neither training data nor the held-out set), so that "0.8 sure" is close to 80% right.
  Fitting it on the training templates made the model far too sure of itself.
- Nothing under `models/` is committed; the script reproduces it in a few minutes on a CPU.
