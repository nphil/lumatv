#!/usr/bin/env python3
"""Round 8's TV slot in one run (about 20 minutes, exclusive use of the set, from the repository root). NOT RUN YET: written offline while
the set was another worker's slot; tests/test_round8.py runs it against a simulated set, which says the script agrees with itself and with
the notes it was written from, and nothing about the TV.

    python3 docs/research/round8/tv_round8.py --plan             # the steps; touches nothing (the default)
    python3 docs/research/round8/tv_round8.py --go               # the slot, with Main's go
    python3 docs/research/round8/tv_round8.py --go --skip x4,logo --approve-test-plugin --out /tmp/r8

Four steps, each skippable (`--skip plugin,x4,maliit,logo`), each writing evidence to docs/research/round8/<date>-<time>/<step>/:

 plugin  What plugins draw (plugins-slice-result.md, panels-slice-result.md): the At-home lane's plugin cards, the quick panel's plugin
         tiles, the ambient settings page and the notification drawer with a plugin note. A test plugin (`plugin/plugin.py`, manifest
         `plugin/plugin.json`) puts a card, a tile and a note through the helper's plugin API; Home is asked for each surface through
         the development door `/tmp/<shortId>-layer` (LumaHome.qml polls it every 2 s: `quick`, `drawer`, `settings`), the D-pad goes
         to the surface that is open, and every surface gets a screenshot, the helper's topic that feeds it and the compositor's log
         for the window (docs/research/perf/gate.py's failure patterns). The test plugin is a change to what the helper has switched
         on, which is the owner's act (Principle 0): without `--approve-test-plugin` (Nitin's yes) the step draws only what the helper's
         own plugins give (the ambient tile, the weather/news/clock cards) and the empty lane and drawer.
 x4      What the architecture study left open (shell-architecture.md "X4"), captured with arch/x4/x4-monitor.sh and x4-analyse.py: the
         input picker open and closed, PlxNative (`com.beb.plxnative`) started and played, standby and wake. No PIN prompt, no Netflix.
 maliit  The on-screen keyboard's QML (lg-restyles-result.md "Needs the TV" 5), copied read-only into the evidence, or what is there if
         the keys are compiled into the plugin.
 logo    The power-off logo's own record (notify-slice-result.md "Set steps" 1): a power-off and a wake, then `logo-events` and the
         `[Logo]` lines. No reboot. Uses the x4 step's standby cycle when that ran.

The set is left as found. Before anything: it answers and is Active, nothing else holds it (no capture, probe, card app or gate
running, no other slot's lock), Home is in front. The receiver is muted through Home Assistant (and read back, from Home Assistant and
from the helper) before anything plays; if it cannot be shown muted nothing plays. At the end, in this order: playback closed, Home
back in front and seen there, the receiver put back as it was, the plugin switched off again, the set in Active Standby (`--no-standby`
leaves it on). Picture Mode is read before and after, never set. `lumatv logo on` is the one switch the logo step may turn (and turns
off again) when the logo was off; `--keep-logo-off` refuses that. Nothing is installed on the set; its files are under /tmp/round8
(and the plugin's entry in `<data>/plugins/`, put back byte for byte).

Every line of summary.txt is PASS, FAIL, INFO or SKIP; a FAIL names what the notes expect there. Exit status 1 when anything failed or
a PROBLEM (something that could not be put back) was printed last.
"""
import argparse
import gzip
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
for _p in (os.path.join(ROOT, "docs", "research", "perf"), os.path.join(ROOT, "docs", "research", "arch"), os.path.join(ROOT, "compositor", "tools")):
    sys.path.insert(0, _p)
import gate  # noqa: E402  the frame gate's log patterns: one list of what a surface may not say

STEPS = ("plugin", "x4", "maliit", "logo")
APP = "org.nphil.lumatv"
APPS = "/media/developer/apps/usr/palm/applications"
TOOL = "%s/%s/tools/lumatv" % (APPS, APP)
HOME_APP = "com.webos.app.home"
PLEX = "com.beb.plxnative"
PICKER_APP = "com.webos.app.quickinputpicker"
SAM = "luna://com.webos.applicationManager"
SM = "luna://com.webos.surfacemanager"
POWER = "luna://com.webos.service.tvpower/power"
HELPER = "http://127.0.0.1:18731"
R = "/tmp/round8"
KB = R + "/kb.py"
LOG = "/var/log/messages"
LOGO_EVENTS_NAME = "logo-events"
RECEIVER = "media_player.living_room_avr"
HA = "http://192.168.1.146"

PLUGIN_DIR = os.path.join(HERE, "plugin")
X4_DIR = os.path.join(ROOT, "docs", "research", "arch", "x4")

# The development door (LumaHome.qml): a layer named in /tmp/<shortId>-layer is shown within 2 s, and the file is polled only while Home
# is the surface. 3 s covers the poll and the surface's own animation.
DOOR_WAIT = 3.2
SETTLE = 1.2

# The order of `Permission` in LumaShow's plugin-api (permission.rs): the consent document lists a manifest's permissions in it, not in the
# author's order. Only the words a test plugin may use are here; another word is refused rather than put in a guessed place.
PERMISSION_ORDER = ("settings", "timeline", "cards", "tiles", "notify", "background", "process.spawn", "ui.view")


def load_brand():
    with open(os.path.join(ROOT, "brand.json")) as f:
        return json.load(f)


BRAND = load_brand()
DOOR = "/tmp/%s-layer" % BRAND["shortId"]
DATA = BRAND["dataDir"]


# ---- the plan (pure; tests/test_round8.py reads it) --------------------------------------------------------------------------

def plan(skip=(), approve_plugin=False, standby=True, keep_logo_off=False):
    """The slot as ordered (step, why, always): `always` marks what runs even after a failure."""
    steps = [("preflight", "read-only apart from waking the set: it answers, is Active and Home is in front; no capture, probe, gate, card "
                           "app or other slot's lock; Picture Mode, the helper and the receiver are recorded; kb.py and the X4 monitor "
                           "script go to /tmp/round8", False),
             ("mute", "the receiver is muted through Home Assistant and read back (Home Assistant and the helper); nothing plays unless "
                      "that is shown", False)]
    if "plugin" not in skip:
        steps.append(("plugin", "Home's lane, the quick panel's tiles, Settings > Ambient light and the notification drawer, each by the "
                                "development door, with a screenshot, the helper's topic and a log check" +
                                ("; a test plugin puts a card, a tile and a note through the plugin API" if approve_plugin
                                 else "; without --approve-test-plugin only the helper's own plugins draw"), False))
    if "maliit" not in skip:
        steps.append(("maliit", "the on-screen keyboard's QML tree copied read-only, or what is there if the keys are compiled in", False))
    if "x4" not in skip:
        steps.append(("x4", "ls-monitor around the input picker (open, close), PlxNative (launch, play, close, Home) and standby and "
                            "wake; each census written beside the capture", False))
    if "logo" not in skip:
        steps.append(("logo", "the power-off logo's record in logo-events and the log" +
                              ("" if keep_logo_off else " (the logo switch is turned on for it if it was off, and back)"), False))
    steps += [("close", "playback closed, Home in front and seen, the monitor and test plugin stopped, the roster put back", True),
              ("unmute", "the receiver is put back as it was, after Home is back", True),
              ("summary", "summary.txt and the Picture Mode after", True)]
    if standby:
        steps.append(("standby", "the set is left in Active Standby", True))
    return steps


# ---- pure readers --------------------------------------------------------------------------------------------------------------

def consent_document(manifest):
    """LumaShow's `Manifest::consent_document` for the manifest a test plugin may have: the id, levels, program, class, leases,
    permissions in the library's order and limits, as compact JSON in that order. tests/test_round8.py pins the hash of the shipped
    manifest to the one the plugin-api crate gives."""
    perms = manifest.get("permissions", [])
    for p in perms:
        if p not in PERMISSION_ORDER:
            raise ValueError("permission %r is not one this script knows the place of" % p)
    if manifest.get("requires"):
        raise ValueError("a manifest with leases is not one this script describes")
    doc = {"consent": 1, "id": manifest["id"], "api": {"min": manifest["api"]["min"], "target": manifest["api"]["target"]},
           "exe": manifest["run"]["exe"], "args": manifest["run"].get("args", []), "class": manifest["class"], "requires": [],
           "permissions": sorted(set(perms), key=PERMISSION_ORDER.index), "limits": manifest.get("limits", {})}
    return json.dumps(doc, separators=(",", ":"), ensure_ascii=False)


