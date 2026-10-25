#!/usr/bin/env python3
"""Drives the C4 gate from the workstation: LumaTV's compositor goes live (cutover.sh enable), the checks of
tools/tv-gate.sh run in order with the receiver muted, the set is woken after the standby check, and the escape
check ends the live session. Exit 0 only when every check passed; the result lines and files land in
/tmp/lumatv-tv-out/out/gate and docs/research/compositor/gate-<date>.txt.

    tv_gate.py [--skip netflix,hdmi,...] [--x1-seconds N] [--enable-after]

Afterwards LG's compositor is live and the enabled flag is removed, whatever failed. --enable-after re-installs
and re-enables LumaTV's compositor when every check passed (Nitin's decision: live once the gate passes)."""
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

def avr_volume():
    try:
        req = urllib.request.Request(HA + "/api/states/" + AVR, headers={"Authorization": "Bearer " + ha_token()})
        return json.loads(urllib.request.urlopen(req, timeout=10).read()).get("attributes", {}).get("volume_level")
    except Exception as e:
        print("avr state failed: %s" % e)
        return None

def volume_check():
    """The remote's Volume keys under LumaTV's compositor: one step up and one down through the compositor's own
    routing (the audio service passes them to the receiver over CEC); the receiver's level, read from Home
    Assistant, must rise and come back. Mute is left alone: on ARC it would toggle the receiver's mute, which the
    gate holds on."""
    before = avr_volume()
    pty("sh /tmp/lumatv/gate.sh key 115", 30); time.sleep(3)
    up = avr_volume()
    pty("sh /tmp/lumatv/gate.sh key 114", 30); time.sleep(3)
    back = avr_volume()
    detail = "receiver volume_level before %s after Volume+ %s after Volume- %s" % (before, up, back)
    ok = None not in (before, up, back) and up > before and abs(back - before) < 1e-6
    return "GATE volume: %s %s" % ("PASS" if ok else "FAIL", detail)

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

def power_state():
    out = pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}'", 20)
    for l in out.splitlines():
        if '"state"' in l:
            return l.split(":", 1)[1].strip().strip('",').lower()
    return out.strip()[:80]

def wake():
    """Active Standby keeps SSH up, so the set is woken the way the X4 session did (powerOn, then the screen);
    a set that has gone on to suspend gets the magic packet first."""
    if not wait_ssh(10):
        wol(); time.sleep(5); wol()
        if not wait_ssh(90):
            return False
    st = power_state()
    print("power state:", st)
    if st == "active":
        return True
    pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/powerOn '{\"reason\":\"remoteKey\"}'", 20)
    time.sleep(20)
    pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/turnOnScreen '{}'", 20)
    time.sleep(8)
    print("power state after wake:", power_state())
    return True

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
    if not wake():
        sys.exit("the set does not answer; nothing was changed")
    avr_mute(True)
    try:
        print(pty("sh /tmp/lumatv/cutover.sh enable", 300))
        step("live", "sh /tmp/lumatv/gate.sh live")
        if "volume" not in skip:
            line = volume_check()
            print(line)
            lines.append(line)
        if "x1" not in skip:
            step("x1", "sh /tmp/lumatv/gate.sh x1 %d" % a.x1_seconds, a.x1_seconds + 120)
        if "netflix" not in skip:
            avr_mute(True)   # the receiver may have been off (no mute to take) when the first one was sent
            step("netflix", "sh /tmp/lumatv/gate.sh netflix")
        if "hdmi" not in skip:
            step("hdmi", "sh /tmp/lumatv/gate.sh hdmi")
        step("evidence", "sh /tmp/lumatv/gate.sh evidence", 120)
        if "standby" not in skip:
            step("standby", "sh /tmp/lumatv/gate.sh standby", 60)
            time.sleep(25)
            if not wake():
                lines.append("GATE standby: FAIL no SSH after the wake")
                raise SystemExit("the set did not come back; LumaTV's compositor may still be enabled on it")
            step("woke", "sh /tmp/lumatv/gate.sh woke", 180)
        if "escape" not in skip:
            step("escape", "sh /tmp/lumatv/gate.sh escape", 300)
    finally:
        # Whatever happened above, LG's compositor is the live one afterwards with the flag removed (a failed
        # escape would otherwise leave LumaTV's running); --enable-after turns it back on below after a pass.
        try:
            print(pty("sh /tmp/lumatv/cutover.sh disable", 300))
        except Exception as e:
            print("disable failed: %s" % e)
        # The session's whole log (the escape and the fallback included) stays in the state directory until the next
        # start of LumaTV's compositor truncates it.
        tv.tv("cp /var/lib/lumatv/compositor/compositor.log /tmp/lumatv/out/gate/compositor-final.log; cp /var/lib/lumatv/compositor/sm.log /tmp/lumatv/out/gate/sm-final.log")
        avr_mute(False)
    ok = lines and all(": PASS" in l for l in lines)
    print("\n".join(lines))
    print("GATE: %s" % ("PASS" if ok else "FAIL"))
    if ok and a.enable_after:
        print(pty("sh /tmp/lumatv/cutover.sh install > /dev/null; sh /tmp/lumatv/cutover.sh enable", 300))
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
