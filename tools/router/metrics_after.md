### after (train rows 14779, head C=30.0)

| Set | All (exact) | Single-task | Chains | OUT only |
|---|---|---|---|---|
| test | 112/129 = 86.8% | 102/112 = 91.1% | 10/17 = 58.8% | 11/14 = 78.6% |
| test2 | 115/133 = 86.5% | 106/120 = 88.3% | 9/13 = 69.2% | 9/12 = 75.0% |
| val | 114/122 = 93.4% | 104/107 = 97.2% | 10/15 = 66.7% | 8/8 = 100.0% |
| val2 | 97/119 = 81.5% | 95/112 = 84.8% | 2/7 = 28.6% | 9/10 = 90.0% |
| test+test2 | 227/262 = 86.6% | 208/232 = 89.7% | 19/30 = 63.3% | 20/26 = 76.9% |
| heldout_phase2 | 112/122 = 91.8% | 106/114 = 93.0% | 6/8 = 75.0% | 19/20 = 95.0% |

Confusable pairs, single-task errors (first -> second / second -> first), test+test2 and heldout_phase2:

| Pair | test+test2 | heldout_phase2 |
|---|---|---|
| S1 vs A3 (volume vs media) | 0 / 1 | 0 / 0 |
| E5 vs F5 (reply vs send file) | 0 / 0 | 0 / 0 |
| E7 vs M1 (email vs whatsapp) | 0 / 0 | 0 / 0 |
| E7 vs E5 (new mail vs reply) | 0 / 0 | 0 / 0 |
| A2 vs A3 (play X vs pause/skip) | 0 / 0 | 0 / 0 |
| OUT looks in scope (OUT -> task) | 6 / 0 | 1 / 0 |
| in-scope refused (task -> OUT) | 2 / 0 | 2 / 0 |
