"""Run with: python3 -m unittest tests.test_capture_install   (the end-to-end test needs `ambient/capture/build.sh`)

payload/capture/capture.sh, what `tools/lumatv` runs for the ambient light's colour reader at apply and revert: the service is
staged at a path of its own, its LS2 role is written for exactly that path, the hub is asked to rescan only when the role changed,
and remove takes both away. Run against a temporary state directory and role directory, with a stand-in `ls-control` that records
what it was asked. Where ls-hubd looks for the role, the role's `type`, and that LG's library registers with exactly these names
are the TV's to say (docs/research/plugins-slice-result.md).
"""
import json
import os
import shutil
import socket
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
SH = os.path.join(REPO, "payload", "capture", "capture.sh")
ROLE = os.path.join(REPO, "payload", "capture", "role.json")
BUILD = os.path.join(REPO, "ambient", "capture", "build", "host")
HOST_BIN = os.path.join(BUILD, "lumatv-capture")

# What LG's library registers and calls (the strings of libvtcapture.so.1.0.0, docs/research/ambient-result.md), and the bus's own service.
CLIENT_NAMES = {"com.webos.service.capture.client", "com.webos.service.capture.client.*"}
MAY_CALL = {"com.webos.service.capture", "com.webos.service.capturepermission", "com.palm.bus"}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="capture-install-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = os.path.join(self.tmp, "state")
        self.ls2 = os.path.join(self.tmp, "ls2")
        self.bin = os.path.join(self.tmp, "bin")
        self.calls = os.path.join(self.tmp, "ls-control.calls")
        os.mkdir(self.bin)
        with open(os.path.join(self.bin, "ls-control"), "w") as f:
            f.write('#!/bin/sh\necho "$@" >> %s\n' % self.calls)
        os.chmod(os.path.join(self.bin, "ls-control"), 0o755)
        self.package = os.path.join(self.tmp, "lumatv-capture")
        with open(self.package, "w") as f:
            f.write("#!/bin/sh\nexec sleep 30\n")        # stands in for the 32-bit binary where the host build is not needed
        os.chmod(self.package, 0o755)
        self.env = dict(os.environ, LUMATV_LS2_DIR=self.ls2, PATH=self.bin + os.pathsep + os.environ["PATH"], LD_LIBRARY_PATH=BUILD)
        self.addCleanup(self.run_sh, "stop", self.state)

    def run_sh(self, *args, binary=None):
        return subprocess.run(["sh", SH, *args], env=self.env, capture_output=True, text=True, timeout=30)

    def install(self, package=None):
        return self.run_sh("install", package or self.package, ROLE, self.state)

    def ls_calls(self):
        try:
            with open(self.calls) as f:
                return f.read().split("\n")[:-1]
        except FileNotFoundError:
            return []

    def role(self):
        with open(os.path.join(self.ls2, "roles.d", "lumatv-capture.json")) as f:
            return json.load(f)



