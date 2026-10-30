#!/usr/bin/env python3
"""Collects the set's state (round 18 checklist step 2 / section 11) into a JSON file: python3 state.py OUT.json"""
import json, re, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv
LT = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
CTL = "/var/lib/lumatv/compositor/lib/ld-linux-aarch64.so.1 --library-path /var/lib/lumatv/compositor/lib /var/lib/lumatv/compositor/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
def sh(c, t=60):
    return tv.tv(c, t).strip()
def js(c, t=60):
    out = sh(c, t)
    m = re.search(r"\{.*\}", out, re.S)
    try:
        return json.loads(m.group(0)) if m else out
    except ValueError:
        return out
s = {"when": time.strftime("%Y-%m-%d %H:%M:%S %Z")}
s["power"] = js("luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}'").get("state")
s["foreground"] = js("luna-send -n 1 -f luna://com.webos.applicationManager/getForegroundAppInfo '{}'").get("appId")
m = re.search(r'"pictureMode"\s*:\s*"([^"]+)"', sh("luna-send -n 1 -f luna://com.webos.settingsservice/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"pictureMode\"]}'"))
s["pictureMode"] = m.group(1) if m else None
s["inputs"] = js(LT + " inputs status")
md = {}
for name, path in (("theme.json", "/var/lib/lumatv/theme/theme.json"), ("layout.json", "/var/lib/lumatv/theme/layout.json"), ("helper.json", "/var/lib/lumatv/helper.json"), ("Tokens.js", "/var/lib/lumatv/theme/Tokens.js")):
    md[name] = sh("md5sum %s 2>&1 | cut -d' ' -f1" % path)
s["md5"] = md
s["keyboard"] = js(LT + " keyboard")
s["transitions"] = js(LT + " transitions status")
s["wallpaper"] = js(LT + " wallpaper status")
s["compositor"] = js(LT + " compositor status").get("compositor") if isinstance(js(LT + " compositor status"), dict) else None
s["ctl_shell"] = js("test -S /tmp/xdg/lumatv-control && " + CTL + " shell 2>&1 | head -c 3000 || echo '{\"note\":\"no compositor session\"}'")
s["pixelRefresher"] = js("luna-send -n 1 -f luna://com.webos.service.oledepl/getPixelRefresherInfoList '{}'")
s["panelUsage"] = js("luna-send -n 1 -f luna://com.webos.service.panelcontroller/getPanelUsageTime '{\"subscribe\":false}'")
s["panelTopic"] = js("curl -s 127.0.0.1:18731/v1/state/panel")
s["oledcare"] = js("grep -o '\"oledcare\":{[^}]*}' /var/lib/lumatv/theme/theme.json | head -1 | sed 's/^\"oledcare\"://'")
s["theme_switches"] = {k: sh("grep -o '\"%s\":{[^}]*}' /var/lib/lumatv/theme/theme.json | head -1" % k) for k in ("appDim", "appShift", "boxDim", "overlays", "capture", "cameras", "refreshGuard")}
s["helper_json"] = sh("cat /var/lib/lumatv/helper.json 2>&1 | head -c 1500")
s["always_ready"] = js(LT + " standbyscreen status")
s["uptime"] = sh("uptime; grep -E 'MemAvailable|CmaFree' /proc/meminfo")
s["lumatv_procs"] = sh("ps | grep -E 'lumatv|gate.sh|tv_gate' | grep -v grep | head -20")
json.dump(s, open(sys.argv[1], "w"), indent=1)
print(json.dumps(s, indent=1)[:6000])
