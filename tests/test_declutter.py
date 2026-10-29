"""Run with: python3 -m unittest tests.test_declutter

tools/declutter.sh against a made-up set: a root directory with LG's customization lists, SAM's deleted list and the app
folders (DECLUTTER_ROOT), and stand-ins for luna-send, systemctl, chattr and mount on the PATH that record what the tool
asked and answer as the set does (a settings store, SAM's app list, an install service that removes a folder). The
`autoinstall` group is what is exercised: LG's two update switches held, the factory stubs removed without a copy (the
owner's choice), every not-installed id of the country lists on the deleted list, and `off` putting the files back byte
for byte without undoing another group that is still on; the `apps` group shows an item with a copy coming back.
"""
import json
import os
import shutil
import stat
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TOOL = os.path.join(REPO, "tools", "declutter.sh")
ITEMS = os.path.join(REPO, "payload", "declutter", "items.txt")
GROUPS = os.path.join(REPO, "payload", "declutter", "groups.txt")

STUBS = ["tv.twitch.tv.starshot.lg", "karaokesmart", "com.joinhighbrow.highbrow", "com.deltatre.tennistv.lgapp"]
INSTALLED = ["netflix", "amazon", "youtube.leanback.v4", "com.apple.appletv", "com.webos.app.browser", "com.miniclip.eightballpool"]
US7 = {"applications_dosci": ["amazon", "youtube.leanback.v4", "dazn"] + STUBS}
USA = {"applications_dosci": ["netflix", "amazon", "com.disney.disneyplus-prod", "hulu", "com.apple.appletv", "com.lgtvod.us.app",
                              "youtube.leanback.v4", "com.miniclip.eightballpool", "shortcutcopilot", "epoxyaisports", "com.folder.only"]}
DELETED = {"deletedList": ["com.disney.disneyplus-prod", "hulu", "youtube.leanback.v4", "epoxyaisports", "netflix", "shortcutcopilot"]}

LUNA_SEND = r'''#!/usr/bin/env python3
import json, os, shutil, sys
state_path = os.environ["FAKE_STATE"]
root = os.environ["DECLUTTER_ROOT"]
args = [a for a in sys.argv[1:] if a not in ("-n", "1", "-f")]
uri, payload = args[0], json.loads(args[1] if len(args) > 1 else "{}")
with open(state_path) as f:
    st = json.load(f)
st.setdefault("calls", []).append({"uri": uri, "payload": payload})
out = {"returnValue": True}
if uri.endswith("settingsservice/getSystemSettings"):
    cat = st["settings"].get(payload.get("category", ""), {})
    found = {k: cat[k] for k in payload.get("keys", []) if k in cat}
    out = {"returnValue": True, "settings": found} if found else {"returnValue": False, "errorText": "There is no matched result from DB"}
elif uri.endswith("settingsservice/setSystemSettings"):
    st["settings"].setdefault(payload["category"], {}).update(payload["settings"])
elif uri.endswith("applicationManager/listApps"):
    out = {"returnValue": True, "apps": [{"id": i} for i in st["apps"]]}
elif uri.endswith("appInstallService/remove"):
    i = payload["id"]
    for d in ("usr/palm/applications", "usr/palm/packages"):
        shutil.rmtree(os.path.join(root, "media/cryptofs/apps", d, i), ignore_errors=True)
    for s in (".control", ".list"):
        try:
            os.remove(os.path.join(root, "media/cryptofs/apps/var/lib/opkg/info", i + s))
        except OSError:
            pass
    st["apps"] = [a for a in st["apps"] if a != i]
with open(state_path, "w") as f:
    json.dump(st, f)
print(json.dumps(out))
'''
SYSTEMCTL = '#!/bin/sh\ncase "$1" in is-active) echo inactive ;; esac\nexit 0\n'
NOOP = "#!/bin/sh\nexit 0\n"


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def read_json(path):
    with open(path) as f:
        return json.load(f)


