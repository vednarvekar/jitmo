# SP2 result: can a tiny model pull out names, files and times from casual wording?

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