class Install(Base):
    def test_the_role_is_written_for_the_staged_path_and_allows_only_the_capture_clients_calls(self):
        r = self.install()
        self.assertEqual(r.returncode, 0, r.stderr)
        exe = os.path.join(self.state, "bin", "lumatv-capture")
        self.assertTrue(os.access(exe, os.X_OK))
        role = self.role()
        self.assertEqual(role["role"]["exeName"], exe, "ls-hubd matches a role by the executable's path")
        self.assertEqual(set(role["role"]["allowedNames"]), CLIENT_NAMES, "no name but the library's client is registered")
        for rule in role["permissions"]:
            self.assertIn(rule["service"], CLIENT_NAMES)
            self.assertEqual(rule["inbound"], [], "nothing is offered to any caller")
            self.assertEqual(set(rule["outbound"]), MAY_CALL, "it may call the capture service, its permission service and the bus, nothing else")
        self.assertEqual(self.ls_calls(), ["scan-volatile-dirs", "scan-services"], "the hub is asked to read it")

    def test_installing_again_does_not_ask_the_hub_to_rescan_and_a_new_binary_is_swapped_in(self):
        self.install()
        self.assertEqual(len(self.ls_calls()), 2)
        self.assertEqual(self.install().returncode, 0)
        self.assertEqual(len(self.ls_calls()), 2, "the role did not change, so every boot does not rescan")
        newer = os.path.join(self.tmp, "newer")
        with open(newer, "w") as f:
            f.write("#!/bin/sh\nexec sleep 31\n")
        os.chmod(newer, 0o755)
        self.assertEqual(self.install(newer).returncode, 0)
        with open(os.path.join(self.state, "bin", "lumatv-capture")) as f:
            self.assertIn("sleep 31", f.read())
        self.assertEqual(len(self.ls_calls()), 2, "the same path, the same role")

    def test_a_package_without_the_binary_installs_nothing(self):
        r = self.install(os.path.join(self.tmp, "no-such-binary"))
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse(os.path.exists(os.path.join(self.ls2, "roles.d", "lumatv-capture.json")))
        self.assertEqual(self.ls_calls(), [])

    def test_remove_takes_the_role_and_the_binary_away_and_asks_for_a_rescan_once(self):
        self.install()
        self.assertEqual(json.loads(self.run_sh("status", self.state).stdout)["role"], True)
        self.assertEqual(self.run_sh("remove", self.state).returncode, 0)
        self.assertFalse(os.path.exists(os.path.join(self.ls2, "roles.d", "lumatv-capture.json")))
        self.assertFalse(os.path.exists(os.path.join(self.state, "bin", "lumatv-capture")))
        self.assertEqual(json.loads(self.run_sh("status", self.state).stdout), {"installed": False, "role": False, "running": False, "pid": 0})
        self.assertEqual(self.ls_calls()[2:], ["scan-volatile-dirs", "scan-services"])
        self.run_sh("remove", self.state)
        self.assertEqual(len(self.ls_calls()), 4, "removing what is not there asks nothing")


@unittest.skipUnless(os.path.exists(HOST_BIN), "needs ambient/capture/build.sh (the host build)")
class EndToEnd(Base):
    """The staged, started service is the real one: it finds the helper's socket and sends what the fake library says."""

    def test_start_runs_the_staged_binary_once_and_stop_ends_it(self):
        self.install(HOST_BIN)
        sock = os.path.join(self.tmp, "ambient.sock")
        self.assertEqual(self.run_sh("start", self.state, sock).returncode, 0)
        st = json.loads(self.run_sh("status", self.state).stdout)
        self.assertTrue(st["running"], "it is the staged executable that runs: that is what the role names")
        pid = st["pid"]
        self.run_sh("start", self.state, sock)
        self.assertEqual(json.loads(self.run_sh("status", self.state).stdout)["pid"], pid, "started once, not twice")
        self.assertEqual(self.run_sh("stop", self.state).returncode, 0)
        self.assertFalse(json.loads(self.run_sh("status", self.state).stdout)["running"])
        self.assertFalse(os.path.exists("/proc/%d" % pid) and open("/proc/%d/stat" % pid).read().split()[2] != "Z", "and it is gone")

    def test_the_started_service_connects_to_the_helper_socket_and_sends_frames(self):
        self.install(HOST_BIN)
        sock = os.path.join(self.tmp, "ambient.sock")
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(sock)
        listener.listen(1)
        listener.settimeout(8)
        self.addCleanup(listener.close)
        self.assertEqual(self.run_sh("start", self.state, sock).returncode, 0)
        conn, _ = listener.accept()
        conn.settimeout(3)
        conn.sendall(b"\x00")
        data = b""
        while len(data) < 204:
            data += conn.recv(204 - len(data))
        self.assertEqual(data[:4], b"AMB1")
        conn.close()
        end = time.time() + 3
        log = os.path.join(self.state, "capture.log")
        def text():
            if not os.path.exists(log):
                return ""
            with open(log) as f:
                return f.read()
        while time.time() < end and "capture ended" not in text():
            time.sleep(0.05)
        self.assertIn("capture ended (the helper no longer wants a picture)", text())


if __name__ == "__main__":
    unittest.main()
