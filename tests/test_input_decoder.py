"""Run with: python3 -m unittest tests.test_input_decoder

An input app (HDMI, Live TV) taken down with the compositor while its sound is started leaves the kernel's audio decoder
port open (/proc/lgtv-driver/audio/adec.p1: Open=1, Connect=HDMI_PORT2) with nobody to close it, and Netflix's sound then
fails (tvq-pb-101). The tool ends the input apps through SAM before it restarts the compositor, and `lumatv inputs
release` frees a port left that way. Driven here against a fake luna-send and a fake adec directory.
"""
import json
import os
import re
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(os.path.dirname(HERE), "tools", "lumatv")

FUNCS = ("foreground_app_id", "input_apps_running", "input_decoder_held", "close_input_apps", "warn_input_decoder_held",
         "inputs_json", "verb_inputs", "restart_surface", "home_back_after_restart")


def source():
    with open(TOOL, encoding="utf-8") as f:
        return f.read()


def function(name):
    """One shell function of tools/lumatv, from its `name() {` line to the closing brace in column 0."""
    m = re.search(r"^%s\(\) \{\n.*?^\}\n" % re.escape(name), source(), re.S | re.M)
    if not m:
        raise AssertionError(name + " not found")
    return m.group(0)


def input_app_ids():
    m = re.search(r'^INPUT_APP_IDS="([^"]*)"$', source(), re.M)
    if not m:
        raise AssertionError("INPUT_APP_IDS not found")
    return m.group(0)


# SAM as far as these functions use it: `running` lists Home and the ids in $d/running; closeByAppId drops the id and, when
# $d/release exists, closes the held port the way the app's own teardown does; launch puts the app in front (an input app
# also becomes running); the foreground is $d/fg.
LUNA = r'''#!/bin/sh
d="%s"
case "$3" in
  */running)
    ids=$(sed 's/.*/{"id":"&"},/' "$d/running" 2>/dev/null | tr -d '\n')
    echo "{\"returnValue\":true,\"running\":[${ids}{\"id\":\"com.webos.app.home\"}]}" ;;
  */closeByAppId)
    id=$(echo "$4" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
    echo "close $id" >> "$d/calls"
    grep -vx "$id" "$d/running" > "$d/running.new" 2>/dev/null; mv "$d/running.new" "$d/running"
    [ -e "$d/release" ] && printf 'AdecPortNum=1\nOpen=0\nConnect=UNKNOWN\nStart=0\n' > "$d/adec/adec.p1"
    echo '{"returnValue":true}' ;;
  */getForegroundAppInfo) echo "{\"returnValue\":true,\"appId\":\"$(cat "$d/fg")\"}" ;;
  */launch)
    id=$(echo "$4" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
    echo "launch $id" >> "$d/calls"
    echo "$id" > "$d/fg"
    case "$id" in com.webos.app.hdmi*|com.webos.app.livetv) echo "$id" >> "$d/running" ;; esac
    echo '{"returnValue":true}' ;;
esac
'''


class Base(unittest.TestCase):
    def run_sh(self, body, running=(), port1=None, fg="com.webos.app.home", release=True):
        """port1: None for a closed port, else the Connect value of an open one."""
        with tempfile.TemporaryDirectory() as d:
            os.mkdir(os.path.join(d, "bin"))
            os.mkdir(os.path.join(d, "adec"))
            with open(os.path.join(d, "bin", "luna-send"), "w") as f:
                f.write(LUNA % d)
            with open(os.path.join(d, "bin", "systemctl"), "w") as f:
                f.write('#!/bin/sh\necho "systemctl $*" >> "%s/calls"\n' % d)
            for name in ("luna-send", "systemctl"):
                os.chmod(os.path.join(d, "bin", name), 0o755)
            with open(os.path.join(d, "adec", "adec.p0"), "w") as f:
                f.write("AdecPortNum=0\nOpen=0\nConnect=UNKNOWN\nStart=0\n")
            with open(os.path.join(d, "adec", "adec.p1"), "w") as f:
                f.write("AdecPortNum=1\nOpen=1\nConnect=%s\nStart=1\n" % port1 if port1 else "AdecPortNum=1\nOpen=0\nConnect=UNKNOWN\nStart=0\n")
            with open(os.path.join(d, "running"), "w") as f:
                f.write("".join(a + "\n" for a in running))
            with open(os.path.join(d, "fg"), "w") as f:
                f.write(fg + "\n")
            if release:
                open(os.path.join(d, "release"), "w").close()
            script = (
                'set -u\nd="%s"\nPATH="$d/bin:$PATH"\nADEC_PROC=$d/adec\nSHORT_ID=lumatv\n%s\n'
                'HOME_LAUNCH_URI=luna://com.webos.applicationManager/launch; HOME_APP_ID=com.webos.app.home\n'
                'SURFACE_SVC=surface-manager-daemon.service; reports_before=\n'
                'log_line() { echo "$1" >> "$d/log"; }\nemit_error() { printf \'{"ok":false,"error":"%%s"}\\n\' "$1"; }\n'
                'sleep() { :; }\ncomp_recovering() { return 1; }\nwam_stamp() { echo 1; }\nsnapshot_reports() { :; }\n'
                'wait_surface_active() { return 0; }\ndisarm_restart_crashes() { echo disarm >> "$d/calls"; }\n'
                'kill_home_app() { :; }\nrelaunch_home() { echo home >> "$d/calls"; }\n'
            ) % (d, input_app_ids())
            script += "".join(function(n) for n in FUNCS) + body
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)

            def read(name):
                p = os.path.join(d, name)
                if not os.path.exists(p):
                    return []
                with open(p) as fh:
                    return fh.read().split("\n")[:-1]
            return r, read("calls"), read("log")


