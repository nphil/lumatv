#!/usr/bin/env python3
"""The card app on the set: install it, launch it as SAM does, press its keys, cover it with Home, bring it back, close it. NOT RUN YET:
written offline, while the set was another worker's slot; tests/test_card_tv.py runs it against a simulated set, which says the script
agrees with itself and nothing about the TV. What it asks of the set is in docs/research/cardapp-result.md ("TV step list").

    python3 rustui/tools/tv-card.py facts      read-only: who an app runs as, the devices, the mounts, what SAM lists
    python3 rustui/tools/tv-card.py install --approved   the card ipk (build/card-ipk/, made by tools/build-ipk.sh) through tools/cardapp.sh
    python3 rustui/tools/tv-card.py scenario   launch, keys, Home, relaunch, Back, SAM's close, Back held (the app must be installed)
    python3 rustui/tools/tv-card.py remove --approved    the app off the set
    python3 rustui/tools/tv-card.py all --approved       install, scenario; the app stays installed

`--approved` says Nitin has said yes to this app being installed or removed (docs/research/cardapp-result.md, "Principle 0"): putting an
app on the set or taking it off is not something an AI does on its own. It is a reminder, not a lock.

Only with a slot, the screen on (tvpower `Active`) and Home in front. The set is left as found: the card closed, Home in front. Nothing here
restarts, stops or kills SAM or any LG service, writes a role file or changes a setting; the only things it changes are the card app's own
files (through appInstallService) and the processes of the card app. Evidence (summary.txt, the launcher's log, screenshots, SAM's own
answers) goes to docs/research/cardapp/tv-<time>/. Every line of summary.txt is PASS, FAIL or INFO; a FAIL names the step the TV step list
has a branch for.
"""
import json
import os
import re
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "docs", "research", "arch"))

APP = "org.nphil.lumatv.card"
MAIN_APP = "org.nphil.lumatv"
APPS = "/media/developer/apps/usr/palm/applications"
DIR = "%s/%s" % (APPS, APP)
LOG = "/tmp/lumatv-card.log"
HOME_APP = "com.webos.app.home"
REMOTE = "/tmp/cardtv"
SAM = "luna://com.webos.applicationManager"


class Run:
    """One run: the verdict lines, the evidence directory, and the few things asked of the set."""

    def __init__(self, tv, out):
        self.tv = tv
        self.out = out
        self.lines = []
        self.failed = 0
        os.makedirs(out, exist_ok=True)

    # ---- verdicts ----
    def say(self, kind, name, detail=""):
        line = "%s %s: %s" % (kind, name, detail)
        print(line, flush=True)
        self.lines.append(line)
        with open(os.path.join(self.out, "summary.txt"), "a") as f:
            f.write(line + "\n")

    def info(self, name, detail=""):
        self.say("INFO", name, detail)

    def check(self, name, ok, detail=""):
        if not ok:
            self.failed += 1
        self.say("PASS" if ok else "FAIL", name, detail)
        return ok

    def save(self, name, text):
        with open(os.path.join(self.out, name), "w") as f:
            f.write(text)

    # ---- the set ----
    def sh(self, cmd, timeout=60):
        return self.tv.tv(cmd, timeout=timeout)

    def luna(self, uri, payload="{}", name=None):
        text = self.sh("luna-send -n 1 -f %s '%s' 2>&1" % (uri, payload))
        if name:
            self.save(name + ".json", text)
        try:
            return json.loads(text)
        except ValueError:
            return {"returnValue": False, "raw": text}

    # SAM answers `{}` with a flat object (appId, processId, windowId); the list with the window type comes with extraInfo.
    def foreground(self):
        d = self.luna(SAM + "/getForegroundAppInfo", '{"extraInfo":true}')
        return [a.get("appId", "") for a in d.get("foregroundAppInfo", [])]

    def foreground_info(self):
        d = self.luna(SAM + "/getForegroundAppInfo", '{"extraInfo":true}', name="foreground")
        items = d.get("foregroundAppInfo", [])
        return items[0] if items else {}

    def processes(self):
        procs = {}
        for line in self.sh("sh /tmp/tv-card-probe.sh pids").splitlines():
            parts = line.split(None, 3)
            if len(parts) >= 3 and parts[0] in ("stub", "shell", "app") and parts[1].isdigit():
                procs.setdefault(parts[0], []).append({"pid": int(parts[1]), "uid": parts[2], "cmd": parts[3] if len(parts) > 3 else ""})
        return procs

    def wait(self, what, cond, seconds):
        end = time.time() + seconds
        while True:
            value = cond()
            if value:
                return value
            if time.time() >= end:
                return value
            time.sleep(0.5)

    def key(self, *tokens):
        self.sh("python3 /tmp/kb.py " + " ".join(tokens), timeout=40)

    def shot(self, name):
        time.sleep(1.0)
        remote = "%s/%s.png" % (REMOTE, name)
        self.sh("mkdir -p %s && luna-send -n 1 -f luna://com.webos.service.capture/executeOneShot "
                "'{\"path\":\"%s\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}' >/dev/null 2>&1" % (REMOTE, remote))
        try:
            self.tv.get(remote, os.path.join(self.out, name + ".png"))
        except Exception as e:  # a screenshot is evidence, not a verdict
            self.info("screenshot " + name, "not fetched: %s" % e)

    def app_log(self):
        return self.sh("cat %s 2>/dev/null" % LOG)

    def close_everything(self):
        """Leaves nothing of the card running: SAM's own close first, then a wait."""
        self.luna(SAM + "/closeByAppId", json.dumps({"id": APP}))
        self.wait("gone", lambda: not self.processes(), 8)


