#!/usr/bin/env python3
"""What a walk of Settings costs the engine, frame by frame, on the host harness (`lumatv-ui --host --log`: Mesa's software
rasteriser, the engine as the compositor runs it, paced at 60 Hz). The layers, the pixels drawn and the CPU time of a frame say
what it asks of the set; the GPU time here is llvmpipe's, so it only says how the work scales.

    python3 tests/host_frames.py [--page picture] [--steps 12] [--gap 100] [--bin PATH] [--list]

The walk: Settings opens on PAGE, Right enters its list, then `--steps` Down keys `--gap` ms apart (a held key's repeat on the
set). Each key's frames (up to the next key) are summed: how many, layers made ready, pixels drawn, CPU. `--bin` names another
`lumatv-ui` (to compare two builds); `LUMATV_UI_BIN` does the same."""
import argparse
import os
import re
import statistics
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import rust_settings_host as host  # noqa: E402
import lgsettings  # noqa: E402

FRAME = re.compile(r"^\[frame\] (\d+) at (\d+) ms: damage (\d+) px, drew (\d+) px in (\d+) rects \(([\d.]+) %\), layers (\d+) culled (\d+) cpu (\d+) us gpu (\d+) us", re.M)
OPEN_AT = 900
ENTER_AT = 2200
FIRST_AT = 2800


def frames(text):
    """The `[frame]` lines of a run: (ms, damage px, drawn px, rects, layers, culled, cpu us, gpu us)."""
    return [(int(m[2]), int(m[3]), int(m[4]), int(m[5]), int(m[7]), int(m[8]), int(m[9]), int(m[10])) for m in FRAME.finditer(text)]


def walk(page, steps, gap, binary=None, kinds="settings", text_scale=1.0, backdrop=None):
    """Runs the walk; returns (frames, key times, output)."""
    if binary:
        host.BIN = binary
    log = os.path.join(tempfile.mkdtemp(prefix="hostframes-"), "frames.log")
    keys = [(ENTER_AT, "right")] + [(FIRST_AT + i * gap, "down") for i in range(steps)]
    extra = ["--log", log] + (["--backdrop", backdrop] if backdrop else [])
    for at, name in keys:
        extra += ["--key", "%d:%s" % (at, name)]
    end = FIRST_AT + steps * gap + 1500
    run_steps = [(OPEN_AT - 300, lambda c: c.push("foreground", {"app": "com.webos.app.home", "home": True})),
                 (OPEN_AT, lambda c: c.push("show", {"kind": "settings", "page": page}))]
    rc, out, _, _, _ = host.run(steps=run_steps, wait=end, extra=extra, lg=lgsettings.LgService(), helper=host.FakeHelper({"topics": {}}), kinds=kinds,
                                text_scale=text_scale)
    text = open(log).read() if os.path.exists(log) else out
    return frames(text), [at for at, _ in keys], text


def per_key(fr, key_times):
    """For each key: the frames from it to the next key. Frame times are the host's own clock from the run's start, which the keys'
    times are on too (both counted from the first step); a frame belongs to the last key before it."""
    rows = []
    for i, at in enumerate(key_times):
        nxt = key_times[i + 1] if i + 1 < len(key_times) else at + 600
        mine = [f for f in fr if at <= f[0] < nxt]
        rows.append((at, mine))
    return rows


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--page", default="picture")
    ap.add_argument("--steps", type=int, default=12)
    ap.add_argument("--gap", type=int, default=100)
    ap.add_argument("--bin", default=os.environ.get("LUMATV_UI_BIN"))
    ap.add_argument("--all", action="store_true", help="print every frame")
    ap.add_argument("--kinds", default="settings", help="the kinds the shell answers (home,settings draws Home under Settings)")
    ap.add_argument("--backdrop", help="a 1920x1080 PNG drawn behind everything (a still of Home)")
    a = ap.parse_args(argv)
    fr, keys, text = walk(a.page, a.steps, a.gap, a.bin, kinds=a.kinds, backdrop=a.backdrop)
    if not fr:
        print(text[-2000:])
        return 1
    if a.all:
        for f in fr:
            print("%6d ms  damage %8d  drew %8d px %2d rects  layers %3d culled %3d  cpu %6d us  gpu %7d us" % f)
    print("key at   frames  layers(max)  drawn px   cpu us (sum)   first frame cpu us")
    cpus = []
    for at, mine in per_key(fr, keys)[1:]:
        if not mine:
            print("%6d   none" % at)
            continue
        cpu = sum(f[6] for f in mine)
        cpus.append(cpu)
        print("%6d   %4d    %4d      %9d   %9d      %9d" % (at, len(mine), max(f[4] for f in mine), sum(f[2] for f in mine), cpu, mine[0][6]))
    if cpus:
        print("per step: cpu median %d us, worst %d us; frames per step median %d" % (
            statistics.median(cpus), max(cpus), statistics.median(len(m) for _, m in per_key(fr, keys)[1:])))
    return 0


if __name__ == "__main__":
    sys.exit(main())
