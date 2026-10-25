#!/usr/bin/env python3
"""Slots 5 (Denon telnet link), 6 (LG input banners restyled) and 7 (banners over video, gear hold) on the set, one SSH
session each, commands batched; evidence (text and PNG captures) under OUT/<slot>/."""
import json, os, sys, time, urllib.request
W = "/tmp/lumatv-slot"  # the clean worktree at HEAD the round was run from
sys.path.insert(0, W + "/compositor/tools"); sys.path.insert(0, W + "/docs/research/arch")
import paramiko, tv, tv_gate
APP = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv"
TOOL = APP + "/tools/lumatv"
HDMI2 = "com.webos.app.hdmi2"
HOME = "com.webos.app.home"


class S:
    def __init__(self, out):
        self.out = out; os.makedirs(out, exist_ok=True)
        self.log = open(os.path.join(out, "log.txt"), "a")
        self.c = paramiko.SSHClient(); self.c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
        self.c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
        self.sftp = self.c.open_sftp()
        self.t0 = time.time()

    def say(self, s):
        line = "[%6.1f] %s" % (time.time() - self.t0, s)
        print(line, flush=True); self.log.write(line + "\n"); self.log.flush()

    def run(self, cmd, timeout=60, pty=False):
        _, o, e = self.c.exec_command(cmd, timeout=timeout, get_pty=pty)
        r = o.read().decode("utf-8", "replace")
        if not pty:
            err = e.read().decode("utf-8", "replace")
            if err.strip():
                r += "\n[stderr] " + err
        return r.replace("\r", "")

    def luna(self, uri, payload="{}", timeout=30):
        return self.run("luna-send -n 1 -f %s '%s'" % (uri, payload), timeout, pty=True)

    def launch(self, app_id):
        return self.luna("luna://com.webos.applicationManager/launch", json.dumps({"id": app_id})).strip()[:200]

    def foreground(self):
        r = self.luna("luna://com.webos.applicationManager/getForegroundAppInfo")
        for l in r.splitlines():
            if '"appId"' in l:
                return l.split(":", 1)[1].strip().strip('",')
        return r.strip()[:80]

    def shot(self, name, w=960, h=540):
        r = self.run("luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{\"path\":\"/tmp/r7-%s.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":%d,\"height\":%d}'" % (name, w, h), 30, pty=True)
        local = os.path.join(self.out, name + ".png")
        try:
            self.sftp.get("/tmp/r7-%s.png" % name, local)
            self.say("shot %s: %d bytes" % (name, os.path.getsize(local)))
        except Exception as ex:
            self.say("shot %s failed: %s %s" % (name, ex, r.strip()[:120]))

    def kb(self, *tokens):
        return self.run("python3 /tmp/kb.py " + " ".join(tokens), 60).strip()

    def put(self, local, remote):
        self.sftp.put(local, remote); self.sftp.chmod(remote, 0o755)

    def close(self):
        self.sftp.close(); self.c.close()


