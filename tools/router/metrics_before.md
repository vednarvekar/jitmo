### before (head V:/Code/jitmo/models/router/head.json)

| Set | All (exact) | Single-task | Chains | OUT only |
|---|---|---|---|---|
| test | 112/129 = 86.8% | 104/112 = 92.9% | 8/17 = 47.1% | 13/14 = 92.9% |
| test2 | 105/133 = 78.9% | 98/120 = 81.7% | 7/13 = 53.8% | 8/12 = 66.7% |
| val | 114/122 = 93.4% | 104/107 = 97.2% | 10/15 = 66.7% | 8/8 = 100.0% |
| val2 | 102/119 = 85.7% | 99/112 = 88.4% | 3/7 = 42.9% | 10/10 = 100.0% |
| test+test2 | 217/262 = 82.8% | 202/232 = 87.1% | 15/30 = 50.0% | 21/26 = 80.8% |
| heldout_phase2 | 100/122 = 82.0% | 96/114 = 84.2% | 4/8 = 50.0% | 14/20 = 70.0% |

Confusable pairs, single-task errors (first -> second / second -> first), test+test2 and heldout_phase2:

| Pair | test+test2 | heldout_phase2 |
|---|---|---|
| S1 vs A3 (volume vs media) | 3 / 0 | 3 / 0 |
| E5 vs F5 (reply vs send file) | 1 / 0 | 0 / 0 |
| E7 vs M1 (email vs whatsapp) | 1 / 0 | 0 / 0 |
| E7 vs E5 (new mail vs reply) | 0 / 0 | 0 / 0 |
| A2 vs A3 (play X vs pause/skip) | 1 / 0 | 0 / 0 |
| OUT looks in scope (OUT -> task) | 5 / 0 | 6 / 0 |
| in-scope refused (task -> OUT) | 4 / 0 | 3 / 0 |
