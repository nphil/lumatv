#!/usr/bin/env python3
"""oledcost.py: the OLED care's cost, box dim and app dim on against off, over a still app and over Home, 2 minutes each, hands off.

Per run: the theme's oledcare tokens set (boxDim and appDim on with short timings, or both off; the tokens that were there are put back at the end,
by md5), the context put in front (LG's browser on a static page of two bright boxes, or Home), the dims waited for until they are engaged
(`ctl status` oledcare.boxDim.engaged / appDim.level), `ctl wake reset`, then a snapshot, WINDOW seconds, a snapshot: the compositor's CPU (its
/proc stat ticks), Mali's busy share (debugfs dvfs_utilization), frames drawn and flipped a second, direct and composed frames (`ctl scanout`, direct
scan-out switched on for the run so that the count means something), the loop's busy percentiles (`ctl wake`) and passUsMax, renderUsMax.
Lines go to OUT (jsonl) and stdout. Needs LumaTV's compositor live (runtime dir /tmp/xdg)."""
import base64, json, os, re, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv

CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
THEME = "/var/lib/lumatv/theme/theme.json"
OUT = os.environ.get("OUT", "/tmp/r19w-gate/oledcost.jsonl")
WINDOW = int(os.environ.get("WINDOW", "120"))
PAGE = ('<!doctype html><meta name=viewport content="width=device-width"><body style="margin:0;background:#0c0c0c;overflow:hidden">'
        '<div style="position:absolute;left:90px;top:130px;width:620px;height:360px;background:#ececec"></div>'
        '<div style="position:absolute;left:760px;top:330px;width:520px;height:300px;background:#f2f0d6"></div></body>')

def ctl(a, t=30):
    out = tv.tv("%s %s 2>/dev/null" % (CTL, a), t).strip()
    try:
        return json.loads(out)
    except Exception:
        return {"raw": out[:300]}

def say(s):
    print("%s %s" % (time.strftime("%T"), s), flush=True)

def luna(uri, payload, t=30):
    return tv.tv("luna-send -n 1 -f %s '%s' 2>&1" % (uri, json.dumps(payload)), t)

def fg():
    out = tv.tv("luna-send -n 1 -f luna://com.webos.service.applicationmanager/getForegroundAppInfo '{}' | grep appId", 20)
    m = re.search(r'"appId":\s*"([^"]*)"', out)
    return m.group(1) if m else ""

def put_context(ctx):
    if ctx == "home":
        if fg() != "com.webos.app.home":
            luna("luna://com.webos.service.applicationmanager/closeByAppId", {"id": "com.webos.app.browser"})
            luna("luna://com.webos.service.applicationmanager/launch", {"id": "com.webos.app.home"})
        for _ in range(20):
            if fg() == "com.webos.app.home":
                break
            time.sleep(1)
    else:
        if fg() != "com.webos.app.browser":
            url = "data:text/html;base64," + base64.b64encode(PAGE.encode()).decode()
            luna("luna://com.webos.service.applicationmanager/launch", {"id": "com.webos.app.browser", "params": {"target": url}})
        for _ in range(30):
            if fg() == "com.webos.app.browser":
                break
            time.sleep(1)
        time.sleep(6)
    return fg()

def theme_merge(tokens):
    code = ("import json,sys\n"
            "p=sys.argv[1]; t=json.load(open(p)); o=t.setdefault('oledcare',{}); o.update(json.loads(sys.argv[2])); json.dump(t,open(p,'w'),indent=2)\n")
    tv.tv("cat > /tmp/r19w-merge.py <<'EOF'\n%sEOF\npython3 /tmp/r19w-merge.py %s '%s'; echo merged" % (code, THEME, json.dumps(tokens)), 30)

def counters():
    st = ctl("status")
    sc = ctl("scanout").get("scanout", {})
    summ = st.get("summary", "")
    m = {k: int(v) for k, v in re.findall(r"\b(flips|late|lost|direct|split)\s+(\d+)", summ)}
    raw = tv.tv("pid=$(for p in /proc/[0-9]*; do if tr '\\0' ' ' < $p/cmdline 2>/dev/null | grep -q -- '--rendere[r] km[s]'; then echo ${p#/proc/}; break; fi; done); "
                "echo pid $pid; cut -d' ' -f14,15 /proc/$pid/stat; "
                "for p in /proc/[0-9]*; do if tr '\\0' ' ' < $p/cmdline 2>/dev/null | grep -q 'lumatv-u[i]'; then echo ui ${p#/proc/} $(cut -d' ' -f14,15 $p/stat); break; fi; done; "
                "sed -n 's/^busy_time: *\\([0-9]*\\)/busy \\1/p;s/.*idle_time: *\\([0-9]*\\)/idle \\1/p' /sys/kernel/debug/mali0/dvfs_utilization; "
                "sed -n 's/^busy_time: *[0-9]* *idle_time: *\\([0-9]*\\)/idle \\1/p' /sys/kernel/debug/mali0/dvfs_utilization; cut -d' ' -f1 /proc/uptime", 30)
    pid = re.search(r"pid (\d+)", raw)
    ticks = re.search(r"^(\d+) (\d+)$", raw, re.M)
    ui = re.search(r"ui (\d+) (\d+) (\d+)", raw)
    busy = re.search(r"busy (\d+)", raw)
    idle = re.search(r"idle (\d+)", raw)
    up = re.findall(r"^(\d+\.\d+)$", raw, re.M)
    return {"t": time.time(), "uptime": float(up[-1]) if up else None, "stats": st.get("stats", {}), "oledcare": st.get("oledcare", {}), "summary": m,
            "scanout": sc.get("frames", {}), "pid": pid.group(1) if pid else None,
            "cpu_ticks": (int(ticks.group(1)) + int(ticks.group(2))) if ticks else None,
            "ui_ticks": (int(ui.group(2)) + int(ui.group(3))) if ui else None,
            "busy": int(busy.group(1)) if busy else None, "idle": int(idle.group(1)) if idle else None}

