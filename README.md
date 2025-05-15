Currently performance

Uses Vyukov's MPMC ring buffer.

Against `Mutex<VecDequeue<T>>`:

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

_Note: this isn't the most fair test since VecDequeue grows_.


Against `flume`:

```
Trial                 Ring (ms)          Other<T> :/ (ms)        Diff (%)
1                       763.761                  2498.028          69.43%
2                       756.708                  2485.040          69.55%
3                       748.352                  2491.846          69.97%
Mean                    756.274                  2491.638          69.65%
Std Dev                   6.298                     5.304           0.23%
```

Against `crossbeam_channel`:

```
Trial                 Ring (ms)          Other<T> :/ (ms)        Diff (%)
1                       759.799                   746.906          -1.73%
2                       750.364                   743.368          -0.94%
3                       762.368                   730.063          -4.42%
Mean                    757.510                   740.112          -2.36%
Std Dev                   5.161                     7.251           1.49%
```

An optimization is maybe instead of a slot being 2 atomics, each slot could be one? Might make default values hard though