def approval(manifest):
    return hashlib.sha256(consent_document(manifest).encode()).hexdigest()


def merged_roster(existing_text, entry, first_uid=20000, span=64):
    """enabled.json with `entry` (id, approve) added under a free user id: (text, uid). The other entries are kept as they are."""
    doc = json.loads(existing_text) if existing_text.strip() else {"plugins": []}
    plugins = doc.get("plugins")
    if not isinstance(plugins, list):
        raise ValueError("enabled.json has no plugins list")
    if any(p.get("id") == entry["id"] for p in plugins):
        raise ValueError("the roster lists %s already" % entry["id"])
    taken = {p.get("uid") for p in plugins}
    uid = next((u for u in range(first_uid, first_uid + span) if u not in taken), None)
    if uid is None:
        raise ValueError("no free plugin user id")
    doc["plugins"] = plugins + [{"id": entry["id"], "uid": uid, "approve": entry["approve"]}]
    return json.dumps(doc, separators=(",", ":")), uid


def json_of(text):
    """The first JSON object in a command's output (luna-send prints it pretty, and a PTY or a [stderr] note may surround it)."""
    try:
        return json.loads(text)
    except ValueError:
        pass
    start, end = text.find("{"), text.rfind("}")
    if 0 <= start < end:
        try:
            return json.loads(text[start:end + 1])
        except ValueError:
            pass
    return {}


def power_word(text):
    m = re.search(r'"state"\s*:\s*"([^"]*)"', text)
    return m.group(1).lower() if m else text.strip().lower()[:40]


def parse_logo_events(text):
    """`up=<s> wall=<ISO> <step>` lines of logo-events: [{up (float or None), wall, step}]."""
    out = []
    for line in text.splitlines():
        m = re.match(r"up=(\S+) wall=(\S+) (.*)$", line.strip())
        if m:
            try:
                up = float(m.group(1))
            except ValueError:
                up = None
            out.append({"up": up, "wall": m.group(2), "step": m.group(3)})
    return out


def new_lines(before, after):
    """The lines `after` has that `before` did not (the file keeps its last 60 lines, so a prefix compare would lose them)."""
    seen = {}
    for l in before.splitlines():
        seen[l] = seen.get(l, 0) + 1
    out = []
    for l in after.splitlines():
        if seen.get(l, 0) > 0:
            seen[l] -= 1
        else:
            out.append(l)
    return out


def power_off_record(events):
    """What the power-off window wrote: (launch, answered, gap in seconds or None, lines kept)."""
    launch = next((e for e in events if e["step"].startswith("power-off logo") and "launch" in e["step"]), None)
    answered = next((e for e in events if "power-off answered tvpowerd" in e["step"]), None)
    gap = None
    if launch and answered and launch["up"] is not None and answered["up"] is not None:
        gap = round(answered["up"] - launch["up"], 2)
    return launch, answered, gap


def load_messages(text):
    """ls-monitor -j lines as dicts (the lines that are not JSON objects are the monitor's own noise)."""
    out = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                out.append(json.loads(line))
            except ValueError:
                pass
    return out


def power_phases(msgs):
    """The order of tvpower's `sub processing` words, as pushed to the compositor's subscription, without repeats (the monitor shows each
    message twice, once per side)."""
    seen = []
    for m in msgs:
        p = m.get("payload")
        if isinstance(p, dict) and m.get("sender") == "com.webos.service.tvpower" and "sub processing" in p:
            word = p["sub processing"]
            if not seen or seen[-1] != word:
                seen.append(word)
    return seen


def plex_facts(msgs):
    """What PlxNative's playback did on the bus: its calls by method, the video placements it made, the calls the compositor made, and
    whether the foreground video window ever named it."""
    calls, placements, window = {}, 0, False
    compositor = {}
    for m in msgs:
        if m.get("transport") not in (None, "TX") or m.get("type") != "call":
            continue
        sender, method = str(m.get("sender", "")), m.get("method", "")
        if "plxnative" in sender:
            calls["%s/%s" % (m.get("destination", "?"), method)] = calls.get("%s/%s" % (m.get("destination", "?"), method), 0) + 1
            if method in ("setDisplayWindow", "connect", "setCropRegion") or "display" in str(m.get("methodCategory", "")):
                placements += 1
        if sender.startswith("com.webos.surfacemanager"):
            key = "%s/%s" % (m.get("destination", "?"), method)
            compositor[key] = compositor.get(key, 0) + 1
    for m in msgs:
        p = m.get("payload")
        if isinstance(p, dict) and m.get("method") == "getForegroundVideoWindow" and PLEX in json.dumps(p):
            window = True
            break
    return {"calls": calls, "placements": placements, "window": window, "compositor_calls": compositor}


def png_change(path_a, path_b, threshold=24):
    """The share of pixels that differ by more than `threshold` in some channel, or None when the images cannot be read."""
    try:
        from PIL import Image, ImageChops
        a, b = Image.open(path_a).convert("RGB"), Image.open(path_b).convert("RGB")
        if a.size != b.size:
            return 1.0
        diff = ImageChops.difference(a, b).convert("L").point(lambda v: 255 if v > threshold else 0)
        return sum(1 for v in diff.getdata() if v) / float(a.size[0] * a.size[1])
    except Exception:
        return None


def png_flat(path):
    """True for a picture with one colour all over (a black or blank capture), None when it cannot be read."""
    try:
        from PIL import Image, ImageStat
        return max(ImageStat.Stat(Image.open(path).convert("L")).stddev) < 2.0
    except Exception:
        return None


# ---- the receiver --------------------------------------------------------------------------------------------------------------