def preflight(run):
    state = run.sh("luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}' 2>&1")
    m = re.search(r'"state"\s*:\s*"([^"]*)"', state)
    if not m or m.group(1) != "Active":
        sys.exit("refusing to run: power state is %r, not Active (screen on)" % (m.group(1) if m else state.strip()))
    run.tv.put(os.path.join(HERE, "tv-card-probe.sh"), "/tmp/tv-card-probe.sh")
    run.tv.put(os.path.join(ROOT, "docs", "research", "perf", "kb.py"), "/tmp/kb.py")


def facts(run):
    text = run.sh("sh /tmp/tv-card-probe.sh facts")
    run.save("facts.txt", text)
    for line in text.splitlines():
        run.info("facts", line)
    apps = run.luna(SAM + "/listApps", name="listApps")
    mine = [a for a in apps.get("apps", []) if a.get("id") == APP]
    run.info("SAM lists the card app", json.dumps({k: mine[0].get(k) for k in ("type", "main", "version", "nativeLifeCycleInterfaceVersion", "folderPath", "visible")}) if mine else "no")
    run.info("foreground", ", ".join(run.foreground()) or "nothing")
    sam = run.luna(SAM + "/running", name="running")
    run.info("running apps", ", ".join(sorted(a.get("id", "") for a in sam.get("running", []))))
    # What other native apps registered as, for comparison with the launcher's `registerApp`: SAM's own list of apps with a lifecycle version.
    native = sorted(a["id"] for a in apps.get("apps", []) if a.get("type") == "native" and not a.get("systemApp"))
    run.info("dev-installed native apps", ", ".join(native))


def install(run):
    ipk = os.path.join(ROOT, "build", "card-ipk", APP + ".ipk")
    if not os.path.exists(ipk):
        sys.exit("no %s: run tools/build-ipk.sh first" % ipk)
    run.tv.put(ipk, "/tmp/%s.ipk" % APP)
    run.tv.put(os.path.join(ROOT, "tools", "cardapp.sh"), "/tmp/cardapp.sh")
    script = ('APP_ID=%s SHORT_ID=lumatv APPDIR=%s/%s CARD_IPK=/tmp/%s.ipk CARD_WAIT=180 sh -c \'log_line() { echo "$1"; }; . /tmp/cardapp.sh; card_install; echo rc=$?; card_status\''
              % (MAIN_APP, APPS, MAIN_APP, APP))
    out = run.sh(script, timeout=300)
    run.save("install.txt", out)
    run.check("install", "rc=0" in out, out.strip().splitlines()[-1] if out.strip() else "no output")
    run.sh("rm -f /tmp/%s.ipk" % APP)
    listing = run.sh("ls -l %s/bin/lumatv-card %s/bin64/lumatv-card-app %s/gfx64/launch.sh %s/gfx64/heapshim.so 2>&1" % (DIR, DIR, DIR, DIR))
    run.save("installed-files.txt", listing)
    run.check("the launcher is executable", re.search(r"^-rwx.* %s/bin/lumatv-card$" % re.escape(DIR), listing, re.M) is not None, listing.strip().splitlines()[0] if listing.strip() else "")
    apps = run.luna(SAM + "/listApps", name="listApps-after-install")
    mine = [a for a in apps.get("apps", []) if a.get("id") == APP]
    run.check("SAM lists the app as native with lifecycle 2", bool(mine) and mine[0].get("type") == "native" and mine[0].get("nativeLifeCycleInterfaceVersion") == 2,
              json.dumps(mine[0] if mine else {})[:240])
    lp = run.luna(SAM + "/listLaunchPoints", name="launchpoints")
    run.check("a launch point exists (the switcher needs one)", any(l.get("id") == APP for l in lp.get("launchPoints", [])))


