# SP8 plan: does the whole pipeline stay in budget?

Status: **planned, not run.** Needs SP1/SP2 ONNX models, SP3/SP4 planner and SP11 shell, so it runs last, on the 8 GB test laptop.

## Pass (doc 5, PR1-PR4, PR6, PR8)
Idle < 300 MB and < 1% CPU; peak < 2 GB; known task starts acting < 0.5 s; planner request < 5 s.

## What the earlier spikes say going in
| Part | Measured so far | Problem for SP8 |
|---|---|---|
| UI shell (SP11) | web view alive ~155 MB private / ~340 MB WS; destroyed ~5 MB; cold open ~1 s | Idle budget depends on destroy-when-idle |
| Extractor (SP2) | PyTorch GLiNER-small 1.6 GB RSS, ~1 s (loaded box) | Must use ONNX int8 (~183 MB file); numbers from the SP2 agent pending |
| Router (SP1) | see sp1-router/RESULT.md | ONNX int8 encoder |
| Planner (SP3) | Qwen2.5-1.5B Q4: RSS 1.85-2.06 GB, 7/10 fresh chains, latency not validly measured | Alone it consumes the whole 2 GB peak; wrong plans ~30-50% |
| Adapters (SP4) | swap < 1 ms, random adapters | Quality untested |

Expected conclusion if nothing changes: Tier 0 fits; Tier 1 as specified does not (peak and 5 s).

## Method (all on the 8 GB laptop, Chrome + a chat app open, AC and battery)
1. Assemble a throwaway pipeline: Tauri shell (SP11 variant B) -> Rust or Python core stub -> model worker process (ONNX router + extractor, optional llama.cpp planner) -> fake connectors that only log.
2. Drive T01-T100 (text from doc 4) through it with a runner that records per request: time to first action, total time, peak private memory of all jitmo processes (sampled every 100 ms), and pass/fail on route + slots.
3. Scenarios:
   - S1 idle: boot, wait 10 min, record RSS (sum of all processes, private + working set) and CPU. Variants: web view kept vs destroyed.
   - S2 known tasks only (Tier 0): 50 requests, latency p50/p95, peak RSS.
   - S3 planner: chained requests T75-T82 plus 10 extra; cold load, warm, and idle unload (30 s, 120 s); peak RSS; latency to first valid plan.
   - S4 mixed day: 200 requests, 20% chained, with 3 browser tabs and Teams/WhatsApp open; watch for memory growth (leak) and swap-ins.
   - S5 startup: login to ready (PR8, < 5 s).
4. Repeat each scenario 3 times; report median and worst. Record laptop CPU, RAM, disk, power mode.

## Levers if it fails (in order)
1. Destroy web view when idle (SP11).
2. Smaller encoder / int8 / drop extractor model for rules plus gazetteer (SP2).
3. Planner: 1B model, shorter prompt with top-5 skills only, KV cache cap, mmap, unload at 30 s.
4. Replace planner for common chains with saved templates (doc 5 SP3 fallback); send rare chains to "can't do that yet".
5. Raise minimum to 12 GB (last resort, doc 5).

## Deliverable
`spikes/sp8-pipeline/RESULT.md` with a table per scenario against PR1-PR8, verdict, and the decision-log updates (D4, D20).

## Early footprint numbers (8 Nov, this PC, Python harness)
Router int8: +33 MB on load (85 MB total), 0.1 s load, 1-2 ms per request. Extractor int8: +186 MB on load (about 240 MB total), 0.85 s load, peak about 365 MB during load. In-process unload freed nothing; process exit is the reliable way. CPU test (8 cores saturated): flat-out worker cost the competing job 7-19%, idle plus EcoQoS 7%, noise floor 7%. Still to do: Rust footprint, real extractor inference, the 8GB laptop.
