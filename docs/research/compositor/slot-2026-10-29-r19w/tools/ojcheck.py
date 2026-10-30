#!/usr/bin/env python3
"""ojcheck.py: OverlayJank's re-check, exactly as asked: with YouTube playing full screen (the video's id as the gate uses it), then with Home in front, an idle
window, then osd (volume key 115, 114 back) and quick (`ctl shell show quick`, Back): `ctl refresh reset; ctl wake reset; ctl note OJ-<kind>; <open>; sleep 4;`
then ctl refresh, ctl wake, ctl status, ctl ring 1500 and the Mali utilisation, each to its own file under the slot's video/oj/. The receiver's mute is the
caller's (the volume key may unmute it)."""
import os, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv

D = "/tmp/lumatv-r19w/docs/research/compositor/slot-2026-10-29-r19w/video/oj"
os.makedirs(D, exist_ok=True)
CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"

def ctl(a, t=40):
    return tv.tv("%s %s 2>/dev/null" % (CTL, a), t)

def say(s):
    print("%s %s" % (time.strftime("%T"), s), flush=True)

def luna(uri, payload):
    return tv.tv("luna-send -n 1 -f %s '%s' 2>&1 | tr -d '\\n' | cut -c1-160" % (uri, payload), 30)

def save(name, text):
    open("%s/%s" % (D, name), "w").write(text)

def window(prefix, kind, opener=None, closer=None):
    ctl("refresh reset"); ctl("wake reset"); ctl("note OJ-%s%s" % (prefix, kind))
    if opener:
        opener()
    time.sleep(4)
    save("%s%s-refresh.json" % (prefix, kind), ctl("refresh"))
    save("%s%s-wake.json" % (prefix, kind), ctl("wake"))
    save("%s%s-status.json" % (prefix, kind), ctl("status"))
    save("%s%s-ring.json" % (prefix, kind), ctl("ring 1500"))
    save("%s%s-mali.txt" % (prefix, kind), tv.tv("cat /sys/kernel/debug/mali0/dvfs_utilization", 20))
    if closer:
        closer()
    time.sleep(3)
    say("%s%s done" % (prefix, kind))

def idle(prefix):
    ctl("refresh reset"); ctl("note OJ-%sidle" % prefix)
    time.sleep(4)
    save("%sidle-refresh.json" % prefix, ctl("refresh"))
    save("%sidle-ring.json" % prefix, ctl("ring 500"))
    say("%sidle done" % prefix)

def osd_open(): ctl("remote 115 tap")
def osd_close(): ctl("remote 114 tap")
def quick_open(): ctl("shell show quick")
def quick_close(): ctl("remote 158 tap")

def run_set(prefix):
    idle(prefix)
    window(prefix, "osd", osd_open, osd_close)
    window(prefix, "quick", quick_open, quick_close)

if __name__ == "__main__":
    say("YouTube full screen")
    luna("luna://com.webos.service.applicationmanager/launch", '{"id":"youtube.leanback.v4","params":{"contentTarget":"https://www.youtube.com/tv?v=aqz-KE-bpKQ"}}')
    time.sleep(30)
    say("foreground: " + tv.tv("luna-send -n 1 -f luna://com.webos.service.applicationmanager/getForegroundAppInfo '{}' | grep appId", 20).strip())
    run_set("")
    luna("luna://com.webos.service.applicationmanager/closeByAppId", '{"id":"youtube.leanback.v4"}')
    luna("luna://com.webos.service.applicationmanager/launch", '{"id":"com.webos.app.home"}')
    time.sleep(8)
    say("foreground: " + tv.tv("luna-send -n 1 -f luna://com.webos.service.applicationmanager/getForegroundAppInfo '{}' | grep appId", 20).strip())
    run_set("home-")
    save("mali-note.txt", "taken %s, build bb7a491b, AR24; the volume keys 115 and 114 went through ctl remote\n" % time.strftime("%T"))
    say("ojcheck done")
