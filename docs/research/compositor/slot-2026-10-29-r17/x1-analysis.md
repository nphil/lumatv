# x1, round 17: regression or noise

Not a regression of `64e015d..0d6d249`. Every late event in the one traced run is the X1 client answering the compositor's frame callbacks late, with the compositor's flips and loop on time. The rule as written (no client swap gap over 20 ms in 60 s) passed 8 of 25 earlier recorded runs, so 0 of 4 is inside what it does on its own. The traced build (`0d6d249`) sent every client's callbacks at the flip; `112d9cc` sends them with the frame's draw (`ctl callbacks clients landing` is the flip again), so a build after it needs both modes measured.

## The compositor's side (`x1-rerun-1.trace.gz`, a `--kwait` run, read with `x1-rerun-1-kwait.txt`)

- 3,594 flips in the client's 59.9 s, every interval 16.60 to 16.77 ms (mean 16.68); the gate line: pacer late 1 -> 1, loop stalls 0.
- The loop thread (31259, under the loader as `ld-linux-aarch6`) switches in 0.01 ms (p50) and 0.03 ms (p99) after a flip; the flip's pass, which sends the callbacks and releases, ends 0.12 ms (p50), 0.28 ms (p99), 0.94 ms (max) after it. A pass is 0.09 ms at p50.
- Frame cost as in round 16: GPU done 4.6 to 4.7 ms after the draw in both (`x1-compositor.txt` of each), render 0.34 ms (r17) against 0.38 ms (r16).

## The client's side

The time from a flip to the client's next GPU job start (in this build the callbacks go out at the flip, so this is the client waking, drawing and submitting):

| | ms |
|---|---|
| p50, p90, p99, p99.9 | 0.35, 0.49, 0.72, 2.49 |
| over 2 ms, over 3 ms | 6, 2 of 3,594 |
| max | 5.28 |

