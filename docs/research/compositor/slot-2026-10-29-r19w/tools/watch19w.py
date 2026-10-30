#!/usr/bin/env python3
"""watch19w.py idle MINUTES: returns when no real remote input has reached the set for MINUTES (the compositor's `ctl keys` delivered count and
`ctl wake` input count, both unchanged; the gates' own keys go through `ctl remote` and count in `delivered`, so this is for the quiet times).
watch19w.py poll: one reading, printed as JSON (for a monitor that runs beside a gate and compares with the reading it took at the start)."""
import json, subprocess, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv

CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"

def reading():
    out = {}
    try:
        k = json.loads(tv.tv("%s keys 2>/dev/null" % CTL, 20).strip())
        w = json.loads(tv.tv("%s wake 2>/dev/null" % CTL, 20).strip())
        out = {"delivered": k.get("delivered"), "fg": k.get("foreground"), "input": w.get("input", {}).get("n")}
    except Exception as e:
        out = {"error": str(e)[:120]}
    return out

if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "poll"
    if mode == "poll":
        print(json.dumps(reading()))
    elif mode == "idle":
        minutes = float(sys.argv[2])
        last = reading()
        since = time.time()
        print("%s start %s" % (time.strftime("%T"), last), flush=True)
        while True:
            time.sleep(20)
            r = reading()
            if "error" in r:
                continue
            if (r.get("delivered"), r.get("input")) != (last.get("delivered"), last.get("input")):
                last = r
                since = time.time()
                print("%s activity %s" % (time.strftime("%T"), r), flush=True)
            elif time.time() - since >= minutes * 60:
                print("%s IDLE for %.1f min %s" % (time.strftime("%T"), (time.time() - since) / 60, r), flush=True)
                break