class BeforeTheRestart(Base):
    def test_a_running_input_is_ended_through_sam_before_the_compositor_goes_down(self):
        r, calls, log = self.run_sh("restart_surface; echo rc=$?\n", running=["com.webos.app.hdmi2"], port1="HDMI_PORT2")
        self.assertIn("rc=0", r.stdout, r.stderr)
        self.assertEqual(calls[:2], ["close com.webos.app.hdmi2", "systemctl restart surface-manager-daemon.service"])
        self.assertTrue(any("restart: input app(s) ended through SAM: com.webos.app.hdmi2 (decoder port closed after 0 s)" in l
                            for l in log), log)

    def test_with_no_input_running_the_restart_asks_sam_nothing_else(self):
        r, calls, log = self.run_sh("restart_surface\n")
        self.assertEqual(calls, ["systemctl restart surface-manager-daemon.service", "disarm"], r.stderr)
        self.assertEqual(log, [])

    def test_a_port_that_does_not_close_is_said_and_the_restart_goes_ahead(self):
        r, calls, log = self.run_sh("restart_surface; echo rc=$?\n", running=["com.webos.app.hdmi2"], port1="HDMI_PORT2",
                                    release=False)
        self.assertIn("rc=0", r.stdout, r.stderr)
        self.assertIn("systemctl restart surface-manager-daemon.service", calls)
        self.assertTrue(any("HDMI_PORT2 port still open after 10 s" in l for l in log), log)

    def test_only_input_apps_are_ended(self):
        r, calls, log = self.run_sh("close_input_apps test\n", running=["netflix", "com.webos.app.livetv", "com.webos.app.hdmi4"])
        self.assertEqual(sorted(c for c in calls if c.startswith("close")),
                         ["close com.webos.app.hdmi4", "close com.webos.app.livetv"], r.stderr)

    def test_a_port_left_open_with_no_input_running_is_logged_after_home_is_back(self):
        r, calls, log = self.run_sh("home_back_after_restart\n", port1="HDMI_PORT2")
        self.assertTrue(any("still open for HDMI_PORT2 with no input app running" in l and "lumatv inputs release" in l
                            for l in log), log)

    def test_an_open_port_with_its_input_running_is_not_a_leak(self):
        r, calls, log = self.run_sh("home_back_after_restart\n", running=["com.webos.app.hdmi2"], port1="HDMI_PORT2")
        self.assertEqual(log, [], r.stderr)


class InputsVerb(Base):
    def test_status_lists_the_ports_the_running_inputs_and_the_held_one(self):
        r, calls, log = self.run_sh("verb_inputs status\n", port1="HDMI_PORT2")
        out = json.loads(r.stdout)
        self.assertEqual(out["inputs"]["ports"], [{"port": 0, "open": False, "connect": "UNKNOWN"},
                                                  {"port": 1, "open": True, "connect": "HDMI_PORT2"}])
        self.assertEqual(out["inputs"]["running"], [])
        self.assertEqual(out["inputs"]["held"], "HDMI_PORT2")

    def test_release_launches_the_ports_input_closes_it_and_puts_the_front_app_back(self):
        r, calls, log = self.run_sh("verb_inputs release; echo rc=$?\n", port1="HDMI_PORT2", fg="com.webos.app.home")
        self.assertIn('"released":true', r.stdout, r.stderr)
        self.assertIn('"app":"com.webos.app.hdmi2"', r.stdout)
        self.assertIn("rc=0", r.stdout)
        self.assertEqual(calls, ["launch com.webos.app.hdmi2", "close com.webos.app.hdmi2", "launch com.webos.app.home"])
        self.assertTrue(any("inputs release: com.webos.app.hdmi2 launched and closed, the port is free" in l for l in log), log)

    def test_release_with_nothing_held_does_nothing(self):
        r, calls, log = self.run_sh("verb_inputs release\n")
        self.assertIn('"released":false', r.stdout, r.stderr)
        self.assertIn("no port held", r.stdout)
        self.assertEqual(calls, [])

    def test_release_leaves_a_port_its_running_input_owns(self):
        r, calls, log = self.run_sh("verb_inputs release\n", running=["com.webos.app.hdmi2"], port1="HDMI_PORT2")
        self.assertIn("owns it", r.stdout, r.stderr)
        self.assertEqual(calls, [])

    def test_a_release_that_does_not_free_the_port_fails(self):
        r, calls, log = self.run_sh("verb_inputs release; echo rc=$?\n", port1="HDMI_PORT1", release=False)
        self.assertIn('"ok":false', r.stdout, r.stderr)
        self.assertIn("rc=1", r.stdout)
        self.assertEqual(calls[0], "launch com.webos.app.hdmi1")


if __name__ == "__main__":
    unittest.main()
