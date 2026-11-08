# SP2 result: can a tiny model pull out names, files and times from casual wording?

## Run 2 (8 Nov 2026, Windows): **FAIL on a fresh held-out set (62/100 = 62%); the rules overfit. ONNX int8 is viable for speed.**

### What was done
- Windows venv (Python 3.12 via uv; torch, gliner, onnxruntime). Rules patched for every gap and over-reach listed in run 1, in `extract2.py` (replayable from `data/extract_v1_baseline.py.txt` with `patch1.py` + `patch2.py` plus one small edit; the original `extract.py` was overwritten back to the old version by something else during the session, so the live code is `extract2.py`; a frozen copy is `data/extract2_FROZEN.py.txt`).
- Patches: family words only count as a contact as an addressee/owner (to/with/from/tell/'s) and never inside a message; "1 hr" is no longer HR; nothing inside a message span is an entity (fixes dates and names inside messages); "put on <artist>" and "put me in X" are not events; file nouns inside "<Name> folder" suppressed; lowercase names after "mail/email" and "send <name> the ..."; more apps (Slack, Zoom, Telegram, ...) and sites (Myntra, Zomato, ...); a generic "open <Word>" / GLiNER app-site fallback for unlisted apps; many more file nouns (salary slip, lease, passport scan, tax return, boarding pass, ...) incl. "yesterday's screenshot"; dates "two weeks ago", "the 1st", "June 5"; numbers in "N between M" / "N times M"; "quiet mode" and similar settings; "call mom and tell Neha ..." (tell after a conjunction); trailing "please" after a song; "Open MKBHD's latest video" creator fix; "capital of X" place prefix.
- Tuning used ONLY doc4 T01-T64, val (60) and `val2.txt` (40 new sentences I wrote to exercise the listed gaps; also tuning data). The old `test.txt` is burned: kept as a regression set and reported as tuned-on (I also read it during the session, and the run-1 miss list came from it).
- Then I froze the rules and wrote `data/fresh.txt` (100 new casual sentences, same `[label|span]` format, written AFTER the freeze, varied phrasing: "drop X a text", "lookup", "throw on", "get me to", "ping me to ...", unseen apps/sites such as Obsidian/Postman/Goodreads/Airbnb). Scored once, no changes afterwards (`fresh_run1.log`).

### Numbers (every span right in the sentence; threshold 0.5, PyTorch)
| Set | Every detail right | Span F1 (P / R) | Status |
|---|---|---|---|
| doc4 T01-T64 | 64/64 = 100% (was 53/64) | 100% | tuned on, not blind |
| val | 60/60 | 100% | tuned on |
| val2 (40 gap-targeted) | 40/40 | 100% | tuned on |
| old test (100) | 94/100 = 94% (was 83) | 96% (96 / 96) | burned, regression only |
| **fresh held-out (100)** | **62/100 = 62%** | **78% (83 / 74)** | **blind, scored once** |
| fresh, GLiNER alone, no rules | 21/100 | 41% | |

The doc4 and old-test gains are mostly the rules memorising those sets: the same rule set loses 32 points on sentences it has not seen. Per type on fresh (TP/FP/FN): query 1/1/6, media 3/2/5, message 7/2/6, event 4/4/4, file 10/5/5, date_time 20/6/6, contact 27/0/3, app 15/2/2, setting 7/0/0, duration 8/0/1, number 8/0/2, website 3/2/2. Closed lists work (setting, contact when the sentence shape is known, app); open-ended spans fail (query, media, message, event, file).

Main failure families on the fresh set:
1. The sentence frame was not in the rule list: "lookup X", "google X", "find videos about X", "reply to X with <msg>", "drop X a text, <msg>", "whatsapp dad <msg>", "ping me to X", "remind me tonight to X", "put X on Spotify", "cancel yoga", "say no to the all hands".
2. Artist/creator found only via a few frames ("newest thing from Fireship", "Shape of You by Ed Sheeran").
3. Unlisted apps/sites behind an unusual verb ("get X running", "get me to X").
4. Bare hours and ranges ("tomorrow at 10", "Thursday at 4", "last summer", "the next hour", "15%" after "at").
5. Noun-phrase boundaries for files ("all my invoices", "photo of the sunset", "design mockup", "today's meeting notes").
6. Over-reach: "sun" read as a date, "laptop" as an event, "LinkedIn" as a website when it is a sender.
A few of these are annotation-convention disagreements, which is itself a finding: same-author conventions drift even inside one project.

### Latency and memory (4 threads, CPU only, one request at a time, after warm-up; machine shared with another agent's runs: system CPU 70-90% during timing, so numbers are pessimistic; each measured twice)
| | model call | model + rules | RSS after load | model files |
|---|---|---|---|---|
| PyTorch fp32 (urchade/gliner_small-v2.1) | mean 157-284 ms | mean 172-187 ms, p95 ~215-226 | 1.34 GB (1.15-1.3 GB steady) | 611 MB |
| **ONNX int8** (onnx-community/gliner_small-v2.1, `model_int8.onnx`) | **mean 29-30 ms** | **mean 29 ms, p95 34** | **710 MB** (still imports torch + transformers via the `gliner` package) | 183 MB |

ONNX int8 is about 6x faster and roughly halves RSS. A runtime that does not import torch (custom span decoding over onnxruntime + tokenizers, as in SP1) should land around 300-400 MB (not measured). The 183 MB file is fine for the size budget.

### Accuracy cost of int8 (frozen rules; sentences fully right)
| Set | PyTorch th 0.5 | int8 th 0.5 | int8 th 0.3 |
|---|---|---|---|
| doc4 (tuned, /64) | 64 | 61 | 63 |
| val + val2 (tuned, /100) | 100 | 96 | 100 |
| old test (burned, /100) | 94 | 83 | 94 |
| **fresh held-out (/100)** | **62** | **52** | **59** |

int8 lowers GLiNER's confidence scores, so recall falls at the 0.5 threshold. Threshold 0.3 was chosen on the tuning sets only (163/164 vs 157/164) and brings int8 within 3 points of PyTorch on fresh (59 vs 62, inside the +/-9 point noise). So int8 at threshold 0.3 is viable; its accuracy loss is small next to the rules gap.

### Verdict and recommendation
- **SP2 does not pass.** 62% on blind sentences vs the 85% target. The 100% on doc4 is not a product number.
- **Rules alone are not the route.** A day of targeted rule work moved doc4 from 83% to 100% and the burned test from 83% to 94%, yet a fresh sample reaches only 62%: every new phrasing opens a new hole, and GLiNER alone (21%) cannot fill open-ended spans (message, query, media, event, file). More rules will keep inflating tuned sets without generalising.
- **Recommend a fine-tune** (doc 6.1 trigger is hit): fine-tune GLiNER-small (or a similar 100-300M token classifier) on a few thousand LLM-paraphrased, hand-checked labelled requests covering the T01-T64 schema (app, contact, file, folder, media, website, query, message, place, event, setting, plus date/time/duration/number); keep the date/time/duration/number/setting rules as post-processing (they generalise: roughly 90%+ on fresh); use runtime gazetteers from the real contact and app lists. Export the fine-tuned model to ONNX int8 (works, ~30 ms).
- Alternative if the fine-tune is deferred: let the planner LLM (SP3) emit the slots, with rules only for time/number/setting. Not measured here.
- `fresh.txt` is now burned too (scored once, failures read): any further score needs another blind set.

### Caveats
- One author for the rules and for all annotated sets; conventions may differ from real users and from what downstream skills need (e.g. a date inside "yesterday's screenshot"; an event-with-person kept whole). 100 sentences is +/-9 points of noise. Fresh-set difficulty is my judgement.
- I read old `test.txt` during the session, so no patch was blind to it.
- Timing was taken on a loaded machine; re-measure on the 8 GB target laptop. RSS includes the torch/transformers libraries that `gliner` imports even for ONNX.
- Files: `extract2.py`, `patch1.py`, `patch2.py`, `tune.py`, `bench2.py`, `score_fresh.py`, `sweep_int8.py`; logs `fresh_run1.log`, `bench_torch.log`, `bench_torch2.log`, `bench_onnx.log`; models in `hf_cache/` (PyTorch weights) and `models/gliner_small_onnx/`.

---

# History: run 1 (7 Nov 2026, WSL)


**Verdict: FAIL** (83% vs the 85% target; the memory/latency numbers also miss the product budget). Run 7 Nov 2026.

## Setup
- GLiNER small v2.1 (zero-shot, threshold 0.5, label set "a") plus hand-written rules for dates, times, durations, numbers, apps/sites/settings, file names, and sentence shapes ("tell X <msg>", "play X", "add X on <day>", "search for X", ...). Code: `extract.py`; data in `data/`.
- Data (annotated by hand, `[label|span]`): `doc4.txt` = T01-T64 (64), `val.txt` = 60 (used for all tuning), `test.txt` = 100 fresh casual sentences (scored once, after tuning was frozen). A sentence counts only if every span (and no extra span) matches; articles, "'s" and case are ignored.
- Everything on val: model (small beat medium: 78 vs 75 F1 at the best threshold), label wording (a beat b), threshold (0.5), all rules.

## Numbers
| Set | Every detail right | Span F1 (P / R) |
|---|---|---|
| Val (tuned on, so inflated) | 60/60 | 100% |
| **doc4 T01-T64** | **53/64 = 82.8%** | 92% (90 / 94) |
| **Held-out test** | **83/100 = 83.0%** | 92% (94 / 91) |
| doc4 + test combined | 136/164 = 82.9% | 92% |
| GLiNER alone, no rules | 19/64 doc4, 21/100 test (about 30%) | 38-41% |

Rules reach 100% on val only because they were written against val's errors; on unseen text they generalise to about 83%. GLiNER alone (30%) is nowhere near, so the rules do most of the work and the model mostly supplies person names.

Speed and memory (one process, 4 threads, CPU only, machine heavily loaded by other agents, load avg about 12): mean ~1.0 s, median ~0.9 s, p95 ~2.2 s per request; process RSS 1.6 GB (2.3 GB in the full eval run). These are pessimistic because of the shared box, but 1.6 GB RSS for a PyTorch process is far above a lean budget (doc 5 SP8: idle <300 MB, peak <2 GB for the whole pipeline) and 1 s misses the 0.5 s start target. Needs re-measuring on the 8 GB test laptop, ideally via ONNX/int8.

## Where the misses are (test + doc4, both below 85%)
- Rule over-reach (about a third): the family-word list fires inside messages ("call mom" -> contact, "1 hr" -> contact HR); "put on <artist>" read as an event; a date inside a quoted message ("saying ... tonight") extracted; file nouns inside folder names ("Receipts folder").
- Rule gaps: lowercase names after "mail" ("mail rahul the ..."), uncommon apps/sites (Slack, Myntra), unlisted file nouns (salary slip, lease), "two weeks ago", "the 1st", "N between M", "yesterday's screenshot", "quiet mode".
- GLiNER: "capital of Canada" as a place, "Zoya" as media, "Taylor Swift" as event.
Most are fixable in a day or two with more rules, but I did not touch them after scoring (tune-on-val-only rule).

## Decision
Do not accept SP2 as passed. Take the doc 5 fallback: **add rules for common patterns** (the list above), then re-score on a *new* held-out set of ~100 (this test set is now burned). Expect to land around 88-92%, but unproven. Alongside: swap in a gazetteer from the real contact list and app list at runtime (closes the name/app gaps without ML), consider ONNX int8 for memory/latency, and if the rule route stalls under 85% try a bigger extractor (medium was not better here) or a small fine-tune on a few hundred labelled requests. Doc 6.1's "SP2 stays under 95%" trigger is already hit, so a fine-tuning plan should be prepared either way.

## Caveats
- Annotation was done by one author (the same agent that wrote rules), so conventions match the rules; real users' phrasing will be harder. I read doc 4 while writing rules, so doc4 is not fully blind; the test set is.
- Labelling inconsistencies I found were fixed on val/test before scoring (e.g. "do not disturb" always a setting, "event with person" kept whole).
- Sample sizes are small: 100 sentences gives roughly +/-7 points of noise.

## Round 3 (8 Nov): fine-tune on generated data - still FAIL
- `gen_train.py` made 1,800 template sentences (`data/train_gen.txt`); `finetune.py` trained GLiNER-small on them plus val, val2 and test (1,940 examples, 750 steps, CPU, ~25 min). doc4 and fresh were never trained on. Model: `models/gliner_small_ft`; scoring: `score_ft.py`.
- Result with the frozen rules on top (threshold 0.3-0.5): **doc4 55/64 = 86%** (rules were tuned on doc4, so not blind), **fresh 63-64/100** (was 62). Fine-tuning gave about +2 points. Final training loss stayed high (2.9), so the model learned little from template data.
- Misses on fresh are mostly: annotation convention disputes (e.g. "Shape of You by Ed Sheeran" as one or two spans, "next hour" as duration), unseen apps/artists/senders (Obsidian, Pritam, Fireship), and message-text boundaries (commas inside messages).
- Conclusion: templates do not teach real phrasing. What helps is real labelled requests with one fixed convention, plus runtime gazetteers (contacts, installed apps, file index) for names. For V1's 22 offline tasks most slots come from closed lists (apps, settings, folders), which the rules already handle (doc4 86%).
- Decision: keep rules + GLiNER-small for V1, add runtime gazetteers, and collect real usage logs for a later fine-tune on real data. Do not spend more time on synthetic data.
