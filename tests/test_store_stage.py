"""Run with: python3 -m unittest tests.test_store_stage

The Content Store's overlay: tools/store.sh (staging, the mount over LG's directory, the close of the running instance, the way back)
and the `store` verb of tools/lumatv that drives it (the flag, the registry's reading of it, apply's boot re-run, revert).

`StoreTool` runs the real script with `mount` and `umount` stood in for by scripts that edit a fake /proc/mounts, and a `luna-send` that
plays SAM and WAM, against a stand-in for LG's directory (a temporary tree named as the firmware's is). What it must do: put one file,
webOSUserScripts/userScript.js, in the upper layer and nothing else; never write into LG's directory; mount nothing from a failed build or
a failed mount; take the directory SAM names only when it is the store's; end a running instance only when it changes what the next
launch loads, and with the second ask (WAM's killApp) only when the close left it listed; and give LG's directory back exactly on `off`.

`Probe` runs the same script's `probe` verb with the real receiver (tools/store_probe.py) on a free loopback port: the receiver starts before
the script is staged and its port is in the script, nothing but the mount changes in LG's directory, `off` and `on` end it and keep what it
wrote, a receiver that cannot start leaves LG's store in place.

`Verb` runs the `store` verb with a stand-in for the tool: the flag keeps the parts the tool settled, `off` removes it, `status` merges it,
and `probe` never writes the flag.
"""
import hashlib
import json
import os
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SCRIPT = os.path.join(ROOT, "tools", "store.sh")
TOOL = os.path.join(ROOT, "tools", "lumatv")
STORE_ID = "com.webos.app.discovery"

# The overlay's merged view is what the script checks after the mount, so the stand-in shows the upper layer in the target and takes it away
# again: webOSUserScripts is the one thing the upper layer holds.
FAKE_MOUNT = """#!/bin/sh
# mount -t overlay NAME -o OPTIONS TARGET
echo "mount $*" >> "$FAKE/calls"
[ -e "$FAKE/mount-fails" ] && exit 1
for last; do :; done
upper=${5#*upperdir=}; upper=${upper%%,*}
cp -a "$upper/." "$last/"
echo "lumatv-store $last overlay rw,relatime 0 0" >> "$PROC_MOUNTS"
"""
FAKE_UMOUNT = """#!/bin/sh
echo "umount $*" >> "$FAKE/calls"
grep -v " $1 overlay " "$PROC_MOUNTS" > "$PROC_MOUNTS.tmp"
mv "$PROC_MOUNTS.tmp" "$PROC_MOUNTS"
rm -rf "$1/webOSUserScripts"
"""
# SAM and WAM as luna-send reaches them: luna-send -n 1 URI/METHOD PAYLOAD. $FAKE/running lists the ids SAM answers as running; a close
# removes the id unless $FAKE/stays exists; WAM's killApp removes it for good.
FAKE_LUNA = """#!/bin/sh
uri=$3; payload=$4
method=${uri##*/}
echo "$method $payload" >> "$FAKE/calls"
case "$method" in
  getAppInfo) printf '{"appInfo":{"id":"com.webos.app.discovery","folderPath":"%s"},"returnValue":true}\\n' "$(cat "$FAKE/folder")" ;;
  running) if [ -s "$FAKE/running" ]; then printf '{"running":[{"id":"%s","processid":"1"}],"returnValue":true}\\n' "$(cat "$FAKE/running")"; else echo '{"running":[],"returnValue":true}'; fi ;;
  closeByAppId) [ -e "$FAKE/stays" ] || : > "$FAKE/running"; echo '{"returnValue":true}' ;;
  killApp) : > "$FAKE/running"; echo '{"returnValue":true}' ;;
  *) echo '{"returnValue":true}' ;;
esac
"""


def tree_hash(path, skip=()):
    h = hashlib.sha256()
    for base, dirs, files in os.walk(path):
        dirs[:] = sorted(d for d in dirs if d not in skip)
        for f in sorted(files):
            p = os.path.join(base, f)
            h.update(os.path.relpath(p, path).encode())
            with open(p, "rb") as fh:
                h.update(fh.read())
    return h.hexdigest()