def launch(run, quiet=False):
    run.luna(SAM + "/launch", json.dumps({"id": APP}))
    return run.wait("foreground", lambda: APP in run.foreground()[:1], 25)


def cpu_busy(run, pid, seconds=5):
    first = run.sh("sh /tmp/tv-card-probe.sh ticks %d" % pid).strip()
    time.sleep(seconds)
    second = run.sh("sh /tmp/tv-card-probe.sh ticks %d" % pid).strip()
    if first.isdigit() and second.isdigit():
        return int(second) - int(first)
    return None


def scenario(run):
    if run.foreground()[:1] != [HOME_APP]:
        sys.exit("Home is not in front (%s): put it there first" % (run.foreground() or "nothing"))
    run.sh("rm -f %s %s.1" % (LOG, LOG))
    run.close_everything()

    # 1. SAM launches it: card in front, registered, left alone.
    began = time.time()
    up = launch(run)
    run.check("launch: the card is in front", bool(up), "after %.1f s" % (time.time() - began) if up else "foreground is %s" % run.foreground())
    procs = run.wait("processes", lambda: run.processes() if run.processes().get("app") else None, 10) or run.processes()
    run.save("processes.txt", json.dumps(procs, indent=1))
    run.check("launch: the launcher and the 64-bit app run", bool(procs.get("stub")) and bool(procs.get("app")), json.dumps({k: [p["pid"] for p in v] for k, v in procs.items()}))
    info = run.foreground_info()
    run.check("launch: the window is a card", info.get("windowType") == "_WEBOS_WINDOW_TYPE_CARD", json.dumps(info)[:200])
    app_pid = procs["app"][0]["pid"] if procs.get("app") else 0
    stub_pid = procs["stub"][0]["pid"] if procs.get("stub") else 0
    run.info("launch: users", json.dumps({k: [p["uid"] for p in v] for k, v in procs.items()}))
    run.info("launch: processId in SAM's foreground info", "%s (the app is %s, the launcher %s)" % (info.get("processId"), app_pid, stub_pid))
    log = run.app_log()
    run.save("card.log", log)
    run.check("launch: the launcher registered with SAM", "SAM: registered" in log, "see LSRegister / SAM refused in card.log" if "SAM: registered" not in log else "")
    run.check("launch: the 64-bit path was used", "64-bit path:" in log and "falling back" not in log and "unavailable" not in log, next((l for l in log.splitlines() if "gfx64:" in l), "no gfx64 line"))
    run.check("launch: no GL error", "GL error" not in log)
    running = run.luna(SAM + "/running", name="running-card")
    run.check("launch: SAM lists it as running", any(a.get("id") == APP for a in running.get("running", [])))
    if app_pid:
        run.info("launch: app RSS kB", run.sh("sh /tmp/tv-card-probe.sh rss %d" % app_pid).strip())
    run.shot("status")
    time.sleep(20)
    alive = bool(run.processes().get("app")) and APP in run.foreground()[:1]
    run.check("launch: SAM leaves it running for 20 s", alive)

    # 2. Keys: OK opens the keys page, Back goes up.
    run.key("OK", "s1")
    run.check("keys: OK reaches the app and opens the keys page", '"page":"keys"' in run.app_log())
    run.shot("keys")
    run.key("BACK", "s1")
    log = run.app_log()
    run.check("keys: Back goes up a page and the app stays", log.count('"page":"status"') >= 1 and bool(run.processes().get("app")))
    codes = re.findall(r"input Key \{ code: (\d+), pressed: true \}", log)
    run.info("keys: codes the compositor sent (presses)", ", ".join(codes))

    # 3. Home: Home in front, the card minimized, idle, in the switcher.
    run.key("HOME", "s3")
    fg = run.foreground()
    run.check("home: Home is in front", fg[:1] == [HOME_APP], ", ".join(fg))
    log = run.app_log()
    run.check("home: the card was told it is minimized", '"state":"minimized"' in log or "State(1)" in log)
    still = bool(run.processes().get("app"))
    run.check("home: the covered card keeps running", still)
    if still:
        ticks = cpu_busy(run, run.processes()["app"][0]["pid"])
        run.check("home: the covered card is idle", ticks is not None and ticks <= 5, "%s clock ticks in 5 s" % ticks)
    recents = run.luna("luna://com.webos.service.recent/listRecents", name="recents")
    listed = json.dumps(recents)
    run.check("home: the app switcher lists the card", APP in listed, listed[:200])
    run.shot("home")

    # 4. A launch while it runs (Home's tile, the switcher): SAM's relaunch brings it back.
    before = run.processes().get("app", [{}])[0].get("pid")
    up = launch(run)
    run.check("relaunch: the card comes back to the front", bool(up), ", ".join(run.foreground()))
    log = run.app_log()
    run.check("relaunch: SAM's relaunch reached the app", "SAM: relaunch" in log and "Relaunch {" in log)
    run.check("relaunch: the same process (no second start)", run.processes().get("app", [{}])[0].get("pid") == before and before is not None)
    run.check("relaunch: the window is fullscreen again", log.rstrip().count("State(3)") >= 2 or "asking the compositor for state 3" in log)
    run.shot("relaunched")

    # 5. Back at the first page ends the app; the card below comes back.
    run.key("BACK", "s2")
    gone = run.wait("gone", lambda: not run.processes(), 8)
    run.check("back: Back at the first page ends the app", bool(gone), json.dumps(run.processes()))
    fg = run.wait("home", lambda: run.foreground()[:1] == [HOME_APP], 6)
    run.check("back: Home is in front again", bool(fg), ", ".join(run.foreground()))
    log = run.app_log()
    run.save("card.log", log)
    run.check("back: the launcher ended with the app's status", "done: status 0" in log, next((l for l in log.splitlines() if "done:" in l), "no 'done' line"))
    run.check("back: SAM no longer lists it as running", not any(a.get("id") == APP for a in run.luna(SAM + "/running").get("running", [])))
    run.check("back: its picture files are gone", run.sh("[ -d /tmp/lumatv-card-shm ] && echo there || echo gone").strip() == "gone")
    run.shot("after-back")

    # 6. SAM's own close (the switcher's close, the memory manager): `close` reaches the app, which ends.
    launch(run)
    time.sleep(3)
    run.luna(SAM + "/closeByAppId", json.dumps({"id": APP}))
    gone = run.wait("gone", lambda: not run.processes(), 10)
    log = run.app_log()
    run.check("close: SAM's close reaches the app", "SAM: close (" in log, next((l for l in log.splitlines() if "SAM: close" in l), ""))
    run.check("close: the app and its launcher end", bool(gone), json.dumps(run.processes()))

    # 7. Back held (LG turns it into Exit, which closes the app unless the window takes it).
    launch(run)
    time.sleep(3)
    run.key("BACK:1300", "s1")
    gone = run.wait("gone", lambda: not run.processes(), 10)
    run.check("exit: Back held closes the app", bool(gone), json.dumps(run.processes()))
    log = run.app_log()
    run.info("exit: how it ended", next((l for l in reversed(log.splitlines()) if "SAM: close" in l or "exit:" in l), "no close or exit line"))

    # The set as found.
    run.close_everything()
    run.save("card.log", run.app_log())
    fg = run.wait("home", lambda: run.foreground()[:1] == [HOME_APP], 10)
    run.check("end: Home is in front, nothing of the card runs", bool(fg) and not run.processes())
    run.info("done", "%d checks failed" % run.failed)


