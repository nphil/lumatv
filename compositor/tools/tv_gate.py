#!/usr/bin/env python3
"""Drives the C4 gate from the workstation: LumaTV's compositor goes live as a session (cutover.sh enable --session: this
boot only, on any firmware), the checks of tools/tv-gate.sh run in order with the receiver muted, the set is woken after
the standby check, and the checks that end the live session (Home held on a virtual remote, the compositor stopped and
killed, the compositor's own hold-Home escape) each get a fresh session. Exit 0 only when every check passed; the result
lines and files land in /tmp/lumatv-tv-out/out/gate and docs/research/compositor/gate-<date>.txt.

    tv_gate.py [--skip netflix,hdmi,...] [--x1-seconds N] [--enable-after]

The gate's rule for the frame check is 60 s with no gap over 20 ms (tv-gate.sh, "The frame rule"); --x1-seconds under 60
can only fail it. Checks that can be skipped: firmware, homeheld, volume, x1, netflix, hdmi, standby, homekey, wd-hang,
wd-crash, escape.

Afterwards LG's compositor is live and the enabled flag is removed, whatever failed. --enable-after (Nitin's decision:
live once the gate passes) takes the firmware build the gate has just passed on into compositor/firmware/verified.list
(the working tree: commit it), installs it on the set, and enables LumaTV's compositor for good; a pass on another build
than the list holds is what makes that build one LumaTV may start on at boot."""
import argparse, os, re, socket, subprocess, sys, time, json, urllib.request
here = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(here, '..', '..', 'docs', 'research', 'arch'))
import tv, paramiko

TV_MAC = "74:c1:7e:bd:ae:30"
HA = "http://192.168.1.146"
AVR = "media_player.living_room_avr"
VERIFIED_LIST = os.path.join(here, "..", "firmware", "verified.list")
LIBRARY = os.path.join(here, "..", "..", "tools", "compositor.sh")

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

def entry_from(text):
    """The verified.list entry `cutover.sh firmware record` printed, from its `build` line to its `end`, without the
    carriage returns a PTY adds; None when there is none."""
    out, on = [], False
    for line in (l.rstrip("\r") for l in text.splitlines()):
        if line.startswith("build "):
            out, on = [line], True
        elif on:
            out.append(line)
            if line == "end":
                return "\n".join(out) + "\n"
    return None

def entry_problem(entry):
    """Why an entry must not be listed, or None: it needs the release line, a 16-digit fingerprint and a SHA-256 for every
    file (a `-` stands for a file that could not be read, and listing it would verify a build nobody hashed)."""
    lines = entry.splitlines()
    files = [l.split() for l in lines if l.startswith("file ")]
    if not any(l.startswith("release ") and len(l) > 8 for l in lines):
        return "no release line"
    fp = [l.split()[1] for l in lines if l.startswith("fingerprint ") and len(l.split()) > 1]
    if not fp or len(fp[0]) != 16 or any(c not in "0123456789abcdef" for c in fp[0]):
        return "no 16-digit fingerprint"
    if len(files) < 9:
        return "only %d file lines" % len(files)
    for f in files:
        if len(f) != 4 or len(f[1]) != 64 or any(c not in "0123456789abcdef" for c in f[1]):
            return "file %s has no SHA-256" % (f[3] if len(f) > 3 else "?")
    return None

def add_entry(path, entry):
    """Appends an entry to the list unless one with its fingerprint is there. Returns "added", "listed" or the reason it is
    refused."""
    problem = entry_problem(entry)
    if problem:
        return "refused: " + problem
    fp = [l.split()[1] for l in entry.splitlines() if l.startswith("fingerprint ")][0]
    try:
        with open(path) as f:
            text = f.read()
    except OSError:
        text = ""
    if any(l.split()[:2] == ["fingerprint", fp] for l in text.splitlines()):
        return "listed"
    body = text.rstrip("\n")
    with open(path, "w") as f:
        f.write(body + "\n\n" + entry if body else entry)
    return "added"

