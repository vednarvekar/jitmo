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
