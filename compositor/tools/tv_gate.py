#!/usr/bin/env python3
"""Drives the C4 gate from the workstation: LumaTV's compositor goes live (cutover.sh enable), the checks of
tools/tv-gate.sh run in order with the receiver muted, the set is woken after the standby check, and the escape
check ends the live session. Exit 0 only when every check passed; the result lines and files land in
/tmp/lumatv-tv-out/out/gate and docs/research/compositor/gate-<date>.txt.

    tv_gate.py [--skip netflix,hdmi,...] [--x1-seconds N] [--enable-after]

--enable-after re-enables LumaTV's compositor at the end when every check passed (Nitin's decision: live once the
gate passes); without it LG's compositor stays after the escape check."""
import argparse, os, socket, subprocess, sys, time, json, urllib.request
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'docs', 'research', 'arch'))
import tv, paramiko

TV_MAC = "74:c1:7e:bd:ae:30"
HA = "http://192.168.1.146"
AVR = "media_player.living_room_avr"

def pty(cmd, timeout=600):
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    try:
        _, out, _ = c.exec_command(cmd, timeout=timeout, get_pty=True)
        return out.read().decode("utf-8", "replace")
    finally:
        c.close()

def wol():
    mac = bytes.fromhex(TV_MAC.replace(":", ""))
    pkt = b"\xff" * 6 + mac * 16
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    for _ in range(3):
        s.sendto(pkt, ("192.168.1.255", 9)); s.sendto(pkt, ("255.255.255.255", 9)); time.sleep(0.2)
    s.close()

def ha_token():
    subprocess.run(["sh", "/data/home/tmp/ha-token-fetch.sh"], capture_output=True)
    return open("/data/home/tmp/ha-token").read().strip()

def avr_mute(on):
    try:
        req = urllib.request.Request(HA + "/api/services/media_player/volume_mute", data=json.dumps({"entity_id": AVR, "is_volume_muted": on}).encode(),
                                     headers={"Authorization": "Bearer " + ha_token(), "Content-Type": "application/json"})
        urllib.request.urlopen(req, timeout=10).read()
        print("avr: %s" % ("muted" if on else "unmuted"))
    except Exception as e:
        print("avr mute %s failed: %s" % (on, e))

def wait_ssh(seconds):
    t0 = time.time()
    while time.time() - t0 < seconds:
        try:
            if tv.tv("echo up", timeout=8).strip().startswith("up"):
                return True
        except Exception:
            pass
        time.sleep(3)
    return False

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip", default="")
    ap.add_argument("--x1-seconds", type=int, default=60)
    ap.add_argument("--enable-after", action="store_true")
    a = ap.parse_args()
    skip = set(a.skip.split(",")) - {""}
    here = os.path.dirname(os.path.abspath(__file__))
    tv.put(os.path.join(here, "tv-gate.sh"), "/tmp/lumatv/gate.sh")
    tv.put(os.path.join(here, "tv-cutover.sh"), "/tmp/lumatv/cutover.sh")
    tv.put(os.path.join(here, "tv-recover-home.sh"), "/tmp/lumatv/recover-home.sh")
    tv.tv("rm -rf /tmp/lumatv/out/gate; mkdir -p /tmp/lumatv/out/gate")
    lines = []
    def step(name, cmd, timeout=600):
        out = pty(cmd, timeout)
        print(out)
        for l in out.splitlines():
            if l.startswith("GATE "):
                lines.append(l)
        return out
    avr_mute(True)
    try:
        print(pty("sh /tmp/lumatv/cutover.sh enable", 300))
        step("live", "sh /tmp/lumatv/gate.sh live")
        if "x1" not in skip:
            step("x1", "sh /tmp/lumatv/gate.sh x1 %d" % a.x1_seconds, a.x1_seconds + 120)
        if "netflix" not in skip:
            step("netflix", "sh /tmp/lumatv/gate.sh netflix")
        if "hdmi" not in skip:
            step("hdmi", "sh /tmp/lumatv/gate.sh hdmi")
        if "standby" not in skip:
            step("standby", "sh /tmp/lumatv/gate.sh standby", 60)
            time.sleep(25)
            wol(); time.sleep(5); wol()
            if not wait_ssh(90):
                lines.append("GATE standby: FAIL no SSH after the wake")
            else:
                step("woke", "sh /tmp/lumatv/gate.sh woke", 180)
        if "escape" not in skip:
            step("escape", "sh /tmp/lumatv/gate.sh escape", 300)
        else:
            print(pty("sh /tmp/lumatv/cutover.sh disable", 300))
    finally:
        avr_mute(False)
    ok = lines and all(": PASS" in l for l in lines)
    print("\n".join(lines))
    print("GATE: %s" % ("PASS" if ok else "FAIL"))
    if ok and a.enable_after:
        print(pty("rm -f /var/lib/lumatv/compositor/escaped; sh /tmp/lumatv/cutover.sh enable", 300))
        print(pty("sh /tmp/lumatv/gate.sh live", 120))
    tv.tv("cd /tmp/lumatv && tar czf /tmp/lumatv-gate.tgz out/gate")
    os.makedirs("/tmp/lumatv-tv-out", exist_ok=True)
    tv.get("/tmp/lumatv-gate.tgz", "/tmp/lumatv-tv-out/gate.tgz")
    os.system("tar -C /tmp/lumatv-tv-out -xzf /tmp/lumatv-tv-out/gate.tgz")
    day = time.strftime("%Y-%m-%d")
    doc = os.path.join(here, "..", "..", "docs", "research", "compositor", "gate-%s.txt" % day)
    with open(doc, "a") as f:
        f.write("# C4 gate %s\n" % time.strftime("%Y-%m-%d %H:%M"))
        f.write("\n".join(lines) + "\nGATE: %s\n\n" % ("PASS" if ok else "FAIL"))
    print("wrote", os.path.relpath(doc))
    sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main()
