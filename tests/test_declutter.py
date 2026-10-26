"""Run with: python3 -m unittest tests.test_declutter

tools/declutter.sh against a made-up set: a root directory with LG's customization lists, SAM's deleted list and the app
folders (DECLUTTER_ROOT), and stand-ins for luna-send, systemctl, chattr and mount on the PATH that record what the tool
asked and answer as the set does (a settings store, SAM's app list, an install service that removes a folder). The
`autoinstall` group is what is exercised: LG's two update switches held, the factory stubs removed and backed up, every
not-installed id of the country lists on the deleted list, and `off` putting the files back byte for byte without undoing
another group that is still on.
"""
import json
import os
import shutil
import stat
import subprocess
import tempfile
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
                              "youtube.leanback.v4", "com.miniclip.eightballpool", "shortcutcopilot", "epoxyaisports"]}
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
        for i in ("netflix", "com.miniclip.eightballpool"):
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

    def run(self, *verb, dry=False):
        env = dict(os.environ, DECLUTTER_ROOT=self.root, FAKE_STATE=self.state, PATH=self.bin + os.pathsep + os.environ.get("PATH", ""))
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
        self.assertEqual(self.fake.installed_dirs(), ["com.miniclip.eightballpool", "netflix"])
        backup = os.path.join(self.fake.root, "var/lib/lumatv/declutter/backup")
        for i in STUBS:
            self.assertTrue(os.path.exists(os.path.join(backup, "app-%s.tgz" % i)), i)
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
        status = self.fake.status()
        self.assertEqual(status["groups"]["autoinstall"], {"on": True, "inTarget": "8/8"})
        self.assertEqual(status["profile"], "custom")
        with open(os.path.join(self.fake.root, "var/lib/lumatv/declutter/before")) as f:
            before = f.read()
        self.assertIn('setting|option.appUpdateMode|"auto"', before)
        self.assertIn('setting|support.automaticUpdate|"off"', before)

    def test_off_puts_the_files_the_switch_and_the_stubs_back(self):
        self.fake.run("on", "--group", "autoinstall")
        rc, out, err = self.fake.run("off", "--group", "autoinstall")
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.fake.st()["settings"]["option"]["appUpdateMode"], "auto")
        for p, text in self.fake.originals.items():
            self.assertEqual(self.fake.file(p), text, p + " did not come back as it was")
        self.assertEqual(self.fake.installed_dirs(), sorted(STUBS + ["com.miniclip.eightballpool", "netflix"]))
        for i in STUBS:
            self.assertTrue(os.path.exists(self.fake.path("media/cryptofs/apps/var/lib/opkg/info/%s.control" % i)), i)
        status = self.fake.status()
        self.assertEqual(status["groups"]["autoinstall"]["on"], False)
        self.assertEqual(status["profile"], "off")
        self.assertFalse(os.path.exists(os.path.join(self.fake.root, "var/lib/lumatv/declutter/before")), "nothing is on: the record goes")

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


if __name__ == "__main__":
    unittest.main()
