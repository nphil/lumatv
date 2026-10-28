#!/usr/bin/env python3
"""Drives the C4 gate from the workstation: LumaTV's compositor goes live as a session (cutover.sh enable --session: this
boot only, on any firmware), the checks of tools/tv-gate.sh run in order with the receiver muted, the set is woken after
the standby check, and the checks that end the live session (Home held on a virtual remote, the compositor stopped and
killed, the compositor's own hold-Home escape) each get a fresh session. Exit 0 only when every check passed; the result
lines and files land in /tmp/lumatv-tv-out/out/gate and docs/research/compositor/gate-<date>.txt.

    tv_gate.py [--skip netflix,hdmi,...] [--only keys,ime] [--x1-seconds N] [--enable-after] [--frames-callbacks landing] [--refresh 120]

The gate's rule for the frame check is 60 s with no gap over 20 ms (tv-gate.sh, "The frame rule"); --x1-seconds under 60
can only fail it. Checks that can be skipped: firmware, homeheld, volume, x1, netflix, hdmi, keys, ime, standby, homekey,
wd-hang, wd-crash, escape. --only runs just the named ones (in one session, the live check and the evidence included); the
step-1 test of the remote's keys and the keyboard is `--only keys,ime`. --frames-callbacks landing runs the surface frame gate with
the shell client's frame callbacks at the flip like every client's (`ctl callbacks shell landing`), against the default of a frame
ahead of it inside a run of its frames, so that a round says what the change made.
--refresh 120 runs the surface frame gate in the compositor's refresh test mode (docs/research/ui120-result.md): `gate.sh refresh 120`
before it (the OSD vblank divider at 1, the pacer at 8.33 ms; a GATE refresh line with the rate measured three ways, Mali busy and the
temperature at idle Home), the gate's dropped-frame threshold at 10 ms, idle Home measured again after the surfaces, then
`gate.sh refresh 60` (and once more in the clean-up, whatever happened). `--only frames --refresh 120` is the whole 120 Hz check.

Every session carries the set's own safety timer (cutover.sh enable --session --max-minutes): if this driver or the
workstation dies in the middle of the gate, the set ends the session by itself and LG's compositor and Home come back.

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
# lumatv's own tool on the set (`homestub on|off|status`).
LUMATV_TOOL = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"

# Minutes a session may live before the set ends it (--max-minutes). The first session (live, volume, x1's 60 s, netflix,
# hdmi, standby and wake, then the key and keyboard checks) ran 4 minutes in the gate of 2026-10-26 (supervisor starts
# 14:03:11 to 14:07:12), each session that follows an ending check 33-35 s; each gets at least three times that, so that a
# healthy run never meets its own timer, which would end it under the checks.
FIRST_SESSION_MINUTES = 12
LATER_SESSION_MINUTES = 6
# Every check that can be skipped or picked with --only, in the order the gate runs them.
CHECKS = ["firmware", "homeheld", "volume", "x1", "netflix", "hdmi", "keys", "ime", "shell", "frames", "frames-load", "standby", "homekey", "wd-hang", "wd-crash", "escape"]
# The surface frame gate (docs/research/perf/gate.py) under LumaTV's compositor: LumaTV's own surfaces, drawn by the shell client.
FRAMES_SURFACES = "home-dpad,quick,picker,toast"
FRAMES_SESSION_MINUTES = 20
PERF_GATE = os.path.join(here, "..", "..", "docs", "research", "perf", "gate.py")
PERF_DIR = os.path.dirname(PERF_GATE)

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

def homestub_status():
    """What `lumatv homestub status` prints: `"state":"stub"` while the Home stub is mounted and guarding Home, `"state":"lg"` after a
    fault gave LG's Home back."""
    return pty("%s homestub status 2>&1 | head -3" % LUMATV_TOOL, 60).strip()

def homestub_is_on(status):
    return '"state":"stub"' in status

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

