import sys, time
sys.path[:0] = ['/tmp/lumatv-r12/compositor/tools', '/tmp/lumatv-r12/docs/research/arch']
import tv_gate, tv
p = tv_gate.pty
D = "D=/tmp/lumatv; LIB=$D/lib; ctl() { $LIB/ld-linux-aarch64.so.1 --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg \"$@\" 2>/dev/null; }; "
print(p("sh /tmp/lumatv/cutover.sh enable --session --max-minutes 12", 300)[-120:])
time.sleep(30)
def fg(): return p("luna-send -n 1 -f luna://com.webos.applicationManager/getForegroundAppInfo '{}' | grep appId", 30).strip()
print("fg before:", fg())
print(p(D + "ctl note livetv-launch >/dev/null; luna-send -n 1 -f luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.livetv\"}'", 40).strip()[:200])
for t in (3, 8, 15):
    time.sleep(t if t == 3 else t - (3 if t == 8 else 8))
    print("t+%ds fg: %s" % (t, fg()))
w = p(D + "ctl windows", 30)
import json, re
j = w[w.index("{"):w.rindex("}")+1]
d = json.loads(j)
for x in d["windows"]:
    if "live" in x["appId"] or x["appId"] == "com.webos.app.home":
        print("win", x["appId"], "mapped", x["mapped"], "inGroup", x["inGroup"], "state", x.get("state"), "front", x.get("front"))
print("groups", [(g["name"], g["members"]) for g in d["groups"]])
print(p("luna-send -n 1 -f luna://com.webos.service.capture/executeOneShot '{\"path\":\"/tmp/ltv.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}' | grep -i written", 40).strip())
tv.get("/tmp/ltv.png", "/tmp/r12out/livetv-front.png")
print(p(D + "ctl journal | sed 's/\",\"/\\n/g' | awk '/note: livetv-launch/{f=1} f' | grep -i 'attach\\|group\\|foreground\\|livetv\\|dmost\\|focus' | cut -c1-200 | head -30", 40))
print(p("grep -a -c LAUNCH_LASTAPP /var/log/messages /var/log/*.log 2>/dev/null | grep -v ':0'; grep -a 'LAUNCH_LASTAPP\\|last_input_handler' /var/log/messages 2>/dev/null | tail -3 | cut -c1-200", 40))
print("-- Back")
print(p(D + "ctl remote 412 tap; sleep 3", 30).strip()[:100]); print("fg after Back:", fg())
print(p("luna-send -n 1 -f luna://com.webos.applicationManager/closeByAppId '{\"id\":\"com.webos.app.livetv\"}'; luna-send -n 1 -f luna://com.webos.applicationManager/closeByAppId '{\"id\":\"com.webos.app.livedmost\"}'; sleep 2; luna-send -n 1 -f luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.home\"}' >/dev/null; sleep 3", 60).strip()[:200])
print("fg end:", fg())
print(p("%s inputs status | head -c 120" % tv_gate.LUMATV_TOOL, 40))