VERDICT = re.compile(r"^GATE [^:]+: (PASS|FAIL)\b")


def all_passed(lines):
    """The gate passes when it has verdicts and every one is PASS. A line that is no verdict (`GATE standby: requested`,
    which only says the power-off call went out) is not one: counted as a failure, it made the gate unpassable."""
    verdicts = [VERDICT.match(l) for l in lines if VERDICT.match(l)]
    return bool(verdicts) and all(m.group(1) == "PASS" for m in verdicts)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip", default="")
    ap.add_argument("--x1-seconds", type=int, default=60)
    ap.add_argument("--enable-after", action="store_true")
    a = ap.parse_args()
    skip = set(a.skip.split(",")) - {""}
    tv.put(os.path.join(here, "tv-gate.sh"), "/tmp/lumatv/gate.sh")
    tv.put(os.path.join(here, "tv-cutover.sh"), "/tmp/lumatv/cutover.sh")
    tv.put(os.path.join(here, "tv-recover-home.sh"), "/tmp/lumatv/recover-home.sh")
    tv.put(os.path.join(here, "tv-uinput-key.py"), "/tmp/lumatv/uinput-key.py")
    tv.put(LIBRARY, "/tmp/lumatv/compositor.sh")
    if os.path.exists(VERIFIED_LIST):
        tv.put(VERIFIED_LIST, "/tmp/lumatv/verified.list")
    tv.tv("rm -rf /tmp/lumatv/out/gate; mkdir -p /tmp/lumatv/out/gate")
    lines = []
    def step(name, cmd, timeout=600):
        out = pty(cmd, timeout)
        print(out)
        for l in out.splitlines():
            if l.startswith("GATE "):
                lines.append(l.rstrip("\r"))
        return out
    def session(label=None):
        """LumaTV's compositor live for this boot only, then `live` says so (the label names the line when it is a
        second session of the run)."""
        print(pty("sh /tmp/lumatv/cutover.sh enable --session", 300))
        step("live", "sh /tmp/lumatv/gate.sh live%s" % (" " + label if label else ""))
    if not wake():
        sys.exit("the set does not answer; nothing was changed")
    avr_mute(True)
    try:
        # Not live yet: the firmware check on the set's own files, and the supervisor's question about a held Home key.
        if "firmware" not in skip:
            step("firmware", "sh /tmp/lumatv/gate.sh firmware", 300)
        if "homeheld" not in skip:
            step("homeheld", "sh /tmp/lumatv/gate.sh homeheld", 120)
        session()
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
        # Each of these ends the live session (that is what they test), so each one that follows gets a fresh one.
        ended = ["homekey", "wd-hang", "wd-crash"]
        for name in ended:
            if name in skip:
                continue
            step(name, "sh /tmp/lumatv/gate.sh %s" % name, 300)
            session("after-" + name)
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
    ok = all_passed(lines)
    print("\n".join(lines))
    print("GATE: %s" % ("PASS" if ok else "FAIL"))
    if ok and a.enable_after:
        # The gate has passed on this firmware: it is a build LumaTV's compositor may start on at boot. The entry comes
        # from the set's own files; it is added to the list in the working tree (to be committed), the list goes to the set
        # with the install, and only then does the persistent enable find the build verified.
        record = pty("sh /tmp/lumatv/cutover.sh firmware record", 300)
        entry = entry_from(record)
        result = add_entry(VERIFIED_LIST, entry) if entry else "refused: no entry in the set's answer"
        print("verified.list: %s" % result)
        if result.startswith("refused"):
            print("not enabling: %s" % record[-600:])
        else:
            tv.put(VERIFIED_LIST, "/tmp/lumatv/verified.list")
            print(pty("sh /tmp/lumatv/cutover.sh install", 300))
            print(pty("sh /tmp/lumatv/cutover.sh enable", 300))
            print(pty("sh /tmp/lumatv/gate.sh live enabled", 120))
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