class ToolCase(unittest.TestCase):
    """The stand-ins: LG's directory, a fake /proc/mounts with mount and umount that edit it, a luna-send that plays SAM and WAM."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.d = self.tmp.name
        self.fake = os.path.join(d, "fake")
        self.state = os.path.join(d, "state")
        os.makedirs(self.fake)
        os.makedirs(self.state)
        # LG's directory, named as the firmware's is, with what the app has
        self.lg = os.path.join(d, "mnt", "otncabi", "usr", "palm", "applications", STORE_ID)
        os.makedirs(os.path.join(self.lg, "js"))
        with open(os.path.join(self.lg, "appinfo.json"), "w") as f:
            json.dump({"id": STORE_ID, "main": "index.html", "type": "web"}, f)
        for name, body in (("index.html", "<html></html>"), ("js/Loader.js", "var loader = 1;")):
            with open(os.path.join(self.lg, name), "w") as f:
                f.write(body)
        self.before = tree_hash(self.lg)
        with open(os.path.join(self.fake, "folder"), "w") as f:
            f.write(self.lg)
        self.proc = os.path.join(self.fake, "mounts")
        open(self.proc, "w").close()
        for name, body in (("mount", FAKE_MOUNT), ("umount", FAKE_UMOUNT), ("luna-send", FAKE_LUNA)):
            p = os.path.join(self.fake, name)
            with open(p, "w") as f:
                f.write(body)
            os.chmod(p, 0o755)
        self.env = dict(os.environ, APPDIR=ROOT, STATE=self.state, FAKE=self.fake, PROC_MOUNTS=self.proc, MOUNT=os.path.join(self.fake, "mount"),
                        UMOUNT=os.path.join(self.fake, "umount"), LUNA_SEND=os.path.join(self.fake, "luna-send"), STORE_STEP="0", THEME_DIR=os.path.join(self.state, "theme"))

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, *args, env=None):
        r = subprocess.run(["sh", SCRIPT, *args], capture_output=True, text=True, timeout=60, env=dict(self.env, **(env or {})), stdin=subprocess.DEVNULL)
        out = r.stdout.strip()
        try:
            return r.returncode, json.loads(out)
        except ValueError:
            self.fail("not one JSON object: %r (stderr %r)" % (out, r.stderr))

    def calls(self):
        p = os.path.join(self.fake, "calls")
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return [l.strip() for l in f if l.strip()]

    def running(self, on):
        with open(os.path.join(self.fake, "running"), "w") as f:
            f.write(STORE_ID if on else "")

    def mounted(self):
        with open(self.proc) as f:
            return " %s overlay " % self.lg in f.read()

    def upper_files(self):
        base = os.path.join(self.state, "store", "upper")
        return sorted(os.path.relpath(os.path.join(b, f), base) for b, _, fs in os.walk(base) for f in fs)


class StoreTool(ToolCase):
    # ---- staging

    def test_stage_builds_the_one_file_and_mounts_nothing(self):
        rc, out = self.run_tool("stage")
        self.assertEqual((rc, out["ok"], out["staged"], out["parts"]), (0, True, True, "look,motion"))
        self.assertEqual(self.upper_files(), ["webOSUserScripts/userScript.js"])
        self.assertFalse(any(c.startswith("mount ") for c in self.calls()))
        self.assertEqual(tree_hash(self.lg), self.before)

    def test_the_script_is_readable_by_wam_and_holds_the_theme(self):
        self.run_tool("stage")
        path = os.path.join(self.state, "store", "upper", "webOSUserScripts", "userScript.js")
        self.assertEqual(os.stat(path).st_mode & 0o777, 0o644)
        with open(path) as f:
            text = f.read()
        self.assertIn("d09GMg", text, "the face (a woff2 file, base64) is in the script")
        self.assertNotIn("@@", text, "a marker the builder fills was left in the script")

    def test_parts_are_kept_in_canonical_order_and_unknown_ones_are_refused(self):
        rc, out = self.run_tool("stage", "lazy, canvas look")
        self.assertEqual((rc, out["parts"]), (0, "look,canvas,lazy"))
        rc, out = self.run_tool("stage", "look sparkle")
        self.assertEqual(rc, 1)
        self.assertFalse(out["ok"])
        self.assertIn("sparkle", out["error"])
        # the failed request left the last good stage as it was
        self.assertEqual(self.upper_files(), ["webOSUserScripts/userScript.js"])

    def test_the_probe_part_is_for_the_probe_verb_alone(self):
        for verb in ("stage", "on"):
            rc, out = self.run_tool(verb, "look,probe")
            self.assertEqual(rc, 1, verb)
            self.assertIn("probe", out["error"])
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(os.path.join(self.state, "store", "upper")))

    # ---- on

    def test_on_mounts_the_overlay_over_lgs_directory_with_it_as_the_lower_layer(self):
        rc, out = self.run_tool("on")
        self.assertEqual((rc, out["ok"], out["store"], out["dir"]), (0, True, "on", self.lg))
        mounts = [c for c in self.calls() if c.startswith("mount ")]
        self.assertEqual(len(mounts), 1)
        upper = os.path.join(self.state, "store", "upper")
        self.assertIn("-t overlay lumatv-store -o lowerdir=%s,upperdir=%s,workdir=%s %s" % (self.lg, upper, os.path.join(self.state, "store", "work"), self.lg), mounts[0])
        self.assertTrue(self.mounted())
        self.assertEqual(tree_hash(self.lg, skip=("webOSUserScripts",)), self.before, "LG's own files are never touched, the theme is only mounted over them")

    def test_a_second_on_replaces_the_mount_instead_of_stacking_one(self):
        self.run_tool("on")
        self.run_tool("on", "look")
        with open(self.proc) as f:
            self.assertEqual(f.read().count(" %s overlay " % self.lg), 1)
        self.assertEqual(self.calls().count("umount %s" % self.lg), 1)

    def test_on_ends_a_running_store_so_the_next_launch_loads_the_theme(self):
        self.running(True)
        rc, out = self.run_tool("on")
        self.assertTrue(out["closed"])
        calls = self.calls()
        self.assertTrue(any(c.startswith("closeByAppId") and STORE_ID in c for c in calls))
        self.assertFalse(any(c.startswith("killApp") for c in calls), "the close ended it; WAM is not asked")
        # the close comes after the mount: the instance that is ended must be replaced by one that reads what is mounted
        self.assertLess([i for i, c in enumerate(calls) if c.startswith("mount ")][0], [i for i, c in enumerate(calls) if c.startswith("closeByAppId")][0])

    def test_a_store_that_is_not_running_is_left_alone(self):
        rc, out = self.run_tool("on")
        self.assertFalse(out["closed"])
        self.assertFalse(any(c.startswith(("closeByAppId", "killApp")) for c in self.calls()))

    def test_wam_is_asked_only_when_the_close_left_the_page_listed(self):
        self.running(True)
        open(os.path.join(self.fake, "stays"), "w").close()
        rc, out = self.run_tool("on")
        self.assertTrue(any(c.startswith("killApp") and STORE_ID in c for c in self.calls()))

    def test_a_failed_build_leaves_lgs_store_in_place(self):
        bad = os.path.join(self.d, "bad-build.py")
        with open(bad, "w") as f:
            f.write('import sys\nif sys.argv[1] == "parts":\n    print("look,motion")\n    sys.exit(0)\nprint(\'{"ok": false, "error": "no font"}\')\nsys.exit(1)\n')
        rc, out = self.run_tool("on", env={"STORE_BUILD_TOOL": bad})
        self.assertEqual(rc, 1)
        self.assertFalse(out["ok"])
        self.assertIn("LG's store is in place", out["error"])
        self.assertFalse(self.mounted())
        self.assertFalse(any(c.startswith("mount ") for c in self.calls()))
        self.assertFalse(os.path.exists(os.path.join(self.state, "store", "upper.new")))

    def test_a_failed_mount_is_undone_and_leaves_lgs_store_in_place(self):
        open(os.path.join(self.fake, "mount-fails"), "w").close()
        self.running(True)
        rc, out = self.run_tool("on")
        self.assertEqual(rc, 1)
        self.assertFalse(out["ok"])
        self.assertFalse(self.mounted())
        self.assertFalse(any(c.startswith("closeByAppId") for c in self.calls()), "nothing changed, so nothing is closed")

    def test_the_directory_sam_names_is_taken_only_when_it_is_the_stores(self):
        elsewhere = os.path.join(self.d, "etc")
        os.makedirs(elsewhere)
        with open(os.path.join(elsewhere, "appinfo.json"), "w") as f:
            json.dump({"id": STORE_ID}, f)
        with open(os.path.join(self.fake, "folder"), "w") as f:
            f.write(elsewhere)
        # SAM's answer is refused (not a .../usr/palm/applications/<id> directory) and the known homes do not exist here
        rc, out = self.run_tool("on", env={"STORE_CANDIDATES": os.path.join(self.d, "none")})
        self.assertEqual(rc, 1)
        self.assertIn("was not found", out["error"])
        self.assertFalse(self.mounted())

    def test_the_directory_must_name_the_store_in_its_appinfo(self):
        with open(os.path.join(self.lg, "appinfo.json"), "w") as f:
            json.dump({"id": "com.webos.app.something"}, f)
        rc, out = self.run_tool("on", env={"STORE_CANDIDATES": os.path.join(self.d, "none")})
        self.assertEqual(rc, 1)

    def test_without_sam_the_known_homes_are_tried(self):
        with open(os.path.join(self.fake, "folder"), "w") as f:
            f.write("")
        rc, out = self.run_tool("on", env={"STORE_CANDIDATES": self.lg})
        self.assertEqual((rc, out["dir"]), (0, self.lg))

    # ---- off and status

    def test_off_gives_lgs_directory_back_exactly(self):
        self.run_tool("on")
        self.running(True)
        rc, out = self.run_tool("off")
        self.assertEqual((rc, out["ok"], out["store"], out["wasMounted"], out["closed"]), (0, True, "off", True, True))
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(os.path.join(self.state, "store")), "nothing of the stage stays")
        self.assertEqual(tree_hash(self.lg), self.before)
        self.assertFalse(os.path.exists(os.path.join(self.lg, "webOSUserScripts")))

    def test_off_on_a_set_that_never_had_the_theme_closes_nothing(self):
        self.running(True)
        rc, out = self.run_tool("off")
        self.assertEqual((rc, out["wasMounted"], out["closed"]), (0, False, False))
        self.assertFalse(any(c.startswith(("closeByAppId", "killApp")) for c in self.calls()))

    def test_status_reports_where_what_and_whether(self):
        rc, out = self.run_tool("status")
        self.assertEqual((out["dir"], out["staged"], out["mounted"], out["parts"], out["running"]), (self.lg, False, False, "", False))
        self.assertEqual((out["probe"]["running"], out["probe"]["port"], out["probe"]["files"]), (False, 0, 0))
        self.run_tool("on", "look")
        self.running(True)
        rc, out = self.run_tool("status")
        self.assertEqual((out["staged"], out["mounted"], out["parts"], out["running"]), (True, True, "look", True))

    def test_an_unknown_verb_is_a_usage_error(self):
        rc, out = self.run_tool("explode")
        self.assertEqual(rc, 1)
        self.assertIn("usage", out["error"])

    def test_the_tool_never_restarts_sam_or_wam(self):
        with open(SCRIPT) as f:
            code = [l for l in f if not l.lstrip().startswith("#")]
        text = "".join(code)
        self.assertNotIn("systemctl", text)
        self.assertNotIn("pkill", text)
        self.assertNotIn("killall", text)


def post(port, body, path="/p", method="POST"):
    req = urllib.request.Request("http://127.0.0.1:%d%s" % (port, path), data=body, method=method, headers={"Content-Type": "text/plain"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return r.status, r.read()


class Probe(ToolCase):
    """The probe verb, on the same stand-ins and the real receiver."""

    def setUp(self):
        super().setUp()
        self.env.update(STORE_PROBE_PORT="0", STORE_PROBE_SECONDS="60", PROBE_STEP="0.05")
        self.pidfile = os.path.join(self.state, ".store-probe.pid")
        self.portfile = os.path.join(self.state, ".store-probe.port")

    def tearDown(self):
        try:
            with open(self.pidfile) as f:
                os.kill(int(f.read().strip()), signal.SIGKILL)
        except (OSError, ValueError):
            pass
        super().tearDown()

    def script(self):
        with open(os.path.join(self.state, "store", "upper", "webOSUserScripts", "userScript.js")) as f:
            return f.read()

    def alive(self, pid):
        try:
            os.kill(pid, 0)
        except OSError:
            return False
        try:
            with open("/proc/%d/stat" % pid) as f:
                return f.read().split(") ")[-1][0] != "Z"
        except OSError:
            return False

    def test_probe_starts_the_receiver_first_and_mounts_the_script_that_posts_to_it(self):
        self.running(True)
        rc, out = self.run_tool("probe")
        self.assertEqual((rc, out["ok"], out["store"], out["parts"], out["closed"]), (0, True, "probe", "probe", True))
        port = out["probe"]["port"]
        self.assertTrue(out["probe"]["running"] and port > 0)
        self.assertIn('"url": "http://127.0.0.1:%d/p"' % port, self.script())
        self.assertTrue(self.mounted())
        self.assertEqual(tree_hash(self.lg, skip=("webOSUserScripts",)), self.before, "only the mount is laid over LG's files")
        self.assertEqual(post(port, b"", "/ping", "GET")[1], b"ok", "the receiver answers where the script says it is")
        # the receiver was up before the page could post, and the store was ended after the mount, as for `on`
        calls = self.calls()
        self.assertLess([i for i, c in enumerate(calls) if c.startswith("mount ")][0], [i for i, c in enumerate(calls) if c.startswith("closeByAppId")][0])

    def test_the_receiver_does_not_keep_the_callers_lock_descriptors(self):
        # lumatv takes its mutation lock on descriptor 9 and the children it starts inherit it: a receiver that kept it would hold every
        # apply, revert and `store off` out for the half hour it runs
        lock = os.path.join(self.d, "lock")
        r = subprocess.run(["sh", "-c", 'exec 9>"$LOCK"; flock -x 9; sh "$SCRIPT" probe'], capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL,
                           env=dict(self.env, LOCK=lock, SCRIPT=SCRIPT))
        self.assertEqual(json.loads(r.stdout)["ok"], True, r.stdout)
        self.assertEqual(subprocess.run(["flock", "-n", lock, "true"]).returncode, 0, "the lock was released with the caller")
        with open(self.pidfile) as f:
            self.assertTrue(self.alive(int(f.read())), "and the receiver is still running")

    def test_the_stock_page_is_what_a_bare_probe_shows_and_parts_add_the_theme(self):
        rc, out = self.run_tool("probe")
        self.assertEqual(out["parts"], "probe")
        self.assertNotIn("@font-face", self.script(), "no look: LG's page as it is")
        rc, out = self.run_tool("probe", "look")
        self.assertEqual((rc, out["parts"]), (0, "look,probe"))
        self.assertIn("font-face", self.script())

    def test_the_receiver_listens_on_loopback_only(self):
        rc, out = self.run_tool("probe")
        port = out["probe"]["port"]
        with open("/proc/net/tcp") as f:
            rows = [l.split() for l in f.readlines()[1:]]
        mine = [r for r in rows if r[1].endswith(":%04X" % port) and r[3] == "0A"]
        self.assertEqual([r[1].split(":")[0] for r in mine], ["0100007F"], "127.0.0.1 and nothing wider")

    def test_status_shows_the_receiver_and_what_it_has_written(self):
        rc, out = self.run_tool("probe")
        port = out["probe"]["port"]
        post(port, json.dumps({"kind": "hello", "data": {}}).encode())
        rc, out = self.run_tool("status")
        self.assertEqual((out["probe"]["running"], out["probe"]["port"], out["probe"]["files"], out["parts"]), (True, port, 1, "probe"))

    def test_off_ends_the_receiver_and_keeps_what_it_wrote(self):
        rc, out = self.run_tool("probe")
        port = out["probe"]["port"]
        post(port, json.dumps({"kind": "dom", "data": {}}).encode())
        with open(self.pidfile) as f:
            pid = int(f.read())
        rc, out = self.run_tool("off")
        self.assertEqual((rc, out["wasMounted"]), (0, True))
        deadline = time.time() + 5
        while self.alive(pid) and time.time() < deadline:
            time.sleep(0.05)
        self.assertFalse(self.alive(pid))
        self.assertFalse(os.path.exists(self.pidfile))
        self.assertTrue(os.path.exists(os.path.join(self.state, "store-probe", "001-dom.json")), "the round's files are for reading afterwards")
        with self.assertRaises(OSError):
            post(port, b"{}")
        self.assertEqual(tree_hash(self.lg), self.before)

    def test_on_ends_a_receiver_left_by_an_earlier_probe_and_the_script_then_has_no_probe(self):
        rc, out = self.run_tool("probe")
        with open(self.pidfile) as f:
            pid = int(f.read())
        rc, out = self.run_tool("on", "look")
        self.assertEqual((rc, out["parts"]), (0, "look"))
        deadline = time.time() + 5
        while self.alive(pid) and time.time() < deadline:
            time.sleep(0.05)
        self.assertFalse(self.alive(pid))
        self.assertNotIn("127.0.0.1", self.script())

    def test_a_new_probe_replaces_the_receiver_and_keeps_the_last_files(self):
        rc, first = self.run_tool("probe")
        post(first["probe"]["port"], json.dumps({"kind": "env", "data": {}}).encode())
        with open(self.pidfile) as f:
            pid = int(f.read())
        rc, second = self.run_tool("probe", "look")
        deadline = time.time() + 5
        while self.alive(pid) and time.time() < deadline:
            time.sleep(0.05)
        self.assertFalse(self.alive(pid), "one receiver at a time")
        self.assertEqual(second["probe"]["files"], 0, "the new session starts empty")
        self.assertTrue(os.path.exists(os.path.join(self.state, "store-probe.prev", "001-env.json")))
        with open("/proc/net/tcp") as f:
            listening = [r.split()[1] for r in f.readlines()[1:] if r.split()[3] == "0A"]
        self.assertNotIn("0100007F:%04X" % first["probe"]["port"], listening)
        self.assertIn("0100007F:%04X" % second["probe"]["port"], listening)

    def test_a_receiver_that_cannot_start_leaves_lgs_store_in_place(self):
        busy = socket.socket()
        busy.bind(("127.0.0.1", 0))
        busy.listen(1)
        try:
            self.running(True)
            rc, out = self.run_tool("probe", env={"STORE_PROBE_PORT": str(busy.getsockname()[1])})
        finally:
            busy.close()
        self.assertEqual(rc, 1)
        self.assertFalse(out["ok"])
        self.assertIn("receiver", out["error"])
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(os.path.join(self.state, "store", "upper")))
        self.assertFalse(any(c.startswith(("mount ", "closeByAppId")) for c in self.calls()), "nothing was changed, so nothing is closed")

    def test_a_failed_build_stops_the_receiver_it_started(self):
        bad = os.path.join(self.d, "bad-build.py")
        with open(bad, "w") as f:
            f.write('import sys\nif sys.argv[1] == "parts":\n    print("probe")\n    sys.exit(0)\nprint(\'{"ok": false}\')\nsys.exit(1)\n')
        rc, out = self.run_tool("probe", env={"STORE_BUILD_TOOL": bad})
        self.assertEqual(rc, 1)
        self.assertFalse(os.path.exists(self.pidfile), "no receiver is left running for a page that will never post")
        self.assertFalse(self.mounted())

    def test_a_failed_mount_stops_the_receiver_and_leaves_lgs_store_in_place(self):
        open(os.path.join(self.fake, "mount-fails"), "w").close()
        rc, out = self.run_tool("probe")
        self.assertEqual(rc, 1)
        self.assertFalse(os.path.exists(self.pidfile))
        self.assertFalse(self.mounted())

    def test_an_unknown_part_is_refused_before_anything_starts(self):
        rc, out = self.run_tool("probe", "sparkle")
        self.assertEqual(rc, 1)
        self.assertIn("sparkle", out["error"])
        self.assertFalse(os.path.exists(self.pidfile))


FAKE_STORE_TOOL = """#!/bin/sh
echo "$*" >> "$FAKE/tool-calls"
case "$1" in
  on) if [ -e "$FAKE/on-fails" ]; then echo '{"ok":false,"error":"could not mount the theme; LG'"'"'s store is in place"}'; exit 1; fi
      [ -e "$FAKE/silent" ] && exit 0
      p="${2:-look,motion}"; [ -n "$p" ] || p=look,motion
      echo '{"ok":true,"store":"on","dir":"/x","parts":"'$p'","closed":false,"build":{"parts": ["look"], "ok": true}}' ;;
  probe) if [ -e "$FAKE/on-fails" ]; then echo '{"ok":false,"error":"could not start the probe'"'"'s receiver; LG'"'"'s store is in place"}'; exit 1; fi
         [ -e "$FAKE/silent" ] && exit 0
         echo '{"ok":true,"store":"probe","dir":"/x","parts":"'"${2:+$2,}probe"'","closed":false,"probe":{"running":true,"port":18742,"files":0,"dir":"/s"}}' ;;
  off) if [ -e "$FAKE/off-fails" ]; then echo '{"ok":false,"error":"could not unmount the overlay"}'; exit 1; fi
       echo '{"ok":true,"store":"off","wasMounted":true,"closed":false}' ;;
  status) [ -e "$FAKE/silent" ] && exit 0
          m=false; [ -e "$FAKE/mounted" ] && m=true
          echo '{"ok":true,"dir":"/x","staged":true,"mounted":'$m',"parts":"look","running":false}' ;;