def frames_check(cycles, load=False, refresh=60):
    """The surface frame gate (`docs/research/perf/gate.py`) over LumaTV's own surfaces under its compositor: Home's D-pad, the quick
    panel and the input picker (the gear and Input through the shell client) and a toast. Its verdict line, with each surface's
    failed checks, becomes `GATE frames:`. With `load` the same surfaces are measured under synthetic background load (a camera-like
    decode, Home Assistant websocket chatter, a BLE scan stand-in, a low-priority download: `lumatv-loadgen`, started and stopped by
    the gate), and the line is `GATE frames-load:`, a report: no baseline of loaded numbers exists yet, so the unloaded thresholds'
    verdict is told but is no verdict of the gate's."""
    out_json = "/tmp/lumatv-tv-out/frames%s%s.json" % ("-load" if load else "", "-%d" % refresh if refresh != 60 else "")
    os.makedirs("/tmp/lumatv-tv-out", exist_ok=True)
    cmd = [sys.executable, PERF_GATE, "--surfaces", FRAMES_SURFACES, "--input-key", "241", "--cycles", str(cycles), "--json", out_json,
           "--tag", "c4shell-load" if load else ("c4shell-%d" % refresh if refresh != 60 else "c4shell")] + (["--load", "decode,ws,ble,download"] if load else []) \
          + (["--refresh", str(refresh)] if refresh != 60 else [])
    name = "frames-load" if load else "frames"
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=1500)
        out = r.stdout + r.stderr
    except subprocess.TimeoutExpired:
        return "GATE %s: FAIL the surface gate did not finish in 25 minutes" % name
    print(out[-4000:])
    verdict = "FAIL"
    for l in out.splitlines():
        if l.startswith("VERDICT "):
            verdict = l.split()[1]
    detail = ""
    try:
        with open(out_json) as f:
            doc = json.load(f)
        parts = []
        for res in doc.get("surfaces", []):
            checks = res.get("checks") or []
            failed = ["%s=%s" % (c["name"], c.get("value")) for c in checks if not c.get("ok")]
            key = [c for c in checks if c["name"] in ("p90_render_ms", "first_frame_ms", "burst_end_ms")]
            numbers = " ".join("%s %s" % (c["name"].replace("_ms", ""), c.get("value")) for c in key)
            parts.append("%s %s%s%s" % (res.get("name"), "ok" if not failed else "NOT", (" " + ",".join(failed)) if failed else "", (" [" + numbers + "]") if numbers else ""))
        detail = "; ".join(parts)
    except (OSError, ValueError, AttributeError):
        detail = "no %s.json" % name
    if load:
        return "GATE frames-load: measured under synthetic background load, the unloaded thresholds say %s: %s (surfaces %s, %d cycles; %s)" % (verdict, detail, FRAMES_SURFACES, cycles, out_json)
    return "GATE frames%s: %s %s (surfaces %s, %d cycles; %s)" % (" at %d Hz" % refresh if refresh != 60 else "", verdict, detail, FRAMES_SURFACES, cycles, out_json)

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
    ap.add_argument("--only", default="")
    ap.add_argument("--x1-seconds", type=int, default=60)
    ap.add_argument("--enable-after", action="store_true")
    ap.add_argument("--frames-cycles", type=int, default=3, help="open/close cycles of the surface frame gate's soak (default 3)")
    ap.add_argument("--frames-load", action="store_true", help="also run the surface frame gate under synthetic background load (decode, Home Assistant websocket, BLE scan, a low-priority download): a report, not a rule")
    ap.add_argument("--kwait", action="store_true", help="trace the x1 run (capture.sh's kwait mode) and name the kernel waits behind each late frame (kwait.py)")
    ap.add_argument("--frames-callbacks", choices=["early", "landing"], default="early",
                    help="when the shell client's frame callbacks go out in the surface frame gate: a frame ahead of the flip inside a run of its frames (early, the default) or at the flip (landing)")
    ap.add_argument("--refresh", type=int, choices=[60, 120], default=60,
                    help="the refresh the surface frame gate runs at: 120 switches the compositor's refresh test mode on for it (gate.sh refresh 120) and off after")
    a = ap.parse_args()
    skip = set(a.skip.split(",")) - {""}
    skipped = set(skip)       # as asked, before --only leaves out what it did not name
    only = set(a.only.split(",")) - {""}
    if only:
        if only - set(CHECKS):
            ap.error("--only: unknown check(s) %s; the checks are %s" % (",".join(sorted(only - set(CHECKS))), ",".join(CHECKS)))
        skip |= set(CHECKS) - only
    tv.put(os.path.join(here, "tv-gate.sh"), "/tmp/lumatv/gate.sh")
    tv.put(os.path.join(here, "tv-lpc-probe.sh"), "/tmp/lumatv/lpc-probe.sh")
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
    # An x1 run longer than the rule's 60 s lengthens the first session by its extra minutes.
    run_frames = "frames" not in skip
    # The loaded run is opt-in: `--frames-load` adds it (whatever --only names, unless --skip names it), `--only frames-load` is it alone.
    run_load = "frames-load" in only or (a.frames_load and "frames-load" not in skipped)
    first_minutes = FIRST_SESSION_MINUTES + max(0, a.x1_seconds - 60 + 59) // 60
    # The frame gate's `enable --session` finds the first session live and does not re-arm its timer, so the first session's timer must cover the frame gate too
    # (the full gate of 2026-10-27 lost its session to the 12 minute timer before standby), the loaded run's as well.
    first_minutes += FRAMES_SESSION_MINUTES * (run_frames + run_load)
    def session(minutes, label=None):
        """LumaTV's compositor live for this boot only, ended by the set itself after `minutes`, then `live` says so (the
        label names the line when it is a second session of the run)."""
        print(pty("sh /tmp/lumatv/cutover.sh enable --session --max-minutes %d" % minutes, 300))
        step("live", "sh /tmp/lumatv/gate.sh live%s" % (" " + label if label else ""))
    if not wake():
        sys.exit("the set does not answer; nothing was changed")
    # The Home stub as the run finds it. The Home holds of the escape checks are also what LG's compositor's stub watches for: its own
    # hold-Home fallback gives LG's Home back (state lg, one fault), so the run puts the stub back at its end.
    stub_before = homestub_status()
    stub_was_on = homestub_is_on(stub_before)
    print("home stub before the run: %s" % stub_before)
    avr_mute(True)
    try:
        # Not live yet: the firmware check on the set's own files, and the supervisor's question about a held Home key.
        if "firmware" not in skip:
            step("firmware", "sh /tmp/lumatv/gate.sh firmware", 300)
        if "homeheld" not in skip:
            step("homeheld", "sh /tmp/lumatv/gate.sh homeheld", 120)
        session(first_minutes)
        if "volume" not in skip:
            line = volume_check()
            print(line)
            lines.append(line)
        if "x1" not in skip:
            env = ""
            if a.kwait:
                tv.tv("mkdir -p /tmp/perf")
                tv.put(os.path.join(PERF_DIR, "capture.sh"), "/tmp/perf/capture.sh")
                tv.put(os.path.join(PERF_DIR, "kwait.py"), "/tmp/perf/kwait.py")
                env = "LUMATV_GATE_KWAIT=1 "
            step("x1", "%ssh /tmp/lumatv/gate.sh x1 %d" % (env, a.x1_seconds), a.x1_seconds + 120)
        if "netflix" not in skip:
            avr_mute(True)   # the receiver may have been off (no mute to take) when the first one was sent
            step("netflix", "sh /tmp/lumatv/gate.sh netflix")
        if "hdmi" not in skip:
            step("hdmi", "sh /tmp/lumatv/gate.sh hdmi")
        if "keys" not in skip:
            step("keys", "sh /tmp/lumatv/gate.sh keys", 300)
            avr_mute(True)   # an app a key opened may have made a sound
        if "ime" not in skip:
            step("ime", "sh /tmp/lumatv/gate.sh ime", 180)
        if "shell" not in skip:
            step("shell", "sh /tmp/lumatv/gate.sh shell", 400)
            avr_mute(True)   # the volume key of the check moved the receiver's mute
        step("evidence", "sh /tmp/lumatv/gate.sh evidence", 120)
        if run_frames or run_load:
            # A session of its own: the surface gate waits for a 90 s old compositor and runs several minutes.
            session(FRAMES_SESSION_MINUTES * (run_frames + run_load), "frames")
            if a.frames_callbacks == "landing":
                print(pty("sh /tmp/lumatv/gate.sh callbacks landing", 60))
        if run_frames:
            if a.refresh != 60:
                # The switch, then idle Home at the new rate: the rate three ways, the compositor's measure, Mali and the temperature.
                step("refresh", "sh /tmp/lumatv/gate.sh refresh %d 10" % a.refresh, 120)
            print(pty("sh /tmp/lumatv/gate.sh wake reset", 60))
            lines.append(frames_check(a.frames_cycles, refresh=a.refresh))
            if a.refresh != 60:
                # Idle Home again after the surfaces (the thermal reading that matters), then the mode off: 60 is the gate's resting state.
                step("refresh", "sh /tmp/lumatv/gate.sh refresh reset 10", 120)
                step("refresh", "sh /tmp/lumatv/gate.sh refresh 60 5", 120)
            # What the loop's wake-ups cost over the gate, and what the compositor holds in memory after it (reports, not rules).
            step("wake", "sh /tmp/lumatv/gate.sh wake frames", 60)
            step("mem", "sh /tmp/lumatv/gate.sh mem", 60)
            # Where the interrupts are served and what wakes at Home just after the gate (docs/research/lpc-tv-result.md, items 5 and 7): reads of /proc, nothing changed.
            print(pty("sh /tmp/lumatv/lpc-probe.sh irq 10 home", 60))
            print(pty("sh /tmp/lumatv/lpc-probe.sh wakeups 10 home", 60))
            # How many of the shell's callbacks went out ahead of their flip during the gate: zero says the change was not in play.
            print(pty("sh /tmp/lumatv/gate.sh callbacks", 60))
        if run_load:
            print(pty("sh /tmp/lumatv/gate.sh wake reset", 60))
            lines.append(frames_check(a.frames_cycles, load=True))
            step("wake", "sh /tmp/lumatv/gate.sh wake loaded", 60)
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
            session(LATER_SESSION_MINUTES, "after-" + name)
        if "escape" not in skip:
            step("escape", "sh /tmp/lumatv/gate.sh escape", 300)
    finally:
        # Whatever happened above, LG's compositor is the live one afterwards with the flag removed (a failed
        # escape would otherwise leave LumaTV's running); --enable-after turns it back on below after a pass.
        try:
            print(pty("sh /tmp/lumatv/cutover.sh disable", 300))
        except Exception as e:
            print("disable failed: %s" % e)
        # The refresh test mode's divider back to LG's 2 whatever happened above (the compositor puts it back itself at a clean exit).
        if a.refresh != 60:
            try:
                print("divider after: " + pty("f=/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip; [ \"$(cat $f)\" = 2 ] || echo 2 > $f; cat $f", 20).strip())
            except Exception as e:
                print("divider restore failed: %s" % e)
        # The session's whole log (the escape and the fallback included) stays in the state directory until the next
        # start of LumaTV's compositor truncates it.
        tv.tv("cp /var/lib/lumatv/compositor/compositor.log /tmp/lumatv/out/gate/compositor-final.log; cp /var/lib/lumatv/compositor/sm.log /tmp/lumatv/out/gate/sm-final.log")
        avr_mute(False)
        if stub_was_on:
            try:
                print(pty("%s homestub on" % LUMATV_TOOL, 120))
                print("home stub after the run: %s" % homestub_status())
            except Exception as e:
                print("home stub on failed: %s" % e)
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