def engaged(ctx):
    o = ctl("status").get("oledcare", {})
    bd, ad, app = o.get("boxDim", {}), o.get("appDim", {}), o.get("app", {})
    return {"boxDim": bd.get("engaged"), "boxCells": bd.get("cells"), "boxLowest": bd.get("lowest"), "appDim": ad.get("level"), "appEngaged": app.get("engaged"),
            "dim": o.get("dim"), "stillSec": app.get("stillSec")}

def run(ctx, state, wait_max=300):
    tokens = ({"boxDim": True, "appDim": True, "boxDimAfterMin": 1, "boxDimRampSec": 20, "appDimAfterMin": 1, "dimOverMin": 1} if state == "on"
              else {"boxDim": False, "appDim": False})
    theme_merge(tokens)
    time.sleep(3)
    front = put_context(ctx)
    ctl("scanout direct")
    t0 = time.time()
    e = engaged(ctx)
    if state == "on" and ctx == "app":
        while time.time() - t0 < wait_max:
            e = engaged(ctx)
            if e.get("boxDim") and (e.get("appDim") or 1.0) <= 0.76:
                break
            time.sleep(10)
    elif state == "on":
        time.sleep(90)
        e = engaged(ctx)
    else:
        time.sleep(20)
        e = engaged(ctx)
    say("%s %s: front %s, engaged %s after %d s" % (ctx, state, front, e, time.time() - t0))
    ctl("wake reset")
    a = counters()
    time.sleep(WINDOW)
    b = counters()
    wake = ctl("wake")
    dt = b["t"] - a["t"]
    du = (b["uptime"] - a["uptime"]) if a["uptime"] and b["uptime"] else dt
    d = lambda k: (b["stats"].get(k, 0) - a["stats"].get(k, 0))
    r = {"ctx": ctx, "state": state, "front": front, "window_s": round(du, 1), "engaged_before": e, "engaged_after": engaged(ctx),
         "drawn_per_s": round(d("drawn") / du, 2), "commits_per_s": round(d("commits") / du, 2),
         "flips_per_s": round((b["summary"].get("flips", 0) - a["summary"].get("flips", 0)) / du, 2),
         "direct_frames": b["scanout"].get("direct", 0) - a["scanout"].get("direct", 0), "composed_frames": b["scanout"].get("composed", 0) - a["scanout"].get("composed", 0),
         "compositor_cpu_pct": round((b["cpu_ticks"] - a["cpu_ticks"]) / du, 2) if b["cpu_ticks"] is not None and a["cpu_ticks"] is not None else None,
         "ui_cpu_pct": round((b["ui_ticks"] - a["ui_ticks"]) / du, 2) if b["ui_ticks"] is not None and a["ui_ticks"] is not None else None,
         "gpu_busy_pct": round(100.0 * (b["busy"] - a["busy"]) / max(1, (b["busy"] - a["busy"]) + (b["idle"] - a["idle"])), 2) if b["busy"] is not None else None,
         "passUsMax": b["stats"].get("passUsMax"), "passUsMax_before": a["stats"].get("passUsMax"), "renderUsMax": b["stats"].get("renderUsMax"),
         "boxdim_sampleUsMax": (b["oledcare"].get("boxDim") or {}).get("sampleUsMax"), "boxdim_steps": ((b["oledcare"].get("boxDim") or {}).get("steps", 0) - (a["oledcare"].get("boxDim") or {}).get("steps", 0)),
         "loop_busy": {k: wake.get(k) for k in ("busy", "tick", "flip")} if isinstance(wake, dict) else None, "pid": b["pid"]}
    ctl("scanout off")
    with open(OUT, "a") as f:
        f.write(json.dumps(r) + "\n")
    say("RESULT " + json.dumps({k: v for k, v in r.items() if k not in ("loop_busy", "engaged_before", "engaged_after")}))
    return r

if __name__ == "__main__":
    plan = sys.argv[1:] or ["app:on", "app:off", "app:on", "home:on", "home:off", "home:on"]
    backup = tv.tv("cp -p %s /tmp/r19w-theme.bak && md5sum /tmp/r19w-theme.bak" % THEME, 20).split()[0]
    say("theme backup md5 %s" % backup)
    try:
        for p in plan:
            ctx, state = p.split(":")
            run(ctx, state)
    finally:
        tv.tv("cp -p /tmp/r19w-theme.bak %s; md5sum %s" % (THEME, THEME), 20)
        ctl("scanout off")
        put_context("home")
        after = tv.tv("md5sum %s" % THEME, 20).split()[0]
        say("theme restored md5 %s (same as before: %s)" % (after, after == backup))