esac
"""


def function(name):
    out, on = [], False
    with open(TOOL) as f:
        for line in f:
            if line.startswith(name + "() {"):
                on = True
            if on:
                out.append(line)
                if line.rstrip() == "}":
                    return "".join(out)
    raise AssertionError(name + " not found")


class Verb(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.tool = os.path.join(self.d, "store.sh")
        with open(self.tool, "w") as f:
            f.write(FAKE_STORE_TOOL)
        self.flag = os.path.join(self.d, ".store-on")

    def tearDown(self):
        self.tmp.cleanup()

    def run_verb(self, *args, lock=True):
        quoted = " ".join('"%s"' % a for a in args)
        script = (f'FAKE="{self.d}"; export FAKE\nSTORE_TOOL="{self.tool}"\nSTORE_ON="{self.flag}"\nLOGFILE="{self.d}/log"\n'
                  f'take_mutation_lock() {{ {"return 0" if lock else "return 1"}; }}\n'
                  f'log_line() {{ echo "$1" >> "{self.d}/loglines"; }}\n'
                  + "".join(function(n) for n in ("json_escape", "emit_error", "store_mounted", "store_set", "store_parts_on", "verb_store"))
                  + f"verb_store {quoted}\n")
        r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
        return r.returncode, r.stdout.strip()

    def calls(self):
        p = os.path.join(self.d, "tool-calls")
        if not os.path.exists(p):
            return []
        with open(p) as f:
            return [l.strip() for l in f if l.strip()]

    def test_on_keeps_the_flag_with_the_parts_the_tool_settled(self):
        rc, out = self.run_verb("on", "")
        self.assertEqual(rc, 0)
        self.assertTrue(json.loads(out)["ok"])
        with open(self.flag) as f:
            self.assertEqual(f.read().strip(), "look,motion")
        rc, out = self.run_verb("on", "look")
        with open(self.flag) as f:
            self.assertEqual(f.read().strip(), "look")
        self.assertEqual(self.calls(), ["on", "on look"])

    def test_a_refused_on_leaves_no_flag_and_passes_the_tools_reason_on(self):
        open(os.path.join(self.d, "on-fails"), "w").close()
        rc, out = self.run_verb("on", "")
        self.assertEqual(rc, 1)
        self.assertIn("LG's store is in place", json.loads(out)["error"])
        self.assertFalse(os.path.exists(self.flag))

    def test_off_removes_the_flag_and_a_refused_off_keeps_it(self):
        rc, _ = self.run_verb("on", "")
        open(os.path.join(self.d, "off-fails"), "w").close()
        rc, out = self.run_verb("off")
        self.assertEqual(rc, 1)
        self.assertTrue(os.path.exists(self.flag))
        os.remove(os.path.join(self.d, "off-fails"))
        rc, out = self.run_verb("off")
        self.assertEqual(rc, 0)
        self.assertFalse(os.path.exists(self.flag))

    def test_status_merges_the_flag_into_the_tools_report(self):
        rc, out = self.run_verb("status")
        self.assertEqual((json.loads(out)["on"], json.loads(out)["parts"]), (False, "look"))
        open(self.flag, "w").close()
        self.assertTrue(json.loads(self.run_verb("status")[1])["on"])

    def test_a_tool_that_does_not_answer_is_an_error_not_a_success(self):
        open(os.path.join(self.d, "silent"), "w").close()
        for args in (("on", ""), ("status",)):
            rc, out = self.run_verb(*args)
            self.assertEqual(rc, 1)
            self.assertIn("did not answer", json.loads(out)["error"])

    def test_another_mutation_in_progress_refuses_on_and_off(self):
        for args in (("on", ""), ("off",)):
            rc, out = self.run_verb(*args, lock=False)
            self.assertEqual(rc, 1)
            self.assertIn("another apply or revert", json.loads(out)["error"])
        self.assertEqual(self.calls(), [])

    def test_probe_asks_the_tool_and_never_writes_the_flag(self):
        rc, out = self.run_verb("probe", "")
        self.assertEqual((rc, json.loads(out)["store"], json.loads(out)["parts"]), (0, "probe", "probe"))
        rc, out = self.run_verb("probe", "look")
        self.assertEqual(json.loads(out)["parts"], "look,probe")
        self.assertFalse(os.path.exists(self.flag), "a reboot returns to what the flag says: LG's store unless `store on` was run")
        self.assertEqual(self.calls(), ["probe", "probe look"])

    def test_probe_leaves_the_flag_of_a_store_that_is_on_as_it_was(self):
        self.run_verb("on", "look")
        self.run_verb("probe", "")
        with open(self.flag) as f:
            self.assertEqual(f.read().strip(), "look")

    def test_a_refused_probe_passes_the_tools_reason_on_and_takes_the_lock_like_on(self):
        open(os.path.join(self.d, "on-fails"), "w").close()
        rc, out = self.run_verb("probe", "")
        self.assertEqual(rc, 1)
        self.assertIn("LG's store is in place", json.loads(out)["error"])
        rc, out = self.run_verb("probe", "", lock=False)
        self.assertIn("another apply or revert", json.loads(out)["error"])

    def test_usage(self):
        rc, out = self.run_verb("sideways")
        self.assertEqual(rc, 1)
        self.assertIn("usage", json.loads(out)["error"])
        self.assertIn("probe", json.loads(out)["error"])

    def test_the_store_is_part_of_apply_and_revert_and_off_by_default(self):
        with open(TOOL) as f:
            text = f.read()
        # apply asserts the mount again while the flag is there; revert takes it away; a fresh set has no flag
        self.assertIn('[ -f "$STORE_ON" ] && { store_mounted || store_set on "$(store_parts_on)"', text)
        self.assertIn('[ -f "$STORE_ON" ] && { store_set off; rm -f "$STORE_ON"; }', text)
        self.assertNotIn(': > "$STORE_ON"', text)


if __name__ == "__main__":
    unittest.main()