class FakeSet:
    """A root directory shaped like the set, and the fake commands beside it."""

    def __init__(self, base):
        self.root = os.path.join(base, "root")
        self.bin = os.path.join(base, "bin")
        self.state = os.path.join(base, "state.json")
        apps = os.path.join(self.root, "media/cryptofs/apps")
        for i in STUBS:
            write(os.path.join(apps, "usr/palm/applications", i, "appinfo.json"), json.dumps({"id": i, "type": "stub", "version": "0.0.1"}))
            write(os.path.join(apps, "var/lib/opkg/info", i + ".control"), "Package: %s\n" % i)
            write(os.path.join(apps, "var/lib/opkg/info", i + ".list"), "/x\n")
        for i in ("netflix", "com.miniclip.eightballpool", "com.folder.only"):          # the last: a folder SAM has not listed (yet)
            write(os.path.join(apps, "usr/palm/applications", i, "appinfo.json"), json.dumps({"id": i, "type": "web"}))
            write(os.path.join(apps, "usr/palm/packages", i, "packageinfo.json"), json.dumps({"id": i}))
        write(os.path.join(self.root, "var/palm/customization/launchpoints/US7/applist.json"), json.dumps(US7))
        write(os.path.join(self.root, "var/palm/customization/launchpoints/USA/applist.json"), json.dumps(USA))
        write(os.path.join(self.root, "var/preferences/com.webos.applicationManager/deletedSystemAppList.json"), json.dumps(DELETED))
        os.makedirs(os.path.join(self.root, "var/lib/lumatv"))
        os.makedirs(os.path.join(self.root, "etc/systemd/system"))
        for name, body in (("luna-send", LUNA_SEND), ("systemctl", SYSTEMCTL), ("chattr", NOOP), ("mount", NOOP), ("umount", NOOP)):
            p = os.path.join(self.bin, name)
            write(p, body)
            os.chmod(p, os.stat(p).st_mode | stat.S_IEXEC)
        with open(self.state, "w") as f:
            json.dump({"settings": {"option": {"appUpdateMode": "auto", "livePlus": "off"}, "support": {"automaticUpdate": "off"}},
                       "apps": list(INSTALLED), "calls": []}, f)
        self.originals = {p: self.file(p) for p in self.list_files()}

    def path(self, rel):
        return os.path.join(self.root, rel)

    def list_files(self):
        return ["var/palm/customization/launchpoints/US7/applist.json", "var/palm/customization/launchpoints/USA/applist.json",
                "var/preferences/com.webos.applicationManager/deletedSystemAppList.json"]

    def file(self, rel):
        with open(self.path(rel)) as f:
            return f.read()

    def run(self, *verb, dry=False, env=None):
        env = dict(os.environ, DECLUTTER_ROOT=self.root, FAKE_STATE=self.state, PATH=self.bin + os.pathsep + os.environ.get("PATH", ""), **(env or {}))
        if dry:
            env["DECLUTTER_DRY"] = "1"
        p = subprocess.run(["sh", TOOL] + list(verb), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        return p.returncode, p.stdout.decode(), p.stderr.decode()

    def status(self):
        rc, out, err = self.run("status")
        assert rc == 0, err
        return json.loads(out)

    def st(self):
        return read_json(self.state)

    def calls(self, suffix):
        return [c["payload"] for c in self.st()["calls"] if c["uri"].endswith(suffix)]

    def deleted(self):
        return read_json(self.path(self.list_files()[2]))["deletedList"]

    def installed_dirs(self):
        return sorted(os.listdir(self.path("media/cryptofs/apps/usr/palm/applications")))


class Autoinstall(unittest.TestCase):
    def setUp(self):
        self.base = tempfile.mkdtemp(prefix="declutter-")
        self.addCleanup(shutil.rmtree, self.base, True)
        self.fake = FakeSet(self.base)

    def test_items_are_well_formed_and_the_group_is_in_the_ads_profile(self):
        groups = {}
        with open(GROUPS) as f:
            for line in f:
                if line.strip() and not line.startswith("#"):
                    name, profile = line.split("|")[:2]
                    groups[name] = profile
        self.assertEqual(groups.get("autoinstall"), "ads")
        with open(TOOL) as f:
            src = f.read()
        deny = set(src.split('DENY="', 1)[1].split('"', 1)[0].split())
        kinds = {}
        with open(ITEMS) as f:
            for line in f:
                if not line.strip() or line.startswith("#"):
                    continue
                kind, group, name = line.rstrip("\n").split("|")[:3]
                self.assertIn(kind, ("unit", "luna", "setting", "app", "sync"), line)
                self.assertIn(group, groups, line)
                if kind == "unit":
                    self.assertNotIn(name, deny, line)
                kinds.setdefault(group, []).append((kind, name))
        self.assertEqual([k for k in kinds["autoinstall"] if k[0] == "setting"],
                         [("setting", "option.appUpdateMode"), ("setting", "support.automaticUpdate")])
        self.assertEqual([n for k, n in kinds["autoinstall"] if k == "app"], ["dazn"] + STUBS)
        self.assertEqual([k for k in kinds["autoinstall"] if k[0] == "sync"], [("sync", "lists")])
        rc, out, err = self.fake.run("on", "ads", dry=True)
        self.assertEqual(rc, 0, err)
        self.assertIn("autoinstall", json.loads(out)["on"].split())
        self.assertEqual(self.fake.calls("/setSystemSettings"), [], "a dry run changed a setting")
        self.assertEqual(self.fake.calls("/remove"), [], "a dry run removed an app")
        for p, text in self.fake.originals.items():
            self.assertEqual(self.fake.file(p), text, "a dry run edited " + p)

    def test_on_holds_the_switches_removes_the_stubs_and_marks_every_missing_id_deleted(self):
        rc, out, err = self.fake.run("on", "--group", "autoinstall")
        self.assertEqual(rc, 0, err)
        self.assertEqual(json.loads(out), {"ok": True, "on": "autoinstall"})
        settings = self.fake.st()["settings"]
        self.assertEqual(settings["option"]["appUpdateMode"], "manual")
        self.assertEqual(settings["support"]["automaticUpdate"], "off")
        self.assertEqual(self.fake.calls("/setSystemSettings"), [{"category": "option", "settings": {"appUpdateMode": "manual"}}],
                         "a switch that already had its value was written")
        self.assertEqual(sorted(c["id"] for c in self.fake.calls("/remove")), sorted(STUBS), "dazn is not on the set: nothing to remove")
        self.assertEqual(self.fake.installed_dirs(), ["com.folder.only", "com.miniclip.eightballpool", "netflix"])
        backup = os.path.join(self.fake.root, "var/lib/lumatv/declutter/backup")
        self.assertEqual([f for f in os.listdir(backup) if f.startswith("app-")], [], "a stub marked nobackup was copied")
        with open(os.path.join(self.fake.root, "var/lib/lumatv/declutter/before")) as f:
            before = f.read()
        for i in STUBS:
            self.assertIn("app|%s|removed-no-backup" % i, before)
        self.assertEqual(read_json(self.fake.path("var/palm/customization/launchpoints/US7/applist.json")),
                         {"applications_dosci": ["amazon", "youtube.leanback.v4"]})
        self.assertEqual(read_json(self.fake.path("var/palm/customization/launchpoints/USA/applist.json")), USA, "the USA list named no stub")
        deleted = self.fake.deleted()
        self.assertEqual(deleted[:len(DELETED["deletedList"])], DELETED["deletedList"], "what was deleted before stays first")
        for i in ["dazn"] + STUBS:
            self.assertIn(i, deleted)
        self.assertIn("com.lgtvod.us.app", deleted, "not installed and in the USA list: the sync would add it")
        for i in INSTALLED:
            self.assertNotIn(i, deleted[len(DELETED["deletedList"]):], "an installed app was marked deleted: " + i)
        self.assertNotIn("com.folder.only", deleted, "an app that is there as a folder, before SAM lists it, must not be marked")
        status = self.fake.status()
        self.assertEqual(status["groups"]["autoinstall"], {"on": True, "inTarget": "8/8"})
        self.assertEqual(status["profile"], "custom")
        self.assertIn('setting|option.appUpdateMode|"auto"', before)
        self.assertIn('setting|support.automaticUpdate|"off"', before)

    def test_off_puts_the_files_and_the_switch_back_and_says_the_stubs_stay_gone(self):
        self.fake.run("on", "--group", "autoinstall")
        rc, out, err = self.fake.run("off", "--group", "autoinstall")
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.fake.st()["settings"]["option"]["appUpdateMode"], "auto")
        for p, text in self.fake.originals.items():
            self.assertEqual(self.fake.file(p), text, p + " did not come back as it was")
        self.assertEqual(self.fake.installed_dirs(), ["com.folder.only", "com.miniclip.eightballpool", "netflix"], "a stub removed without a copy came back")
        with open(os.path.join(self.fake.root, "var/lib/lumatv/declutter/log")) as f:
            log = f.read()
        self.assertIn("removed without a copy, not restored (the store can install them): " + " ".join(STUBS), log)
        status = self.fake.status()
        self.assertEqual(status["groups"]["autoinstall"]["on"], False)
        self.assertEqual(status["profile"], "off")
        self.assertFalse(os.path.exists(os.path.join(self.fake.root, "var/lib/lumatv/declutter/before")), "nothing is on: the record goes")

    def test_an_app_item_without_nobackup_is_copied_and_comes_back(self):
        rc, out, err = self.fake.run("on", "--group", "apps")
        self.assertEqual(rc, 0, err)
        tgz = os.path.join(self.fake.root, "var/lib/lumatv/declutter/backup/app-com.miniclip.eightballpool.tgz")
        self.assertTrue(os.path.exists(tgz))
        self.assertNotIn("com.miniclip.eightballpool", self.fake.installed_dirs())
        rc, out, err = self.fake.run("off", "--group", "apps")
        self.assertEqual(rc, 0, err)
        self.assertIn("com.miniclip.eightballpool", self.fake.installed_dirs())
        self.assertTrue(os.path.exists(self.fake.path("media/cryptofs/apps/usr/palm/packages/com.miniclip.eightballpool/packageinfo.json")))

    def test_switching_one_group_off_keeps_the_edits_of_another_that_is_still_on(self):
        rc, out, err = self.fake.run("on", "--group", "apps", "--group", "autoinstall")
        self.assertEqual(rc, 0, err)
        self.assertNotIn("com.miniclip.eightballpool", self.fake.installed_dirs())
        rc, out, err = self.fake.run("off", "--group", "autoinstall")
        self.assertEqual(rc, 0, err)
        usa = read_json(self.fake.path("var/palm/customization/launchpoints/USA/applist.json"))["applications_dosci"]
        self.assertNotIn("com.miniclip.eightballpool", usa, "the apps group is still on, its strip was undone")
        self.assertIn("com.miniclip.eightballpool", self.fake.deleted())
        self.assertNotIn("dazn", self.fake.deleted(), "the autoinstall group is off, its mark stayed")
        self.assertIn("dazn", read_json(self.fake.path("var/palm/customization/launchpoints/US7/applist.json"))["applications_dosci"])
        self.assertEqual(self.fake.status()["groups"]["apps"], {"on": True, "inTarget": "2/2"})
        rc, out, err = self.fake.run("off", "--group", "apps")
        self.assertEqual(rc, 0, err)
        for p, text in self.fake.originals.items():
            self.assertEqual(self.fake.file(p), text, p + " did not come back as it was")



