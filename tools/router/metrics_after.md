### after (train rows 14779, head C=30.0)

| Set | All (exact) | Single-task | Chains | OUT only |
|---|---|---|---|---|
| test | 112/129 = 86.8% | 102/112 = 91.1% | 10/17 = 58.8% | 12/14 = 85.7% |
| test2 | 115/133 = 86.5% | 107/120 = 89.2% | 8/13 = 61.5% | 10/12 = 83.3% |
| val | 111/122 = 91.0% | 101/107 = 94.4% | 10/15 = 66.7% | 7/8 = 87.5% |
| val2 | 100/119 = 84.0% | 98/112 = 87.5% | 2/7 = 28.6% | 10/10 = 100.0% |
| test+test2 | 227/262 = 86.6% | 209/232 = 90.1% | 18/30 = 60.0% | 22/26 = 84.6% |

Confusable pairs, single-task errors (first -> second / second -> first), test+test2:

| Pair | test+test2 | heldout_phase2 |
|---|---|---|
| S1 vs A3 (volume vs media) | 0 / 1 | - |
| E5 vs F5 (reply vs send file) | 0 / 0 | - |
| E7 vs M1 (email vs whatsapp) | 0 / 0 | - |
| E7 vs E5 (new mail vs reply) | 0 / 0 | - |
| A2 vs A3 (play X vs pause/skip) | 0 / 0 | - |
| OUT looks in scope (OUT -> task) | 4 / 0 | - |
| in-scope refused (task -> OUT) | 4 / 0 | - |