class HomeAssistant:
    """The receiver's entity through Home Assistant's REST API, with the homelab's own token (tv_gate.ha_token)."""

    def __init__(self, gate_module):
        self.gate = gate_module

    def _call(self, path, body=None):
        import urllib.request
        req = urllib.request.Request(HA + path, data=json.dumps(body).encode() if body is not None else None,
                                     headers={"Authorization": "Bearer " + self.gate.ha_token(), "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read() or b"null")

    def state(self):
        st = self._call("/api/states/" + RECEIVER)
        return {"state": st.get("state"), "muted": st.get("attributes", {}).get("is_volume_muted"),
                "volume": st.get("attributes", {}).get("volume_level")}

    def mute(self, on):
        self._call("/api/services/media_player/volume_mute", {"entity_id": RECEIVER, "is_volume_muted": bool(on)})


# ---- one run -------------------------------------------------------------------------------------------------------------------

class Run:
    """One run: the verdict lines, the evidence directory and the few things asked of the set. `tv` is docs/research/arch/tv.py (or a
    stand-in with tv(cmd, timeout), put and get), `gate` compositor/tools/tv_gate.py (wake, power_state), `ha` a HomeAssistant."""

    def __init__(self, tv, gate_module, ha, out, a=None):
        self.tv, self.gate, self.ha = tv, gate_module, ha
        self.out = out
        self.a = a
        self.lines = []
        self.failed = 0
        self.problems = []
        self.muted_before = None      # None: the receiver was not touched
        self.muted_by_us = False
        self.locked = False
        self.woke = False
        self.monitor_on = False
        self.plugin_state = None      # what the test plugin changed, to put back
        self.logo_was_on = None
        self.cycles = []              # the standby cycles of this run, for the logo step
        self.logo_ready = False       # the logo's record will be written by a power-off now
        self.door_used = False
        os.makedirs(out, exist_ok=True)

    # ---- verdicts
    def say(self, kind, name, detail=""):
        line = "%s %s: %s" % (kind, name, detail)
        print(line, flush=True)
        self.lines.append(line)
        with open(os.path.join(self.out, "summary.txt"), "a") as f:
            f.write(line + "\n")

    def info(self, name, detail=""):
        self.say("INFO", name, detail)

    def skip(self, name, why):
        self.say("SKIP", name, why)

    def check(self, name, ok, detail=""):
        if not ok:
            self.failed += 1
        self.say("PASS" if ok else "FAIL", name, detail)
        return ok

    def problem(self, text):
        self.problems.append(text)
        print("PROBLEM:", text, file=sys.stderr, flush=True)

    def dir(self, *parts):
        path = os.path.join(self.out, *parts)
        os.makedirs(path, exist_ok=True)
        return path

    def save(self, rel, text):
        path = os.path.join(self.out, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)
        return path

    # ---- the set
    def sh(self, cmd, timeout=60):
        return self.tv.tv(cmd, timeout=timeout)

    def luna(self, uri, payload="{}"):
        return json_of(self.sh("luna-send -n 1 -f %s '%s' 2>&1" % (uri, payload)))

    def foreground(self):
        d = self.luna(SAM + "/getForegroundAppInfo", '{"extraInfo":true}')
        return [x.get("appId", "") for x in d.get("foregroundAppInfo", [])] or ([d["appId"]] if d.get("appId") else [])

    def power(self):
        return power_word(self.sh("luna-send -n 1 -f %s/getPowerState '{}' 2>&1" % POWER))

    def wait(self, cond, seconds, every=1.0):
        end = time.time() + seconds
        while True:
            value = cond()
            if value:
                return value
            if time.time() >= end:
                return value
            time.sleep(every)

    def key(self, *tokens):
        return self.sh("python3 %s %s" % (KB, " ".join(tokens)), timeout=90)

    def picture_mode(self):
        out = self.sh("luna-send -n 1 -f luna://com.webos.settingsservice/getSystemSettings "
                      "'{\"category\":\"picture\",\"keys\":[\"pictureMode\"]}' 2>&1")
        m = re.search(r'"pictureMode"\s*:\s*"([^"]*)"', out)
        return m.group(1) if m else out.strip()[:60]

    def helper(self, path, body=None):
        """(http status, parsed JSON) of the helper's loopback API, read through the set's curl."""
        if body is None:
            cmd = "curl -s -m 8 -w '\\n%%{http_code}' %s%s" % (HELPER, path)
        else:
            cmd = ("curl -s -m 8 -w '\\n%%{http_code}' -X POST -H 'X-Luma-Client: round8' -H 'Content-Type: application/json' -d '%s' %s%s"
                   % (json.dumps(body), HELPER, path))
        text = self.sh(cmd)
        head, _, code = text.rstrip("\n").rpartition("\n")
        return (int(code) if code.strip().isdigit() else 0), json_of(head)

    def topic(self, name):
        status, doc = self.helper("/v1/state/" + name)
        return doc.get("value") if status == 200 else None

    def log_offset(self):
        out = self.sh("wc -c < %s 2>/dev/null" % LOG).strip()
        return int(out) if out.isdigit() else 0

    def log_since(self, offset):
        return self.sh("tail -c +%d %s 2>/dev/null" % (offset + 1, LOG))

    def log_check(self, step, name, offset):
        """The compositor's log since `offset`, kept as evidence; a line that matches the gate's failure patterns fails the step."""
        text = self.log_since(offset)
        self.save("%s/%s.log" % (step, name), text)
        bad, ignored = gate.log_failures(text.splitlines())
        self.check("%s: no failure line in the log" % name, not bad,
                   "%d line(s) read, %d known noise%s" % (len(text.splitlines()), ignored, "; first: " + bad[0][:160] if bad else ""))
        return bad

    def shot(self, step, name):
        """A screenshot of the panel (960x540) into <step>/<name>.png; the path, or None when it was not fetched."""
        time.sleep(1.0)
        remote = "%s/%s-%s.png" % (R, step, name)
        self.sh("mkdir -p %s && rm -f %s && luna-send -n 1 -f luna://com.webos.service.capture/executeOneShot "
                "'{\"path\":\"%s\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}' >/dev/null 2>&1" % (R, remote, remote))

        def size():
            return self.sh("wc -c < %s 2>/dev/null" % remote).strip()

        def written():     # the file lands after executeOneShot has answered (cardapp 17:41): wait for its size to hold
            before = size()
            time.sleep(0.5)
            return before.isdigit() and before != "0" and before == size()

        if self.wait(lambda: size().isdigit() and size() != "0", 10, 0.5):
            self.wait(written, 5, 0.5)
        local = os.path.join(self.dir(step), name + ".png")
        try:
            self.tv.get(remote, local)
            return local
        except Exception as e:
            self.info("screenshot %s/%s" % (step, name), "not fetched: %s" % e)
            return None

    def door(self, name):
        """Asks Home for a layer by the development door and waits for it to draw."""
        self.door_used = True
        self.sh("printf '%%s' %s > %s" % (name, DOOR))
        time.sleep(DOOR_WAIT)

    def door_close(self):
        self.sh(": > %s" % DOOR)
        time.sleep(DOOR_WAIT)

    def ensure_home(self, seconds=20):
        """Home in front: launched if something else is, and seen there."""
        if HOME_APP in self.foreground()[:1]:
            return True
        self.luna(SAM + "/launch", json.dumps({"id": HOME_APP}))
        return bool(self.wait(lambda: self.foreground()[:1] == [HOME_APP], seconds, 1.0))

    # ---- the receiver
    def receiver_reading(self):
        """(Home Assistant's mute, the helper's mute): None where a source has none. Home Assistant's entity stays stale after an unmute
        (memory-result.md), so the helper's direct reading is the second opinion."""
        try:
            ha = self.ha.state()
        except Exception as e:
            self.info("receiver (Home Assistant)", "not read: %s" % e)
            ha = {}
        helper = self.topic("receiver")
        helper_mute = helper.get("mute") if isinstance(helper, dict) and helper.get("fresh") else None
        return ha.get("muted"), helper_mute, ha

    def set_mute(self, on):
        """True when the receiver is shown muted (on) or shown not muted (off). Muting needs a reading that says so, from Home Assistant or
        from the helper. Unmuting needs the call to have gone through and no source still saying muted: Home Assistant's entity stays
        stale at "muted" after an unmute (memory-result.md), so its word alone is no evidence against."""
        called = True
        try:
            self.ha.mute(on)
        except Exception as e:
            called = False
            self.info("receiver mute %s" % on, "the call failed: %s" % e)
        time.sleep(2.0)
        ha_mute, helper_mute, _ = self.receiver_reading()
        if on:
            return called and shown_muted(ha_mute, helper_mute)
        return called and helper_mute is not True


# ---- preflight ------------------------------------------------------------------------------------------------------------------

# The bracket in each name keeps the pattern from matching the shell that runs this very command line. LumaTV's own colour reader and
# helper are the package's and run all the time: they are not in the list.
BUSY = ("pgrep -fl 'ls-[m]onitor|x1[c]lient|x2[s]rv|lumatv-[c]ompositor|lumatv-[s]m |[s]lot\\.sh|[k]b\\.py|lumatv-card-[a]pp|"
        "[p]lug[a-z]*\\.py' || echo none; "
        "ls -d %s.lock /tmp/ambient-slot-*/run.lock /var/luna/preferences/vt_devmode 2>/dev/null || echo nothing" % R)


def free_problems(busy_text):
    """What says the set is not free: another slot's processes, locks, or the probe's developer flag."""
    lines = [l.strip() for l in busy_text.splitlines() if l.strip()]
    found = [l for l in lines if l not in ("none", "nothing")]
    return found


def preflight(run, a):
    try:
        was = power_word(run.tv.tv("luna-send -n 1 -f %s/getPowerState '{}' 2>&1" % POWER, timeout=15))
    except Exception:
        was = "unreachable"
    if not run.gate.wake():
        sys.exit("the set does not answer; nothing was changed")
    run.info("power", "%s before, %s now" % (was, run.power()))
    if was != "active":
        run.woke = True
    if not run.wait(lambda: run.power() == "active", 30, 2.0):
        sys.exit("the set is %r, not Active; nothing was changed" % run.power())
    busy = free_problems(run.sh(BUSY))
    if busy:
        run.problem("the set is not free (another slot's work is on it): %s" % " | ".join(busy)[:300])
        sys.exit("the set is not free: %s" % " | ".join(busy)[:300])
    fg = run.foreground()
    if fg[:1] != [HOME_APP]:
        sys.exit("Home is not in front (%s); not starting over something else" % (fg[:1] or "nothing"))
    out = run.sh("mkdir %s.lock 2>&1 && echo locked" % R)
    if "locked" not in out:
        sys.exit("%s.lock exists: another run of this script, or a dead one (rm -r it)" % R)
    run.locked = True
    run.sh("mkdir -p %s/x4 && chmod 755 %s" % (R, R))
    run.tv.put(os.path.join(ROOT, "docs", "research", "perf", "kb.py"), KB)
    with open(os.path.join(X4_DIR, "x4-monitor.sh")) as f:
        monitor = f.read().replace("D=/tmp/x4", "D=%s/x4" % R)
    local = os.path.join(run.dir("x4"), "x4-monitor.sh")
    with open(local, "w") as f:
        f.write(monitor)
    run.tv.put(local, R + "/x4-monitor.sh")
    run.info("picture mode", run.picture_mode())
    run.info("foreground", ",".join(fg))
    status = json_of(run.sh("%s status 2>&1" % TOOL))
    run.info("lumatv", "live %s, applied %s, helper %s, compositor %s" % (
        status.get("live"), status.get("applied"), (status.get("helper") or {}).get("running"), json.dumps(status.get("compositor"))[:80]))
    run.save("preflight-lumatv-status.json", json.dumps(status, indent=1))
    return status


def shown_muted(ha_mute, helper_mute):
    """Whether the receiver is muted, by the helper's direct reading when it has a fresh one: Home Assistant's entity stays stale at
    "muted" after an unmute until the receiver goes off (memory-result.md), so it is only the fallback."""
    return bool(helper_mute) if helper_mute is not None else bool(ha_mute)


def mute_receiver(run):
    ha_mute, helper_mute, ha = run.receiver_reading()
    run.save("receiver-before.json", json.dumps({"homeAssistant": ha, "helperMute": helper_mute}, indent=1))
    run.info("receiver before", "Home Assistant %s, helper mute %s" % (ha, helper_mute))
    run.muted_before = shown_muted(ha_mute, helper_mute)
    if run.muted_before:
        run.check("receiver muted", True, "it was muted already; left as it is")
        return True
    ok = run.set_mute(True)
    run.muted_by_us = True
    run.check("receiver muted", ok, "muted through Home Assistant and read back" if ok else "NOT shown muted by Home Assistant or the helper: nothing will play")
    return ok


def remute(run):
    """After a wake the receiver may have followed the set (CEC): mute again and read back before anything plays."""
    if run.muted_before:
        return True
    ok = run.set_mute(True)
    run.muted_by_us = True
    return ok


# ---- step: plugin ---------------------------------------------------------------------------------------------------------------

def helper_config(run):
    text = run.sh("cat %s/helper.json 2>/dev/null" % DATA)
    cfg = json_of(text).get("plugins", {}) if text.strip() else {}
    return {"firstUid": int(cfg.get("firstUid", 20000)), "socket": cfg.get("socket", "/run/lumashow-plugins/broker.sock")}


def install_test_plugin(run, d):
    """Puts the test plugin on in the helper's roster, starts it as its user and waits until its card is a topic. Returns True when it
    is up. Records what it changed in run.plugin_state so close_plugin can put it back whatever happens next."""
    with open(os.path.join(PLUGIN_DIR, "plugin.json")) as f:
        manifest_text = f.read()
    manifest = json.loads(manifest_text)
    cfg = helper_config(run)
    roster_dir = DATA + "/plugins"
    original = run.sh("cat %s/enabled.json 2>/dev/null" % roster_dir)
    existed = run.sh("test -e %s/enabled.json && echo yes || echo no" % roster_dir).split()[0] == "yes"
    run.save("%s/enabled.json.before" % d, original if existed else "")
    try:
        text, uid = merged_roster(original if existed else "", {"id": manifest["id"], "approve": approval(manifest)}, cfg["firstUid"])
    except ValueError as e:
        run.check("test plugin: roster", False, str(e))
        return False
    run.plugin_state = {"roster_dir": roster_dir, "existed": existed, "dir_existed": run.sh("test -d %s && echo yes || echo no" % roster_dir).split()[0] == "yes",
                        "uid": uid, "original": original, "socket": cfg["socket"], "started": False}
    run.sh("mkdir -p %s/%s %s && rm -f %s/plugin.log && : > %s/plugin.cmd && : > %s/plugin.log && chmod 666 %s/plugin.cmd %s/plugin.log"
           % (roster_dir, manifest["id"], R, R, R, R, R, R))
    local_text = os.path.join(run.dir(d), "enabled.json.test")
    with open(local_text, "w") as f:
        f.write(text)
    run.tv.put(os.path.join(PLUGIN_DIR, "plugin.json"), "%s/%s/plugin.json" % (roster_dir, manifest["id"]))
    run.tv.put(os.path.join(PLUGIN_DIR, "plugin.py"), R + "/plugin.py")
    run.tv.put(local_text, roster_dir + "/enabled.json")
    run.sh("chmod 644 %s/plugin.py" % R)
    status, _ = run.helper("/v1/plugins/rescan", {})
    run.info("test plugin: rescan", "HTTP %s, user id %d, approval %s" % (status, uid, approval(manifest)[:16]))

    def listed():
        s, doc = run.helper("/v1/plugins")
        for p in (doc.get("plugins") or []) if s == 200 else []:
            if p.get("id") == manifest["id"]:
                return p
        return None

    entry = run.wait(listed, 45, 3.0)
    run.save("%s/plugins-listed.json" % d, json.dumps(entry, indent=1))
    if not entry or entry.get("state") == "inert":
        run.check("test plugin: the helper switched it on", False, "listed as %s" % json.dumps(entry)[:200])
        return False
    run.check("test plugin: the helper switched it on", True, "state %s" % entry.get("state"))
    run.sh("cd %s && nohup python3 plugin.py --user %d --group 2000 --socket %s --ctl %s/plugin.cmd --log %s/plugin.log > %s/plugin.out 2>&1 < /dev/null & echo $! > %s/plugin.pid"
           % (R, uid, cfg["socket"], R, R, R, R))
    run.plugin_state["started"] = True
    cards = run.wait(lambda: (run.topic("plugin.%s.cards" % manifest["id"]) or {}).get("items"), 45, 2.0)
    run.save("%s/plugin-cards.json" % d, json.dumps(run.topic("plugin.%s.cards" % manifest["id"]), indent=1))
    run.save("%s/plugin.log" % d, run.sh("cat %s/plugin.log 2>/dev/null; cat %s/plugin.out 2>/dev/null" % (R, R)))
    run.check("test plugin: its card is a topic", bool(cards), "plugin.%s.cards: %s" % (manifest["id"], json.dumps(cards)[:160] if cards else "empty after 45 s (plugin.log/plugin.out say why)"))
    return bool(cards)


def close_plugin(run, d="plugin"):
    """The test plugin stopped and the roster as it was. Safe to call twice."""
    st, run.plugin_state = run.plugin_state, None
    if not st:
        return
    run.sh("echo quit >> %s/plugin.cmd" % R)
    time.sleep(3.0)
    run.sh("kill $(cat %s/plugin.pid 2>/dev/null) 2>/dev/null; true" % R)
    if st["existed"]:
        local = os.path.join(run.dir(d), "enabled.json.restore")
        with open(local, "w") as f:
            f.write(st["original"])
        run.tv.put(local, st["roster_dir"] + "/enabled.json")
    else:
        run.sh("rm -f %s/enabled.json" % st["roster_dir"])
    run.sh("rm -rf %s/round8" % st["roster_dir"])
    if not st["dir_existed"]:
        run.sh("rmdir %s 2>/dev/null; true" % st["roster_dir"])
    run.helper("/v1/plugins/rescan", {})
    time.sleep(4.0)
    status, doc = run.helper("/v1/plugins")
    gone = status == 200 and all(p.get("id") != "round8" for p in doc.get("plugins") or [])
    back = run.sh("cat %s/enabled.json 2>/dev/null" % st["roster_dir"]) == (st["original"] if st["existed"] else "")
    if not (gone and back):
        run.problem("THE TEST PLUGIN IS NOT FULLY REMOVED (listed gone %s, roster as before %s): remove %s/round8, restore %s/enabled.json, POST /v1/plugins/rescan"
                    % (gone, back, st["roster_dir"], st["roster_dir"]))
    run.check("test plugin: switched off, roster as it was", gone and back, "the helper no longer lists it; enabled.json %s" % ("as before" if back else "DIFFERS"))


def changed_vs(run, base, path, step, name, floor):
    """A surface's picture must differ from Home at rest by more than `floor` (a share of the pixels): a surface that is open changes
    most of its own area; the clock's tick changes far less."""
    if not path or not base:
        return run.check("%s: the screen shows it" % name, False, "no screenshot")
    share = png_change(base, path)
    flat = png_flat(path)
    if share is None:
        return run.info("%s: the screen shows it" % name, "the images could not be compared (Pillow?); look at %s/%s.png" % (step, name))
    return run.check("%s: the screen shows it" % name, share > floor and flat is not True,
                     "%.1f %% of the pixels differ from Home at rest (limit %.1f %%)%s" % (share * 100, floor * 100, ", capture is one flat colour" if flat else ""))


def pings(run):
    """How many `action ping` lines the test plugin has logged: each action the helper handed it."""
    out = run.sh("grep -c ' action ping ' %s/plugin.log 2>/dev/null" % R).strip()
    return int(out) if out.isdigit() else 0


def step_plugin(run, a):
    d = "plugin"
    run.dir(d)
    status = json_of(run.sh("%s status 2>&1" % TOOL))
    if not run.check("plugin: LumaTV's Home is the live one", status.get("live") is True and (status.get("helper") or {}).get("running") is True,
                     "live %s, helper running %s (the surfaces below are LumaHome's: `lumatv apply` first)" % (status.get("live"), (status.get("helper") or {}).get("running"))):
        return
    if not run.ensure_home():
        run.check("plugin: Home in front", False, "foreground %s" % run.foreground())
        return
    offset = run.log_offset()
    base = run.shot(d, "00-home")
    run.info("device", json.dumps(run.helper("/v1/device")[1])[:200])
    for name in ("plugins", "plugin.ambient.tiles", "plugin.weather.cards", "plugin.news.cards", "plugin.clock.cards"):
        run.save("%s/topic-%s.json" % (d, name), json.dumps(run.topic(name), indent=1))
    own = run.topic("plugin.ambient.tiles") or {}
    run.check("plugin: the helper's own ambient tile is a topic", bool(own.get("items")), json.dumps(own)[:160])

    test_up = False
    try:
        if a.approve_test_plugin:
            test_up = install_test_plugin(run, d)
        else:
            run.skip("plugin: the test plugin", "not switched on without --approve-test-plugin (Nitin's yes): the lane, the tile and the drawer show the helper's own plugins only")
        time.sleep(2.0)

        # Home's lane (the At-home lane draws PluginHub.homeCards: external plugins' cards; the weather, news and clock cards are Home's own widgets)
        lane = run.shot(d, "01-lane")
        cards = (run.topic("plugin.round8.cards") or {}).get("items") if test_up else None
        if test_up:
            run.check("lane: the plugin's card is in the helper's topic for Home to draw", bool(cards), json.dumps(cards)[:160])
        changed_vs_base = png_change(base, lane) if base and lane else None
        run.info("lane", "%s of the pixels differ from Home before the plugin (the clock's tick alone is below 0.1 %%); look at %s/01-lane.png: a chip with the card's title and its first fact"
                 % ("%.2f %%" % (changed_vs_base * 100) if changed_vs_base is not None else "unknown", d))
        run.log_check(d, "lane", offset)
        if test_up:
            # Home's own focus path: Up from the dock's first tile reaches the At-home widget, OK enters its lane, OK on a chip runs the
            # card's first action (HomeAssistantWidget.laneKey -> PluginHub.act -> POST /v1/plugins/<id>/actions/<action>).
            before = pings(run)
            run.key("UP", "s1", "OK", "s1")
            run.shot(d, "01b-lane-focus")
            run.key("OK", "s1.5")
            heard = run.wait(lambda: pings(run) == before + 1, 10, 1.0)
            run.check("lane: OK on the plugin's chip ran the card's action", bool(heard), "plugin.log pings %d before, %d after (keys UP, OK, OK from the dock's first tile; 01b-lane-focus.png shows where the focus was)" % (before, pings(run)))
            run.key("BACK", "s0.5", "DOWN", "s1")

        # A note posted now: the toast (INFO) and then the drawer
        if test_up:
            offset = run.log_offset()
            run.sh("echo 'note 2' >> %s/plugin.cmd" % R)
            time.sleep(2.0)
            toast = run.shot(d, "02-toast")
            share = png_change(lane, toast) if lane and toast else None
            run.info("toast", "%s of the pixels changed within about 3 s of the note (a plugin note may toast; look at %s/02-toast.png)" %
                     ("%.2f %%" % (share * 100) if share is not None else "unknown", d))
            notes = (run.topic("plugin.round8.notifications") or {}).get("items") or []
            run.save("%s/topic-plugin.round8.notifications.json" % d, json.dumps(notes, indent=1))
            run.check("drawer: the helper holds both notes", len(notes) >= 2, "ids %s" % [n.get("id") for n in notes])
            time.sleep(4.0)

        # The quick panel
        run.ensure_home()
        offset = run.log_offset()
        run.door("quick")
        quick = run.shot(d, "03-quick")
        changed_vs(run, base, quick, d, "03-quick", 0.03)
        tiles = run.topic("plugin.round8.tiles") if test_up else None
        run.save("%s/topic-quick-tiles.json" % d, json.dumps({"round8": tiles, "ambient": run.topic("plugin.ambient.tiles")}, indent=1))
        if test_up:
            before = pings(run)
            status_code, answer = run.helper("/v1/plugins/round8/actions/ping", {})
            run.check("quick: a tile's action is accepted by the helper", status_code == 202 and "op" in answer, "HTTP %s %s" % (status_code, json.dumps(answer)[:100]))
            heard = run.wait(lambda: pings(run) == before + 1, 15, 1.0)
            run.check("quick: the plugin was told, and answered", bool(heard), "plugin.log pings %d before, %d after" % (before, pings(run)))
            after = (run.topic("plugin.round8.tiles") or {}).get("items") or [{}]
            run.info("quick: the tile's state", "%s before, %s after" % (json.dumps(((tiles or {}).get("items") or [{}])[0].get("state")), json.dumps(after[0].get("state"))))
            time.sleep(1.0)
            pinged = run.shot(d, "04-quick-after-ping")
            share = png_change(quick, pinged) if quick and pinged else None
            run.info("quick: after the ping", "%s of the pixels changed (the tile flips on, the card counts)" % ("%.2f %%" % (share * 100) if share is not None else "unknown"))
        run.log_check(d, "quick", offset)
        run.door_close()

        # Settings > Ambient light: the rail's fifth section (picture, sound, inputs, network, ambient)
        run.ensure_home()
        offset = run.log_offset()
        run.door("settings")
        run.key("UPx10@0.15", "DOWNx4@0.6")
        time.sleep(SETTLE + 1.0)
        amb = run.shot(d, "05-settings-ambient")
        changed_vs(run, base, amb, d, "05-settings-ambient", 0.10)
        run.save("%s/helper-settings-base.ambient.json" % d, json.dumps(run.helper("/v1/settings/base.ambient")[1], indent=1))
        run.save("%s/helper-ambient.json" % d, json.dumps(run.helper("/v1/ambient")[1], indent=1))
        run.log_check(d, "settings-ambient", offset)
        run.door_close()

        # The notification drawer
        run.ensure_home()
        offset = run.log_offset()
        run.door("drawer")
        drawer = run.shot(d, "06-drawer")
        changed_vs(run, base, drawer, d, "06-drawer", 0.03)
        run.log_check(d, "drawer", offset)
        run.door_close()
        if test_up:
            run.sh("echo 'end 2' >> %s/plugin.cmd" % R)
    finally:
        try:
            run.sh(": > %s" % DOOR)
        except Exception:
            pass
        close_plugin(run, d)
    time.sleep(DOOR_WAIT)
    after = run.shot(d, "07-home-after")
    run.info("plugin: Home after", "%s of the pixels differ from before the step" % ("%.2f %%" % (png_change(base, after) * 100) if base and after and png_change(base, after) is not None else "unknown"))


# ---- step: maliit -----------------------------------------------------------------------------------------------------------------

MALIIT_COMMANDS = (
    ("find", "find / -xdev -iname '*maliit*' 2>/dev/null | head -400"),
    ("server-conf", "cat /var/lib/maliit/server.conf 2>&1"),
    ("ls", "ls -lR /usr/share/maliit* /usr/lib/maliit* /usr/lib/qt5/plugins/maliit* 2>&1 | head -400"),
    ("process", "ps -ef 2>/dev/null | grep -i '[m]aliit'; for p in $(pidof MaliitServer 2>/dev/null); do echo pid $p; grep -i 'qml\\|qrc\\|maliit\\|plugin' /proc/$p/maps | awk '{print $6}' | sort -u; done"),
    ("plugin-libs", "find / -xdev \\( -name 'libplugin-global.so*' -o -name 'libmaliit*' \\) 2>/dev/null | head -50"),
)

MALIIT_REFERENCES = "for f in $(find / -xdev -name 'libplugin-global.so*' 2>/dev/null | head -4); do echo \"== $f\"; grep -a -o -E 'qrc:[A-Za-z0-9_./-]*|[A-Za-z0-9_./-]+\\.qml' $f | sort -u | head -200; done"
MALIIT_QML = "find / -xdev -ipath '*maliit*' \\( -name '*.qml' -o -name 'qmldir' -o -name '*.js' -o -name '*.json' -o -name '*.conf' -o -name '*.xml' \\) 2>/dev/null"
MAX_TREE_BYTES = 80 * 1024 * 1024


def step_maliit(run, a):
    d = "maliit"
    run.dir(d)
    for name, cmd in MALIIT_COMMANDS:
        run.save("%s/%s.txt" % (d, name), run.sh(cmd, timeout=180))
    refs = run.sh(MALIIT_REFERENCES, timeout=120)
    run.save("%s/references.txt" % d, refs)
    run.save("%s/found.txt" % d, run.sh("find / -xdev -iname '*maliit*' 2>/dev/null | wc -l"))
    listing = run.sh(MALIIT_QML, timeout=180)
    files = [l.strip() for l in listing.splitlines() if l.strip().startswith("/")]
    run.save("%s/qml-files.txt" % d, "\n".join(files) + "\n")
    server = run.sh("pidof MaliitServer 2>/dev/null; echo; test -e /usr/sbin/MaliitServer && echo present").strip()
    run.info("maliit: the server", server.replace("\n", " ")[:80] or "not found")
    qml = [f for f in files if f.endswith(".qml")]
    if not qml:
        qrc = [l for l in refs.splitlines() if "qrc:" in l]
        run.info("maliit: QML on disk", "none: %d .qml/qrc reference(s) in the plugin library (references.txt); the keys are compiled in, so a restyle is a rebuilt resource: Nitin's decision (lg-restyles-result.md)" % len(qrc))
        return
    run.tv.put(os.path.join(run.out, d, "qml-files.txt"), R + "/maliit.list")
    sizes = run.sh("cd / && tar cf - $(sed 's#^/##' %s/maliit.list) 2>/dev/null | wc -c" % R, timeout=180).strip()
    run.save("%s/tree-size.txt" % d, sizes + "\n")
    if sizes.isdigit() and int(sizes) > MAX_TREE_BYTES:
        run.check("maliit: the tree is a size to copy", False, "%s bytes over %d; see qml-files.txt" % (sizes, MAX_TREE_BYTES))
        return
    run.sh("cd / && tar czf %s/maliit.tgz $(sed 's#^/##' %s/maliit.list) 2>/dev/null; md5sum $(cat %s/maliit.list) > %s/maliit.md5 2>/dev/null; true" % (R, R, R, R), timeout=240)
    local = os.path.join(run.out, d, "maliit.tgz")
    try:
        run.tv.get(R + "/maliit.tgz", local)
        run.tv.get(R + "/maliit.md5", os.path.join(run.out, d, "md5sum.txt"))
    except Exception as e:
        run.check("maliit: the QML tree fetched", False, str(e))
        return
    import tarfile
    tree = os.path.join(run.out, d, "tree")
    os.makedirs(tree, exist_ok=True)
    count = 0
    with tarfile.open(local) as t:
        for m in t.getmembers():
            name = m.name.lstrip("/")
            if m.isfile() and ".." not in name.split("/"):      # regular files only: a link in the tree is not followed out of it
                m.name = name
                t.extract(m, tree)
                count += 1
    os.remove(local)
    run.check("maliit: the QML tree is copied", count >= len(qml), "%d file(s) (%d .qml) under %s/tree, read-only on the set (tar and md5sum only)" % (count, len(qml), d))


# ---- step: x4 ---------------------------------------------------------------------------------------------------------------------

def monitor(run, cmd):
    return run.sh("sh %s/x4-monitor.sh %s" % (R, cmd), timeout=60)


def x4_capture(run, name, body):
    """One segment under `ls-monitor`: `body(dir)` runs with the monitor going; the capture is fetched (gzip), the census written beside it
    by arch/x4/x4-analyse.py, and the parsed messages returned."""
    d = "x4/" + name
    run.dir(d)
    run.sh("rm -f %s/x4/all.jsonl" % R)
    out = monitor(run, "start")
    run.monitor_on = True
    run.check("x4 %s: the monitor started" % name, "started pid" in out or "already" in out, out.strip()[:100])
    try:
        body(d)
    finally:
        out = monitor(run, "stop")
        run.monitor_on = False
        run.save("%s/monitor-stop.txt" % d, out)
    run.sh("gzip -c %s/x4/all.jsonl > %s/x4-%s.jsonl.gz" % (R, R, name), timeout=120)
    local = os.path.join(run.out, d, "all.jsonl.gz")
    try:
        run.tv.get("%s/x4-%s.jsonl.gz" % (R, name), local)
    except Exception as e:
        run.check("x4 %s: the capture fetched" % name, False, str(e))
        return []
    with gzip.open(local, "rt", errors="replace") as f:
        text = f.read()
    msgs = load_messages(text)
    raw = run.save("%s/all.jsonl" % d, text)
    try:
        p = subprocess.run([sys.executable, os.path.join(X4_DIR, "x4-analyse.py"), raw, "round 8 %s" % name], capture_output=True, text=True, timeout=120)
        run.save("%s/census.md" % d, p.stdout)
        if p.returncode != 0:
            run.info("x4 %s: census" % name, "x4-analyse.py failed: %s" % p.stderr.strip()[-160:])
    finally:
        os.remove(raw)     # the gzip is the kept copy
    run.check("x4 %s: the capture holds messages" % name, len(msgs) > 0, "%d messages, %d bytes of text" % (len(msgs), len(text)))
    return msgs


def x4_picker(run, a):
    def body(d):
        time.sleep(2.0)
        out = run.luna(SM + "/showInputPicker")
        run.check("picker: showInputPicker answered", out.get("returnValue") is True, json.dumps(out)[:120])
        time.sleep(3.0)
        run.shot(d, "open")
        run.save("%s/foreground-open.json" % d, json.dumps(run.luna(SAM + "/getForegroundAppInfo", '{"extraInfo":true}'), indent=1))
        closed = run.luna("luna://com.webos.service.systemUIManager/closeByAppId", json.dumps({"appId": PICKER_APP}))
        run.check("picker: closeByAppId answered", closed.get("returnValue") is True, json.dumps(closed)[:120])
        time.sleep(2.0)
        run.shot(d, "closed")
        run.check("picker: Home is in front after the close", run.ensure_home(), "foreground %s" % run.foreground())
        time.sleep(2.0)

    msgs = x4_capture(run, "picker", body)
    seen = {(m.get("destination"), m.get("method")) for m in msgs if m.get("type") == "call"}
    run.check("picker: the capture holds the open and the close",
              ("com.webos.surfacemanager", "showInputPicker") in seen and any(me == "closeByAppId" for _, me in seen),
              "showInputPicker %s, closeByAppId %s" % (("com.webos.surfacemanager", "showInputPicker") in seen, any(me == "closeByAppId" for _, me in seen)))


def x4_plex(run, a):
    if not remute(run):
        run.check("plex: the receiver is muted before playback", False, "not shown muted: nothing is played")
        return
    run.check("plex: the receiver is muted before playback", True, "")
    stages = [s.strip() for s in (a.plex_keys or "OK s4|OK s5|OK s5").split("|") if s.strip()]
    state = {"playing": False, "window": ""}

    def video_window():
        text = run.sh("luna-send -n 1 -f %s/getForegroundVideoWindow '{}' 2>&1" % SM)
        return text if PLEX in text else ""

    def body(d):
        try:
            run.luna(SAM + "/launch", json.dumps({"id": PLEX}))
            up = run.wait(lambda: run.foreground()[:1] == [PLEX], 25, 1.0)
            run.check("plex: PlxNative reaches the foreground", bool(up), "foreground %s" % run.foreground())
            if not up:
                return
            time.sleep(4.0)
            run.shot(d, "first-screen")
            pic = run.picture_mode()
            for n, stage in enumerate(stages, 1):
                run.key(*stage.split())
                time.sleep(1.0)
                run.shot(d, "after-keys-%d" % n)
                state["window"] = run.wait(video_window, 6, 2.0)
                if state["window"]:
                    break
            run.save("%s/video-window.txt" % d, state["window"] or "no video window named %s" % PLEX)
            state["playing"] = bool(state["window"])
            run.info("plex: live video window", "named PlxNative after %d key stage(s)" % n if state["playing"] else "never named PlxNative (the capture below decides)")
            if state["playing"]:
                time.sleep(float(a.plex_seconds) / 2)
                run.shot(d, "playing")
                run.info("plex: picture mode", "%s on the first screen, %s while playing" % (pic, run.picture_mode()))
                time.sleep(float(a.plex_seconds) / 2)
        finally:
            # Playback closed and Home back, before the capture ends and long before the receiver is touched.
            run.luna(SAM + "/closeByAppId", json.dumps({"id": PLEX}))
            time.sleep(2.0)
            home = run.ensure_home()
            run.check("plex: playback closed and Home back", home and PLEX not in json.dumps(run.luna(SAM + "/running")), "foreground %s" % run.foreground())
            time.sleep(3.0)

    msgs = x4_capture(run, "plex", body)
    facts = plex_facts(msgs)
    run.save("x4/plex/facts.json", json.dumps(facts, indent=1, sort_keys=True))
    run.check("plex: playback happened", state["playing"] or facts["placements"] > 0 or facts["window"],
              "live window %s, %d placement call(s) by PlxNative, window pushed %s; keys tried: %s (--plex-keys changes them; the screenshots show what each opened)"
              % (bool(state["window"]), facts["placements"], facts["window"], " | ".join(stages)))
    run.info("plex: calls PlxNative made", json.dumps(facts["calls"], sort_keys=True)[:300])
    run.info("plex: calls the compositor made", json.dumps(facts["compositor_calls"], sort_keys=True)[:300] or "none")


def standby_cycle(run, a, hold=20.0):
    """Power off, wait for Active Standby, hold, wake, wait for Active and Home. The logo record around it is kept in run.cycles."""
    events_before = run.sh("cat %s/%s 2>/dev/null" % (DATA, LOGO_EVENTS_NAME))
    offset = run.log_offset()
    run.sh("luna-send -n 1 -f %s/powerOff '{\"reason\":\"remoteKey\"}' >/dev/null 2>&1; sleep 0.4; luna-send -n 1 -f luna://com.webos.service.capture/executeOneShot "
           "'{\"path\":\"%s/poweroff.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":480,\"height\":270}' >/dev/null 2>&1" % (POWER, R))
    t_off = time.time()
    standby = run.wait(lambda: "standby" in run.power(), 45, 2.0)
    run.check("standby: the set reaches Active Standby", bool(standby), "power state %s after %.0f s" % (run.power(), time.time() - t_off))
    time.sleep(hold)
    woke = run.gate.wake()
    active = run.wait(lambda: run.power() == "active", 40, 2.0)
    run.check("wake: the set is Active again", bool(woke and active), "power state %s" % run.power())
    home = run.ensure_home(30)
    run.check("wake: Home in front after the wake", home, "foreground %s" % run.foreground())
    events_after = run.sh("cat %s/%s 2>/dev/null" % (DATA, LOGO_EVENTS_NAME))
    cycle = {"events_before": events_before, "events_after": events_after, "log": run.log_since(offset), "offset": offset}
    run.cycles.append(cycle)
    try:
        run.tv.get(R + "/poweroff.png", os.path.join(run.dir("logo"), "poweroff-frame.png"))
    except Exception:
        pass
    return cycle


def x4_standby(run, a):
    def body(d):
        cycle = standby_cycle(run, a)
        run.save("%s/log-since-poweroff.txt" % d, cycle["log"])
        remute(run)
        run.shot(d, "after-wake")

    msgs = x4_capture(run, "standby", body)
    phases = power_phases(msgs)
    run.save("x4/standby/power-phases.json", json.dumps(phases, indent=1))
    want = ["request power off", "prepare power off", "prepare power on"]
    have = [w for w in want if w in phases]
    run.check("standby: the power rounds reached the compositor's subscription", have == want and any("standby" in p for p in phases),
              "phases %s" % phases)
    mine = sum(1 for m in msgs if str(m.get("sender", "")) == "com.webos.surfacemanager")
    run.info("standby: the compositor's own sender side", "%d message(s) sent by com.webos.surfacemanager (pass 2 saw none of its pushes: a restarted process, shell-architecture.md)" % mine)
    acks = sum(1 for m in msgs if m.get("method") == "responsePreparePowerOff")
    run.info("standby: prepare-power-off acknowledgements", "%d responsePreparePowerOff call(s) (pass 2: 28 within 160 ms)" % acks)


def step_x4(run, a):
    run.dir("x4")
    run.ensure_home()
    x4_picker(run, a)
    x4_plex(run, a)
    x4_standby(run, a)


# ---- step: logo -------------------------------------------------------------------------------------------------------------------

def logo_prepare(run, a):
    """Before any power-off of the run: the logo switch on (turned on here if it was off, and put back at the end) and the staged
    power-off window one that writes the record. True when a power-off now will leave one."""
    d = "logo"
    run.dir(d)
    status = json_of(run.sh("%s logo status 2>&1" % TOOL))
    on = status.get("on") is True
    run.logo_was_on = on
    run.info("logo: the switch", "on" if on else "off")
    if not on:
        if a.keep_logo_off:
            run.skip("logo: the power-off record", "the logo switch is off and --keep-logo-off says to leave it")
            return False
        out = run.sh("%s logo on 2>&1" % TOOL, timeout=120)
        run.save("%s/logo-on.json" % d, out)
        if not run.check("logo: the switch turned on for the record", json_of(out).get("ok") is True, out.strip()[:200]):
            return False
    staged = run.sh("grep -c %s %s/systemui/MasterBrand/MasterBrand.qml 2>/dev/null" % (LOGO_EVENTS_NAME, DATA)).strip()
    return run.check("logo: the staged power-off window writes the record", staged.isdigit() and int(staged) > 0,
                     "%s reference(s) to %s in the staged MasterBrand.qml%s" % (staged or "no", LOGO_EVENTS_NAME,
                                                                           "" if staged.isdigit() and int(staged) > 0 else
                                                                           ": deploy tools/ and payload/systemui, `lumatv apply` (notify-slice-result.md, \"Set steps\")"))


def step_logo(run, a):
    d = "logo"
    run.dir(d)
    if not run.logo_ready:
        return
    if not run.cycles:
        remute(run)
        standby_cycle(run, a, hold=10.0)
        remute(run)
    cycle = run.cycles[-1]
    run.save("%s/logo-events.before.txt" % d, cycle["events_before"])
    run.save("%s/logo-events.after.txt" % d, cycle["events_after"])
    fresh = new_lines(cycle["events_before"], cycle["events_after"])
    run.save("%s/logo-events.new.txt" % d, "\n".join(fresh) + "\n")
    events = parse_logo_events("\n".join(fresh))
    launch, answered, gap = power_off_record(events)
    run.check("logo: the power-off window ran and said so", launch is not None,
              launch["step"] if launch else "no `power-off logo N launch` line among %d new line(s): the redirect did not run the window" % len(fresh))
    run.check("logo: the window answered tvpowerd", answered is not None,
              ("up=%s" % answered["up"]) if answered else "no `power-off answered tvpowerd` line")
    if launch and answered:
        if gap is None:
            run.info("logo: the gap", "up= is `-` (/proc/uptime is not readable through TextFile); the wall stamps %s and %s say it" % (launch["wall"], answered["wall"]))
        else:
            run.info("logo: launch to answer", "%.2f s (LumaTV's logo is meant to take about 0.5 s, LG's about 3 s)" % gap)
    logo_lines = [l for l in cycle["log"].splitlines() if "[Logo]" in l]
    run.save("%s/log-logo-lines.txt" % d, "\n".join(logo_lines) + "\n")
    run.info("logo: [Logo] lines in the log", "%d: %s" % (len(logo_lines), " | ".join(l.strip()[:90] for l in logo_lines[:4])))
    bad, _ = gate.log_failures(cycle["log"].splitlines())
    run.check("logo: no failure line in the log around the power-off", not bad, bad[0][:160] if bad else "")


# ---- the end ----------------------------------------------------------------------------------------------------------------------

def finish(run, a):
    """What always runs, in this order: the monitor stopped, the test plugin off, playback closed and Home back and seen, then the
    receiver, the logo switch, the summary, standby."""
    try:
        if run.monitor_on:
            monitor(run, "stop")
            run.monitor_on = False
    except Exception as e:
        run.problem("the ls-monitor may still be running (%s): pkill -x ls-monitor" % e)
    try:
        close_plugin(run)
    except Exception as e:
        run.problem("the test plugin may still be on (%s): see docs/research/round8/tv_round8.py close_plugin" % e)
    home = quiet = False
    try:
        run.luna(SAM + "/closeByAppId", json.dumps({"id": PLEX}))
        run.luna("luna://com.webos.service.systemUIManager/closeByAppId", json.dumps({"appId": PICKER_APP}))
        if run.door_used:
            run.sh(": > %s" % DOOR)
        home = run.ensure_home(30)
        run.check("end: Home in front before the receiver is touched", home, "foreground %s" % run.foreground())
        # PlxNative may still be running behind Home (it would be heard the moment the receiver is unmuted): closed again, and looked for.
        def plex_gone():
            if PLEX not in json.dumps(run.luna(SAM + "/running")):
                return True
            run.luna(SAM + "/closeByAppId", json.dumps({"id": PLEX}))
            return False

        quiet = bool(run.wait(plex_gone, 8, 2.0))
        run.check("end: PlxNative is not running", quiet, "SAM's running list")
    except Exception as e:
        run.problem("Home was not brought back (%s)" % e)
    try:
        if run.logo_was_on is False and not a.keep_logo_off:
            out = run.sh("%s logo off 2>&1" % TOOL, timeout=120)
            run.info("end: the logo switch put back off", out.strip()[:120])
    except Exception as e:
        run.problem("THE LOGO SWITCH IS STILL ON (%s): %s logo off" % (e, TOOL))
    try:
        if run.muted_by_us and not (home and quiet):
            run.problem("THE RECEIVER IS STILL MUTED ON PURPOSE: Home is not confirmed in front or PlxNative still runs, so it was not unmuted (it would be heard): unmute %s once the screen is quiet" % RECEIVER)
        elif run.muted_by_us:
            ok = run.set_mute(False)
            if not ok:
                run.problem("THE RECEIVER MAY STILL BE MUTED: unmute %s" % RECEIVER)
            run.check("end: the receiver is put back", ok, "unmuted (read back from the helper, or Home Assistant)")
        elif run.muted_before:
            run.info("end: the receiver", "it was muted before the run and is left muted")
    except Exception as e:
        run.problem("THE RECEIVER IS STILL MUTED (%s): unmute %s" % (e, RECEIVER))
    try:
        run.info("picture mode after", run.picture_mode())
    except Exception:
        pass
    try:
        if run.locked:
            run.sh("rm -rf %s %s.lock" % (R, R))
            run.locked = False
    except Exception as e:
        run.problem("/tmp/round8 stays on the set (%s)" % e)
    try:
        if not a.no_standby:
            run.sh("luna-send -n 1 -f %s/powerOff '{\"reason\":\"remoteKey\"}' >/dev/null 2>&1" % POWER)
            ok = run.wait(lambda: "standby" in run.power(), 45, 2.0)
            run.check("end: the set is in Active Standby", bool(ok), "power state %s" % run.power())
    except Exception as e:
        run.problem("the set was not put in standby (%s)" % e)


def run_slot(a, tv=None, gate_module=None, ha=None):
    if tv is None or gate_module is None:
        import tv as _tv
        import tv_gate as _gate
        tv, gate_module = tv or _tv, gate_module or _gate
    stamp = time.strftime("%Y-%m-%d-%H%M")
    out = a.out or os.path.join(HERE, stamp)
    run = Run(tv, gate_module, ha or HomeAssistant(gate_module), out, a)
    skip = set(a.skip.split(",")) - {""}
    started = False
    try:
        preflight(run, a)
        started = True
        run.info("started", stamp)
        muted = mute_receiver(run)
        if "logo" not in skip and muted:
            run.info("step", "logo (switch)")
            run.logo_ready = logo_prepare(run, a)
        steps = (("plugin", step_plugin), ("maliit", step_maliit), ("x4", step_x4), ("logo", step_logo))
        for name, fn in steps:
            if name in skip:
                run.skip(name, "--skip")
                continue
            if name in ("x4", "logo") and not muted:
                run.check("%s: the receiver was muted first" % name, False, "it could not be shown muted, so nothing that makes a sound or a standby cycle ran")
                continue
            run.info("step", name)
            try:
                fn(run, a)
            except SystemExit:
                raise
            except Exception as e:
                import traceback
                run.check("%s: ran to its end" % name, False, "%s: %s" % (type(e).__name__, e))
                run.save("%s/traceback.txt" % name, traceback.format_exc())
    finally:
        if started:
            finish(run, a)
        else:
            try:
                if run.locked:
                    run.sh("rm -rf %s %s.lock" % (R, R))
                if run.woke and not a.no_standby:
                    run.sh("luna-send -n 1 -f %s/powerOff '{\"reason\":\"remoteKey\"}' >/dev/null 2>&1" % POWER)
            except Exception:
                pass
    run.save("summary-final.txt", "round 8 slot %s\n\n%s\n\nPROBLEMS: %s\n" % (stamp, "\n".join(run.lines), run.problems or "none"))
    print("evidence: %s" % os.path.relpath(out, ROOT))
    return 1 if (run.failed or run.problems) else 0


def main(argv=None, tv=None, gate_module=None, ha=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--plan", action="store_true", help="print the steps; touch nothing (the default)")
    ap.add_argument("--go", action="store_true", help="run the slot (only with Main's go)")
    ap.add_argument("--skip", default="", help="steps to leave out: plugin,x4,maliit,logo")
    ap.add_argument("--approve-test-plugin", action="store_true",
                    help="Nitin has said yes to the test plugin being switched on in the helper's roster for the run (Principle 0); without it the plugin step draws only the helper's own plugins")
    ap.add_argument("--keep-logo-off", action="store_true", help="do not turn the logo switch on for the record if it is off")
    ap.add_argument("--no-standby", action="store_true", help="leave the set on at the end")
    ap.add_argument("--plex-keys", default="", help="remote keys that start playback in PlxNative, stages separated by | (kb.py tokens; default `OK s4|OK s5|OK s5`)")
    ap.add_argument("--plex-seconds", type=float, default=20.0, help="how long playback runs once the video window names PlxNative")
    ap.add_argument("--out", help="where the evidence goes (default docs/research/round8/<date>-<time>)")
    a = ap.parse_args(argv)
    bad = set(a.skip.split(",")) - {""} - set(STEPS)
    if bad:
        ap.error("--skip knows %s, not %s" % (", ".join(STEPS), ", ".join(sorted(bad))))
    if a.go:
        return run_slot(a, tv, gate_module, ha)
    skip = set(a.skip.split(",")) - {""}
    for n, (step, why, always) in enumerate(plan(skip, a.approve_test_plugin, not a.no_standby, a.keep_logo_off), 1):
        print("%2d. %-9s %s%s" % (n, step, why, "   [always, even after a failure]" if always else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