MOUNT = r'''#!/bin/sh
echo "mount $*" >> "$FAKE_CALLS"
[ "$1" = --bind ] && printf 'stub %s tmpfs rw 0 0\n' "$3" >> "$DECLUTTER_MOUNTS"
exit 0
'''
UMOUNT = r'''#!/bin/sh
echo "umount $*" >> "$FAKE_CALLS"
grep -vF " $1 " "$DECLUTTER_MOUNTS" > "$DECLUTTER_MOUNTS.new"
mv "$DECLUTTER_MOUNTS.new" "$DECLUTTER_MOUNTS"
exit 0
'''
SYSTEMCTL_EARLY = r'''#!/bin/sh
echo "systemctl $*" >> "$FAKE_CALLS"
case "$1" in
  is-active) shift; for u; do case " $FAKE_ACTIVE " in *" $u "*) echo active ;; *) echo inactive ;; esac; done ;;
esac
exit 0
'''
SETSID = '#!/bin/sh\necho "setsid $*" >> "$FAKE_CALLS"\nexit 0\n'
PRIVACY_GROUPS = ("ads", "uploaders", "push")


class Early(unittest.TestCase):
    """`declutter.sh early`, the step autostart.sh's worker runs about 14 s into a boot.

    On the set LG's pbs started at +16.8 s and contentminer at +19 s, while the first pass of `boot` came at +73 s; the
    masks and the stubs now go down at +14 s. The made-up set keeps the units and programs the groups name as files, a
    stand-in `mount` writes the mount table the tool reads (DECLUTTER_MOUNTS), and a stand-in `setsid` records the settle
    watch instead of starting it.
    """

    def setUp(self):
        self.boot_id = "boot-1"
        self.base = tempfile.mkdtemp(prefix="declutter-early-")
        self.addCleanup(shutil.rmtree, self.base, True)
        self.fake = FakeSet(self.base)
        self.mounts = os.path.join(self.base, "mounts")
        self.log = os.path.join(self.base, "calls")
        write(self.mounts, "")
        write(self.log, "")
        for name, body in (("mount", MOUNT), ("umount", UMOUNT), ("systemctl", SYSTEMCTL_EARLY), ("setsid", SETSID)):
            p = os.path.join(self.fake.bin, name)
            write(p, body)
            os.chmod(p, 0o755)
        self.units, self.stubs = {}, {}
        with open(ITEMS) as f:
            for line in f:
                if not line.strip() or line.startswith("#"):
                    continue
                kind, group, name, arg = line.rstrip("\n").split("|")[:4]
                if kind == "unit":
                    self.units.setdefault(group, []).append(name)
                    write(self.fake.path("etc/systemd/system/" + name), "[Unit]\n")
                elif kind == "luna" and arg == "stub":
                    self.stubs.setdefault(group, []).append(name)
                    write(self.fake.path(name.lstrip("/")), "#!/bin/sh\necho the real program\n")

    def dc(self, *verb, **env):
        env = dict({"DECLUTTER_MOUNTS": self.mounts, "FAKE_CALLS": self.log, "FAKE_ACTIVE": "", "DECLUTTER_SETTLE_STEP": "0.1", "DECLUTTER_BOOT_ID": self.boot_id}, **env)
        return self.fake.run(*verb, env=env)

    def status(self):
        rc, out, err = self.dc("status")
        self.assertEqual(rc, 0, err)
        return json.loads(out)

    def d(self, name):
        return self.fake.path("var/lib/lumatv/declutter/" + name)

    def activate(self, *groups):
        write(self.d("active"), "".join(g + "\n" for g in groups))

    def persist(self):
        link = self.fake.path("var/lib/webosbrew/init.d/52-declutter")
        os.makedirs(os.path.dirname(link), exist_ok=True)
        os.symlink(TOOL, link)

    def reboot(self):
        shutil.rmtree(self.fake.path("run"), ignore_errors=True)
        write(self.mounts, "")
        self.boot_id = "boot-%d" % (int(self.boot_id.split("-")[1]) + 1)

    def mounted(self):
        with open(self.mounts) as f:
            return sorted(l.split()[1] for l in f if l.strip())

    def calls(self):
        with open(self.log) as f:
            return [l.rstrip("\n") for l in f if l.strip()]

    def masked(self):
        d = self.fake.path("run/systemd/system.control")
        return sorted(os.listdir(d)) if os.path.isdir(d) else []

    def test_it_masks_the_units_and_stubs_the_programs_of_the_groups_that_are_on_and_records_the_pass(self):
        self.activate(*PRIVACY_GROUPS)
        self.persist()
        rc, out, err = self.dc("early")
        self.assertEqual(rc, 0, err)
        res = json.loads(out)
        self.assertEqual((res["ok"], res["early"], res["running"]), (True, True, []))
        units = sorted(u for g in PRIVACY_GROUPS for u in self.units[g])
        programs = sorted(self.fake.path(x.lstrip("/")) for g in PRIVACY_GROUPS for x in self.stubs.get(g, []))
        self.assertEqual(self.masked(), units)
        for u in units:
            self.assertEqual(os.readlink(self.fake.path("run/systemd/system.control/" + u)), "/dev/null", u)
        self.assertEqual(self.mounted(), programs)
        self.assertTrue(os.path.isfile(self.d("stub")), "the do-nothing program was not laid down")
        self.assertEqual([c for c in self.calls() if c.startswith("systemctl daemon-reload")], ["systemctl daemon-reload"], "one reload for all masks")
        self.assertFalse(os.path.exists(self.fake.path("run/systemd/system.control/" + self.units["voice"][0])), "a group that is off was masked")
        rec = read_json(self.d("early.json"))
        self.assertEqual((rec["ran"], rec["running"], rec["why"]), (True, [], ""))
        self.assertEqual((sorted(rec["masked"]), sorted(rec["stubbed"])), (units, sorted(x for g in PRIVACY_GROUPS for x in self.stubs.get(g, []))))
        self.assertTrue(any(c.startswith("setsid") and c.endswith("--early-settle") for c in self.calls()), "the settle watch was not started")
        early = self.status()["early"]
        self.assertEqual((early["last"]["ran"], early["pending"]), (True, True), "the marker stays until the set has answered")
        with open(self.d("log")) as f:
            self.assertRegex(f.read(), r"early: %d unit\(s\) masked, %d program\(s\) stubbed, already running: none, stubs at [\d.]+ s, masks at [\d.]+ s, done at uptime [\d.]+ s" % (len(units), len(programs)))

    def test_a_second_pass_lays_nothing_twice(self):
        self.activate("ads")
        self.persist()
        self.dc("early")
        os.remove(self.d(".early-pending"))
        before = self.calls()
        rc, out, err = self.dc("early")
        self.assertEqual(rc, 0, err)
        self.assertTrue(json.loads(out)["early"])
        self.assertEqual(self.mounted(), sorted(self.fake.path(x.lstrip("/")) for x in self.stubs["ads"]))
        self.assertEqual([c for c in self.calls()[len(before):] if c.startswith(("mount", "systemctl daemon-reload"))], [])

    def test_it_stops_and_kills_what_already_runs_and_says_so(self):
        sleep = shutil.which("sleep")
        exe = self.fake.path("usr/sbin/pbs")
        shutil.copy(sleep, exe)
        proc = subprocess.Popen([exe, "60"])
        self.addCleanup(lambda: proc.poll() is None and proc.kill())
        time.sleep(0.3)
        if proc.poll() is not None:
            self.skipTest("this host's sleep does not run under another name")
        self.activate("ads")
        self.persist()
        rc, out, err = self.dc("early", FAKE_ACTIVE="contentminer.service")
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(json.loads(out)["running"]), ["/usr/sbin/pbs", "contentminer.service"])
        self.assertIsNotNone(proc.wait(timeout=10), "the program LG started before the hook was left running")
        self.assertIn("systemctl stop --no-block contentminer.service", self.calls())
        self.assertIn(self.fake.path("usr/sbin/pbs"), self.mounted(), "it was killed but not stubbed")
        with open(self.d("log")) as f:
            log = f.read()
        self.assertIn("killed a running copy, pid %d" % proc.pid, log)
        self.assertIn("already running: contentminer.service /usr/sbin/pbs, stubs at", log)
        self.assertEqual(read_json(self.d("early.json"))["running"], ["contentminer.service", "/usr/sbin/pbs"])

    def test_a_boot_that_is_not_to_be_touched_is_left_alone_and_says_why(self):
        def touch(name):
            return lambda: write(self.d(name), "")
        cases = [("persist is off", lambda: os.remove(self.fake.path("var/lib/webosbrew/init.d/52-declutter"))),
                 ("the skip file is present", touch("skip")),
                 ("the previous boot never completed", touch(".boot-pending")),
                 ("the previous boot did not settle after the early pass", touch(".early-pending"))]
        for why, prepare in cases:
            with self.subTest(why):
                self.setUp()
                self.activate("ads")
                self.persist()
                prepare()
                rc, out, err = self.dc("early")
                self.assertEqual(rc, 0, err)
                self.assertEqual(json.loads(out), {"ok": True, "early": False, "why": why})
                self.assertEqual((self.masked(), self.mounted()), ([], []))
                rec = read_json(self.d("early.json"))
                self.assertEqual((rec["ran"], rec["why"], rec["masked"]), (False, why, []))
                if why.startswith("the previous boot did not"):
                    self.assertFalse(os.path.exists(self.d(".early-pending")), "the skip must happen once")
                    self.assertTrue(json.loads(self.dc("early")[1])["early"], "the boot after a skipped one is covered again")
                if why.startswith("the previous boot never"):
                    self.assertTrue(os.path.exists(self.d(".boot-pending")), "the marker is `boot`'s to consume")
        self.setUp()
        rc, out, err = self.dc("early")
        self.assertEqual(json.loads(out), {"ok": True, "early": False, "why": "no group is on"})
        self.assertFalse(os.path.exists(self.d("early.json")))

    def test_the_settle_watch_clears_the_marker_only_when_the_set_answers(self):
        self.activate("ads")
        self.persist()
        self.dc("early")
        self.assertTrue(os.path.exists(self.d(".early-pending")))
        self.dc("--early-settle")
        self.assertTrue(os.path.exists(self.d(".early-pending")), "the set never answered")
        with open(self.d("log")) as f:
            self.assertIn("early: the set did not settle; the next boot skips the early pass once", f.read())
        self.dc("--early-settle", FAKE_ACTIVE="sam.service webapp-mgr.service")
        self.assertFalse(os.path.exists(self.d(".early-pending")))
        with open(self.d("log")) as f:
            self.assertIn("early: settled", f.read())

    def test_status_says_whether_the_step_will_run_at_the_next_boot(self):
        def hook():
            return self.status()["early"]["hook"]
        self.activate("ads")
        self.assertEqual(self.status()["early"], {"hook": False, "pending": False, "thisBoot": False, "last": None})
        write(self.fake.path("var/systemd/system/env/kdump.env"), "PATH=/var/lib/lumatv/earlybin:/usr/sbin:/usr/bin:/sbin:/bin\n")
        write(self.fake.path("var/lib/lumatv/earlybin/grep"), "#!/bin/sh\n")
        os.chmod(self.fake.path("var/lib/lumatv/earlybin/grep"), 0o755)
        write(self.fake.path("var/lib/lumatv/config.json"), '{"persist": true}\n')
        self.assertFalse(hook(), "this tool's own persist link is missing")
        self.persist()
        self.assertTrue(hook())
        write(self.d("skip"), "")
        self.assertFalse(hook(), "the skip file switches it off")
        os.remove(self.d("skip"))
        self.assertTrue(hook())
        write(self.fake.path("var/lib/lumatv/config.json"), '{"persist": false}\n')
        self.assertFalse(hook(), "the worker is switched off")
        write(self.fake.path("var/lib/lumatv/config.json"), '{"persist": true}\n')
        os.chmod(self.fake.path("var/lib/lumatv/earlybin/grep"), 0o644)
        self.assertFalse(hook(), "the shim is not executable")

    def test_after_a_reboot_the_step_brings_back_what_the_switch_put_in_place(self):
        self.assertEqual(self.dc("on", "--group", "ads")[0], 0)
        self.assertEqual(self.dc("persist", "on")[0], 0)
        self.assertEqual(self.status()["groups"]["ads"], {"on": True, "inTarget": "6/6"})
        self.reboot()
        self.assertEqual(self.status()["groups"]["ads"], {"on": True, "inTarget": "0/6"}, "the masks and binds are gone with the boot")
        self.assertTrue(json.loads(self.dc("early")[1])["early"])
        self.assertEqual(self.status()["groups"]["ads"], {"on": True, "inTarget": "6/6"})
        self.assertTrue(self.status()["early"]["last"]["ran"])
        self.assertEqual(self.dc("on", "--group", "uploaders")[0], 0)
        self.assertIsNone(self.status()["early"]["last"], "a pass says nothing about a group switched on after it")

    def test_the_worker_starts_it_first_in_parallel_and_bounded(self):
        with open(os.path.join(REPO, "tools", "autostart.sh"), encoding="utf-8") as f:
            text = f.read()
        call = 'timeout 20 "$APPDIR/tools/declutter.sh" early'
        order = [text.index(s) for s in ("--worker", call, 'if [ -f "$STATE/bootcap" ]', '"$CLI" homestub-boot', '"$CLI" compositor-boot', '"$CLI" boot-bind')]
        self.assertEqual(order, sorted(order), "the early step comes before the logo capture and the Home stub")
        line = [l for l in text.split("\n") if call in l][0]
        self.assertTrue(line.rstrip().endswith(") &"), "it must not hold the worker back")
        self.assertIn("</dev/null", line)

    def test_the_stubs_are_in_before_the_masks_are_loaded_and_the_pass_times_itself(self):
        self.activate(*PRIVACY_GROUPS)
        self.persist()
        rc, out, err = self.dc("early")
        self.assertEqual(rc, 0, err)
        calls = self.calls()
        binds = [i for i, c in enumerate(calls) if c.startswith("mount --bind")]
        reload = [i for i, c in enumerate(calls) if c == "systemctl daemon-reload"]
        self.assertTrue(binds and reload and max(binds) < min(reload), "every stub before the one reload: %r" % calls)
        self.assertEqual([c for c in calls if c.startswith("systemctl stop")], [], "nothing was running, nothing to stop")
        rec = read_json(self.d("early.json"))
        self.assertLessEqual(rec["startSec"], rec["stubbedSec"])
        self.assertLessEqual(rec["stubbedSec"], rec["maskedSec"])
        self.assertLessEqual(rec["maskedSec"], rec["uptimeSec"])
        self.assertEqual(rec["bootId"], self.boot_id)
        self.assertEqual(len([c for c in calls if c.startswith("systemctl is-active")]), 1, "the unit states come in one call")

    def test_a_record_is_this_boots_only_when_the_boot_id_agrees(self):
        self.activate("ads")
        self.persist()
        self.dc("early")
        early = self.status()["early"]
        self.assertEqual((early["thisBoot"], early["last"]["ran"]), (True, True))
        self.reboot()
        early = self.status()["early"]
        self.assertEqual((early["thisBoot"], early["last"]["ran"]), (False, True), "the old record stays, and says it is not this boot's")
        self.dc("early")
        self.assertTrue(self.status()["early"]["thisBoot"])


if __name__ == "__main__":
    unittest.main()