Two reactions over 3 ms: 5.28 ms at flip 12199.612 (+13.9 s; the frame before 0.44 ms, so the client's jobs 21.5 ms apart) and 3.64 ms at 12240.815 (+55.1 s; 20.0 ms apart). Rerun 1's one client gap over 20 ms was 21.1 ms; the second is at the limit. CPU 2, where the loop ran its flip pass, also ran `captureservice` (LG) in the first window and `micomservice` and `tAUDDTV` (LG) in the second. CPU 2 is where the loop and the client's threads meet: the loop's callbacks wake them there, behind the loop. With the callbacks at the flip a reaction under about 6 ms is absorbed, since the tick that latches the next frame comes 6 ms after a flip (10.7 ms before the refresh): the flips stayed on time under a 21 ms client gap. The set also starts at least 4 short-lived processes a second during the run (`grep`, `pgrep`, `zcat`, `gzip`, `luna-send`, `node`, `modprobe`); the trace does not show whose they are.

With the callbacks at the draw (`112d9cc`, the default now) the client's frame starts at the tick, 6 ms after the flip, and its commit is drawn at the next tick, so its budget is about 16 ms and not 6 [INFERENCE from the commit, no trace of that build]. The client's swap gaps are a wake-up after a callback in both modes, so nothing says their count changes; the two modes measured side by side will. `kwait.py` counts the reaction from the flips or from the compositor's draw, whichever the client's jobs follow, and says which.

The other runs: the gate run's gap (20.6 ms before swap 1031, monotonic 7630.742; `gate-run/x1-client.txt`) and rerun 3's (20.0 ms before swap 2364 at 12491.750; `x1-rerun-3-client.txt`) came with the compositor clean (loop stalls 0, frame gaps 0, pacer late 0). In reruns 1 and 2 the compositor also logged a client frame gap (a late commit); the compositor's log of rerun 2, the run with 4 gaps and one late flip, is not kept.

## The candidates

| change | on x1's path | evidence |
|---|---|---|
| `f24772c` scan-out (off) | `Scanout::choose` and `settle` return at once; a few field writes per flip | frames composed, same GPU time as r16 |
| `d853c55` callbacks, held counts, hot descriptors | a live surface's callbacks are paid as before; `release_buffer` asks `holds` (a map lookup) | flip pass 0.12 ms p50 |
| ring S line, pass count | two integer updates a pass, one string a second | pass 0.09 ms p50 |
| `d027688` luna pipe, `72e26fb` saver vote | the bridge and the shell client, which Home's cover idles | supervisor 3.7 loop wakes a second, `qml-runner` 2.5 GPU jobs a second |
| `5808ab5` OLED care sampling | one GPU sample every 8.008 s (4 s doubled on a tick that drew): 7 in the traced minute, begun in 0.40 to 0.60 ms, read in 0.06 to 0.08 ms (the gate run's log) | reactions in the 6 flips around each sample 0.23 to 0.57 ms; the slow ones are 2.5 and 5.5 s, 1.3 and 6.7 s from a sample; the gate run's gap came 1.19 s after sample 4 (7629.552) |
| `8204d58` volume hold, `7319cc8` refresh guard | the helper, a separate process: a key; a read every 10 minutes | not in the trace |
| memory | this run: MemAvailable 1,092 MiB at the compositor's start, 2 h after a boot; round 16 noted 493 MiB and swap 599 of 600 after its gate | `gate-full-stdout.txt`, `mem` |

## The rule's own rate

Runs recorded in `docs/research/compositor/` since the swapchain priming (`f0d3cb4`, 10-27 18:39): 25 before round 17, 8 passed, 34 gaps over 20 ms in all (1.36 a run). Round 17: 0 of 4, 7 gaps (1.75 a run). Chance of 0 passes in 4 at the earlier rate: 0.21; of 7 or more gaps in 4 runs (Poisson): 0.30. Counting from slot 0200 on 10-28 only (15 runs, 7 passed, 0.87 a run): 0.08 and 0.06. Rounds 14, 15 and 16 passed x1 on one to two runs each, with a failing run beside the passing one in 15 and 16.

## Leads that are not these gaps

- The loop writes its log with a blocking `write()` to ext4: two of its three D-state sleeps in the traced minute were in it (`jbd2__journal_start` / `wait_transaction_locked`, 13 us, right after the late commit, when the compositor logs its frame gap line [INFERENCE: the line is not in the kept log]; `__block_write_begin_int` / `io_schedule` at the end of the run). In x1's steady state it logs a handful of lines a minute; a busier moment puts a flash stall in a FIFO thread.
- The first second of each run is the client's start-up and carries the `trace:` lines (about 580 lines in 1.5 s).

## kwait

`--kwait` could not find the loop: the compositor runs under its private loader, so its threads are `ld-linux-aarch6`, not `lumatv-composit`. `capture.sh start` now writes `loop TID` into the trace and `kwait.py` reads it (then `--comm`, then the loader's name among the compositor's own). With the flips on time there is no late frame to explain, so `kwait.py` also reports the client's reaction after each flip, the ones over 3 ms with the tasks seen on the loop's CPU, and with `--x1` the client's gap lines beside the gaps the trace implies.

## Next round

Three configurations of `tv_gate.py --only x1 --kwait`, each a few times (`x1client.c` needs the new bundle for the third):

| | switch | what it settles |
|---|---|---|
| 1 | none: callbacks with the draw (`112d9cc`'s default) | the first look at the new default: the line names the loop thread, the moment the client follows (`the compositor's draw`) and the reaction spread |
| 2 | `--x1-callbacks landing` | callbacks at the flip, as in round 17: the same measure as the traced rerun here |
| 3 | `--x1-rt 10` (the client SCHED_FIFO 10, below the loop's 20 and the Mali threads' 19), with 1 or 2 | if the client's wake-ups are what the gaps are, the reaction tail ends (p99.9 near p99) and the gaps go; if they stay, the cause is not the client's scheduling |

Every gap of the client's should pair with a reaction 3.3 ms or more over the one before (`x1 log: N gap(s) ...; the trace implies N`), with the flips on time (longest interval under 17 ms) and pacer late unchanged; a gap that pairs with a late flip, a loop wait or a callback sent late would be the compositor's. Whether the rule itself should keep counting the client's reaction (it passes about a third of the runs on this set) is the owner's decision; nothing here changes it.

## Resume here

Done (one commit on forge/main, "x1: kwait finds the loop thread ..."): the analysis above; `docs/research/perf/kwait.py` (`find_loop`, `marked_loop`, `reactions`, `x1_lines`, `.gz` traces) and `docs/research/perf/capture.sh` (`comp_pid`, the `loop TID` marker); `compositor/tools/tv-gate.sh` x1 step (`--x1`, `LUMATV_X1_RT`, `LUMATV_X1_CALLBACKS`, `boxdim:` lines in `x1-compositor.txt`) and `compositor/tools/tv_gate.py` (`--x1-rt`, `--x1-callbacks`, `--kwait` also puts `sample.py`); `docs/research/arch/x1/x1client.c` (`X1_RT`); tests in `tests/test_kwait.py` (`LoopThread`, `ClientReaction`, `CommandOnATracedRun`, `CaptureScript`) and `tests/test_cutover.py` (`test_x1_rt_...`, `test_x1_callbacks_...`). No Rust changed.

Next:

1. The TV round of "Next round" (a new bundle: `x1client.c` changed). Read `x1-kwait.txt` and `x1-result.txt` of each run; put them beside `x1-rerun-1-kwait.txt` in the round's slot.
2. If the SCHED_FIFO client has no gaps and the reaction tail is gone, the client's scheduling is the cause and the rule is the owner's to rule on; if gaps stay, read `x1-kwait.txt` for a late flip or a loop wait (kwait's kernel waits), then the compositor.
3. Not done, found in the trace: the compositor's `StderrLogger::log` (`compositor/crates/lumatv-compositor/src/main.rs`) writes each line to stderr (ext4) on the FIFO loop thread, and two of the loop's three D-state sleeps in the traced minute were in that `write()`. A writer thread for the file with the ring kept synchronous would take it off the loop; it needs `cargo test` and a look at who reads the log file (the supervisor's crash bundle, the gate). The box dim logs only its first five samples (`SAMPLE_LINES` in `boxdim.rs`): more would show each gap's distance from a sample in every run.

Decisions taken: the rule (no client swap gap over 20 ms in 60 s) is unchanged; the FIFO client and the callback mode are opt-in switches, off by default; the `trigger` of `reactions` is picked from the data (the flips or the compositor's draws, whichever the client's jobs follow closest) because a trace has no callback event.