def ha(path, data=None):
    req = urllib.request.Request(tv_gate.HA + path, data=json.dumps(data).encode() if data is not None else None,
                                 headers={"Authorization": "Bearer " + tv_gate.ha_token(), "Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=10).read())


def avr():
    j = ha("/api/states/" + tv_gate.AVR)
    return j["state"], j["attributes"].get("volume_level"), j["attributes"].get("is_volume_muted")


# ---------------------------------------------------------------- slot 5: the receiver's telnet link
def slot5(out):
    s = S(out)
    s.say("slot 5 start; receiver %s %s muted %s" % avr())
    s.put("/tmp/lt/kb.py", "/tmp/kb.py")
    # 192.168.1.93 = 5D01A8C0 in /proc/net/tcp, port 23 = 0017, 8080 = 1F90; st 01 = ESTABLISHED
    conns = "grep -i '5D01A8C0:0017\\|5D01A8C0:1F90' /proc/net/tcp | awk '{print $2, $3, $4}'"
    s.say("helper log receiver lines:\n" + s.run("grep -n 'receiver' /var/lib/lumatv/log-helper | tail -8"))
    s.say("connections to the receiver (local remote state):\n" + s.run(conns))
    s.say("helper pid/uptime: " + s.run("cat /var/lib/lumatv/helper.lock 2>/dev/null; pgrep -f lumatv-helper | head -2 | tr '\\n' ' '; ps -o etime= -p $(pgrep -f 'bin/lumatv-helper|helper/lumatv-helper' | head -1) 2>/dev/null").strip())
    rd = "wget -qO- http://127.0.0.1:18731/v1/receiver 2>&1 || curl -s http://127.0.0.1:18731/v1/receiver"
    base = s.run(rd).strip()
    s.say("/v1/receiver before: " + base[:400])

    # one Volume+ from the remote's key device, the topic read at once and every 0.4 s after
    _, v0, _ = avr()
    r = s.run("python3 /tmp/kb.py 115 >/dev/null; " + "for i in 0 1 2 3 4 5 6 7; do echo \"+$((i*4))00ms $(" + rd + ")\"; sleep 0.4; done", 60)
    s.say("after Volume+ (key 115), the topic every 0.4 s:\n" + r)
    time.sleep(1.5)
    s.say("HA after Volume+: %s %s muted %s (was %s)" % (avr() + (v0,)))
    r = s.run("python3 /tmp/kb.py 114 >/dev/null; " + "for i in 0 1 2 3 4 5 6 7; do echo \"+$((i*4))00ms $(" + rd + ")\"; sleep 0.4; done", 60)
    s.say("after Volume- (key 114):\n" + r)
    time.sleep(1.5)
    s.say("HA after Volume-: %s %s muted %s" % avr())
    # a change made from Home Assistant: volume_up, then volume_down
    t = time.time(); ha("/api/services/media_player/volume_up", {"entity_id": tv_gate.AVR})
    r = s.run("for i in 0 1 2 3 4 5 6 7; do echo \"+$((i*4))00ms $(" + rd + ")\"; sleep 0.4; done", 60)
    s.say("after HA volume_up at %.3f (set clock below), the topic:\n%s" % (t, r))
    ha("/api/services/media_player/volume_down", {"entity_id": tv_gate.AVR})
    time.sleep(2)
    s.say("after HA volume_down: HA %s %s muted %s; topic %s" % (avr() + (s.run(rd).strip()[:300],)))
    s.say("connections after: \n" + s.run(conns))
    s.say("helper log tail:\n" + s.run("tail -6 /var/lib/lumatv/log-helper"))
    s.say("HA media_player state at the end: %s %s muted %s" % avr())
    s.say("slot 5 end; foreground " + s.foreground())
    s.close()


# ---------------------------------------------------------------- slot 6: LG's input banners restyled (inputcommon)
def slot6(out):
    s = S(out)
    s.say("slot 6 start; foreground " + s.foreground())
    s.put("/tmp/lt/kb.py", "/tmp/kb.py")
    s.say("QML dir: " + s.run("ls -d /usr/palm/applications/com.webos.app.inputcommon/qml 2>&1; d=/usr/palm/applications/com.webos.app.inputcommon/qml; "
                              "find $d -name '*.qml' 2>/dev/null | wc -l; md5sum $d/UserInterfaceLayer/GlobalStyle.qml 2>&1; "
                              "pidof com.webos.app.inputcommon; ls /mnt/lg/cmn_data/checkpoint/ 2>&1 | grep -i inputcommon").strip())
    s.say("census GlobalStyle md5: " + os.popen("md5sum /data/home/lgtv/uiinv/inputcommon/qml/UserInterfaceLayer/GlobalStyle.qml").read().strip())
    s.say("key device caps: " + s.run("for b in /sys/class/input/event*; do n=$(cat $b/device/name); case \"$n\" in *RCU*|*Network*) echo \"$b $n: $(cat $b/device/capabilities/key)\";; esac; done").strip()[:600])
    # HEAD's tools/inputcommon.sh and tools/inputcommon_patch.py are not in the ipk (tools/build-ipk.sh does not list them); for this
    # slot the two committed files are put in the installed app's tools/ and removed at the end, so the set matches the ipk again
    for f in ("inputcommon.sh", "inputcommon_patch.py"):
        s.put(W + "/tools/" + f, APP + "/tools/" + f)
    s.say("put HEAD's inputcommon.sh and inputcommon_patch.py into the app dir: " + s.run("md5sum %s/tools/inputcommon.sh %s/tools/inputcommon_patch.py" % (APP, APP)).strip())
    s.say("inputcommon.sh status direct: " + s.run("sh %s/tools/inputcommon.sh status 2>&1; echo exit=$?" % APP).strip()[:400])
    s.say("reports before: " + s.run("ls /tmp/var/log/reports/librdx/ 2>&1 | tr '\\n' ' '").strip()[:300])
    r = s.run("%s inputcommon on 2>&1; echo exit=$?" % TOOL, 180, pty=True).strip()
    s.say("inputcommon on: " + r[:600])
    s.say("mounts: " + s.run("grep inputcommon /proc/mounts").strip()[:400])
    s.say("tool log tail: " + s.run("tail -8 /var/lib/lumatv/log 2>/dev/null; tail -5 /var/lib/lumatv/inputcommon/log 2>/dev/null").strip()[:900])
    if '"ok":true' not in r.replace(" ", ""):
        s.say("inputcommon on refused; no HDMI switch")
        s.run("rm -f %s/tools/inputcommon.sh %s/tools/inputcommon_patch.py" % (APP, APP))
        s.say("slot 6 end; foreground " + s.foreground()); s.close(); return
    # the switch to HDMI 2 from Home: the pill (4 s), then OK (the banner with the options key), Right, INFO (the detail card)
    s.say("launch hdmi2: " + s.launch(HDMI2)); time.sleep(1.6)
    s.shot("pill"); time.sleep(0.6)
    s.say("foreground " + s.foreground() + "; inputcommon pid " + s.run("pidof com.webos.app.inputcommon").strip())
    time.sleep(4)
    s.kb("OK"); time.sleep(1.2); s.shot("ok-banner")
    s.kb("RIGHT"); time.sleep(0.8); s.shot("ok-right")
    s.kb("BACK"); time.sleep(1.0)
    s.kb("358"); time.sleep(1.5); s.shot("info")
    s.kb("BACK"); time.sleep(1.0)
    s.say("launch home: " + s.launch(HOME)); time.sleep(2.5)
    s.say("foreground " + s.foreground())
    s.say("inputcommon off: " + s.run("%s inputcommon off 2>&1; echo exit=$?" % TOOL, 120, pty=True).strip()[:400])
    s.say("mounts after: [" + s.run("grep inputcommon /proc/mounts").strip() + "]")
    s.say("reports after: " + s.run("ls /tmp/var/log/reports/librdx/ 2>&1 | tr '\\n' ' '").strip()[:300])
    s.say("inputcommon alive: " + s.run("pidof com.webos.app.inputcommon; grep -c inputcommon /var/log/messages 2>/dev/null; grep -i 'inputcommon' /var/log/messages 2>/dev/null | grep -i 'crash\\|segv\\|fault' | tail -3").strip()[:400])
    s.say("inputcommon status: " + s.run("%s inputcommon status 2>&1" % TOOL, 60, pty=True).strip()[:300])
    s.say("removed the two scripts again: " + s.run("rm -f %s/tools/inputcommon.sh %s/tools/inputcommon_patch.py; ls %s/tools/ | tr '\\n' ' '" % (APP, APP, APP)).strip()[:300])
    s.say("slot 6 end; foreground " + s.foreground())
    s.close()


# ---------------------------------------------------------------- slot 7: banners over video, the gear hold
def slot7(out):
    s = S(out)
    s.say("slot 7 start; foreground " + s.foreground())
    s.put("/tmp/lt/kb.py", "/tmp/kb.py")
    s.say("QML log lines: " + s.run("grep -E '\\[Luma(Keys|Home|Overlay)\\]|LumaOverlay failed' /var/log/messages | tail -8").strip()[:900])
    s.say("theme before: " + s.run("%s theme get" % TOOL, 60, pty=True).strip()[:300])
    # the gear on Home: a tap is LG's Quick Settings, a hold of 650 ms the drawer
    s.kb("MENU"); time.sleep(1.5); s.shot("gear-tap-home")
    s.say("after gear tap: foreground " + s.foreground() + "; log: " + s.run("grep -E '\\[LumaKeys\\]' /var/log/messages | tail -2").strip()[:300])
    s.kb("BACK"); time.sleep(1.0)
    s.kb("MENU:1000"); time.sleep(1.2); s.shot("gear-hold-home")
    s.say("after gear hold: " + s.run("grep -E '\\[LumaKeys\\]|\\[LumaHome\\]' /var/log/messages | tail -3").strip()[:400])
    s.kb("BACK"); time.sleep(1.0)
    # banners.own on; the switch to HDMI 2 shows the pill at the bottom left for 4 s
    s.say("banners.own on: " + s.run("%s theme set '{\"banners\":{\"own\":true}}'" % TOOL, 60, pty=True).strip()[:200])
    time.sleep(2.5)
    s.say("launch hdmi2: " + s.launch(HDMI2)); time.sleep(1.8)
    s.shot("own-pill"); time.sleep(3)
    spec = '{"kind":"input","icon":"cable","name":"HDMI 2","facts":"","buttons":true,"actions":["inputs","picture"],"holdMs":60000}'
    s.run("printf '%%s' '%s' > /tmp/lumatv-overlay" % spec); time.sleep(1.8); s.shot("door-banner")
    s.kb("RIGHT"); time.sleep(0.8); s.shot("door-banner-right")
    s.kb("OK"); time.sleep(1.8); s.shot("door-ok-quicksettings")
    s.say("after OK on the banner: foreground " + s.foreground())
    s.kb("BACK"); time.sleep(1.2); s.kb("BACK"); time.sleep(1.0)
    s.run("printf '%s' '{\"open\":\"drawer\"}' > /tmp/lumatv-overlay"); time.sleep(1.8); s.shot("door-drawer")
    s.kb("DOWN"); time.sleep(0.6); s.shot("door-drawer-down")
    s.kb("BACK"); time.sleep(1.2)
    s.run("rm -f /tmp/lumatv-overlay")
    # the gear held over the input: the drawer over the picture; Back closes it
    s.kb("MENU:1000"); time.sleep(1.2); s.shot("gear-hold-hdmi")
    s.say("after gear hold over hdmi2: " + s.run("grep -E '\\[LumaKeys\\]|\\[LumaOverlay\\]' /var/log/messages | tail -3").strip()[:500] + "; foreground " + s.foreground())
    s.kb("BACK"); time.sleep(1.2); s.shot("gear-hold-hdmi-closed")
    s.say("launch home: " + s.launch(HOME)); time.sleep(2.5)
    s.say("banners.own off: " + s.run("%s theme set '{\"banners\":{\"own\":false}}'" % TOOL, 60, pty=True).strip()[:200])
    s.say("theme after: " + s.run("%s theme get" % TOOL, 60, pty=True).strip()[:300])
    s.say("fps/jank lines: " + s.run("grep -c 'FPSJANK' /var/log/messages 2>/dev/null; grep -E 'LumaOverlay failed|\\[LumaKeys\\].*failed' /var/log/messages | tail -2").strip()[:300])
    s.say("slot 7 end; foreground " + s.foreground())
    s.close()


if __name__ == "__main__":
    {"5": slot5, "6": slot6, "7": slot7}[sys.argv[1]](sys.argv[2])
