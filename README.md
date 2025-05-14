Currently performance

Uses Vyukov's MPMC with support for a single-consumer optimized `take_sc`.

```
Trial                 Ring (ms) Mutex<VecDequeue<T>> (ms)        Diff (%)
1                       790.992                   792.962           0.25%
2                       762.984                   772.668           1.25%
3                       759.107                   780.309           2.72%
4                       762.151                   787.134           3.17%
5                       764.031                   820.384           6.87%
6                       761.911                   793.853           4.02%
7                       789.491                   791.479           0.25%
8                       752.893                   780.804           3.57%
9                       766.203                   780.719           1.86%
10                      749.559                   798.004           6.07%
Mean                    765.932                   789.832           3.00%
Std Dev                  13.079                    12.594           2.13%
```
