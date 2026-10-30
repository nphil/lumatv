# Settings at 120 Hz: why frames miss (round 21 data)

## Resume here

Done (branch: forge main, own worktree `/tmp/Frames120-wt`, delete it only when finished):

- Read the round 21 frame gates (`compositor/slot-2026-10-29-r21/frames60q.out`, `frames120q.out`; the saved traces and `frames-120.json` stayed on the
  workstation in `/tmp/perf/gate-c4shell-120-*.trace` and `/tmp/lumatv-tv-out/`, so the numbers below are copied here).
- `perf/frametl.py` + `perf/frametl_parse.py`: per-flip timeline of an ftrace dump (compositor thread, shell process, GPU jobs).

Next, in order:

1. Build `lumatv-ui` (release) in the worktree, run the Settings walk on the host harness (`tests/rust_settings_host.py`, `--log`) and read the
   `[frame]` lines around a focus step: layers, drawn px, cpu, gpu. Compare HEAD with `6b4add12^` (focus plates made up front).
2. Fix what step 1 shows (candidates below), host tests, clippy, push.
3. Give the TV re-check: `python3 compositor/tools/tv_gate.py --only frames --quick` and `... --quick --refresh 120` (pass: every surface 0 dropped,
   0 `burst_over20`).

Open questions: what the 12 ms compositor frames after a focus step contain (needs the ring's `D` lines from the set, or the host's layer counts);
whether the gate's `sm_pid` should follow the compositor (below).

## What the round 21 traces say (120 Hz, `settings-scroll`, first cycle)

The gate's analysis reads the wrong process. `perf/gate.py` passes `pidof surface-manager` (the supervisor, `lumatv-sm`, pid 8856) as `--sm`;
LumaTV's compositor is its child (`ld-linux-aarch6`, pid 8969, the `loop 8969` marker `capture.sh` writes into the trace). So `p90_render_ms`
(0.1-0.6 ms in every LumaTV gate), `anim_frames` and `dropped` are read off the supervisor's few runs. The same trace with `--sm 8969`: 163 animated
frames (gate: 52), 82 dropped (gate: 35, 15 in motion), render p90 3.4 ms (gate: 0.5). `burst_over20` comes from the flips alone and stands.

Per frame, from `sched_switch` and `mali_job_slots_event` (compositor thread 8969, shell process 9478):

| phase | what a frame costs | result at 120 Hz |
|---|---|---|
| Settings opening, first 0.2 s | compositor render 3.3-5.2 ms, GPU busy 11-35 ms in 4-10 jobs per interval, shell up to 8-15 ms | intervals 16.7-33.4 ms |
| 0.2-0.7 s | render 3.5-4.2 ms from 4.7 ms after the flip, GPU 2.5 ms | mix of 8.3 and 16.7 ms |
| a held key on the System page, steady frames | one render burst 2.9-3.3 ms, GPU 1.1 ms | 8.3 ms, fits |
| the frame(s) after each focus step (every 100 ms key repeat) | renders 4.8 + 3.6 ms, GPU busy 13-19 ms in 6-9 jobs | one 16.7 ms interval each, about 25 per walk |
| rail steps (page switches) | GPU busy 8-17 ms a frame | 16.7 ms |

At 60 Hz the same phases cost the same CPU per frame (2.4-5 ms) with no 12 ms step, and nothing misses (round 21: 1 dropped in 3 cycles).

Not the harness alone: the process storm in the first 0.7 s of each open (`sh`, `grep`, `systemctl`, LG's SettingsService, kb.py's python3 at 70-80 ms
CPU per key) uses 70 % of the four cores for 0.2 s, but the 12 ms frames recur every key step for seconds with the CPUs idle.

Candidates for the step cost: a focus plate made on the focus step and dropped when the lift rests (`6b4add12`: two groups and a layer per move, the
first draw of a new layer), the pacer's `reshaped` rule, glyph/atlas uploads for rows entering the view.