def remove(run):
    run.close_everything()
    run.tv.put(os.path.join(ROOT, "tools", "cardapp.sh"), "/tmp/cardapp.sh")
    script = ('APP_ID=%s SHORT_ID=lumatv APPDIR=%s/%s sh -c \'log_line() { echo "$1"; }; . /tmp/cardapp.sh; card_remove; echo rc=$?\'' % (MAIN_APP, APPS, MAIN_APP))
    out = run.sh(script, timeout=120)
    run.save("remove.txt", out)
    run.check("remove", "rc=0" in out, out.strip().splitlines()[-1] if out.strip() else "no output")


def main(argv=None, tv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv or argv[0] not in ("facts", "install", "scenario", "remove", "all"):
        sys.exit(__doc__)
    if argv[0] in ("install", "remove", "all") and "--approved" not in argv[1:]:
        sys.exit("%s puts an app on the set or takes it off: Nitin's go first, then pass --approved" % argv[0])
    if tv is None:
        import tv as tv_module  # docs/research/arch/tv.py (paramiko)
        tv = tv_module
    out = os.path.join(ROOT, "docs", "research", "cardapp", "tv-" + time.strftime("%Y%m%d-%H%M"))
    run = Run(tv, out)
    preflight(run)
    run.info("evidence", out)
    if argv[0] == "facts":
        facts(run)
    elif argv[0] == "install":
        install(run)
    elif argv[0] == "scenario":
        scenario(run)
    elif argv[0] == "remove":
        remove(run)
    else:
        facts(run)
        install(run)
        scenario(run)
    return 1 if run.failed else 0


if __name__ == "__main__":
    sys.exit(main())
