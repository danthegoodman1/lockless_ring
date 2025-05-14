Currently getting

```
thread '<unnamed>' panicked at src/lib.rs:138:17:
read null value, did the read head pass the write head? read_head=11 write_head=9 next_read_head=12 debug_reads=113 debug_writes=214
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread '<unnamed>' panicked at src/lib.rs:138:17:
read null value, did the read head pass the write head? read_head=21 write_head=18 next_read_head=22 debug_reads=123 debug_writes=221

thread '<unnamed>' panicked at src/lib.rs:138:17:
read null value, did the read head pass the write head? read_head=37 write_head=35 next_read_head=38 debug_reads=139 debug_writes=238

thread '<unnamed>' panicked at src/lib.rs:138:17:
read null value, did the read head pass the write head? read_head=97 write_head=95 next_read_head=98 debug_reads=198 debug_writes=298

thread '<unnamed>' panicked at src/lib.rs:138:17:
read null value, did the read head pass the write head? read_head=98 write_head=96 next_read_head=99 debug_reads=200 debug_writes=299

thread '<unnamed>' panicked at src/lib.rs:138:17:
read null value, did the read head pass the write head? read_head=14 write_head=13 next_read_head=15 debug_reads=217 debug_writes=317
```

checked that it's definitely null values that we are swapping in that we're eventually reading as an ok value

I wonder if this is ABA problem? I don't think so because we'd see _another value_ I think (we'd have a shared pointer), not the impossible value

not an ordering problem because i tried everything with SeqCst
