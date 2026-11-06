#!/usr/bin/env python3
"""The AI access module on the set. Python 3, standard library only; it runs ON the TV (smoke-ai-tv.py uploads and runs it).

    python3 smoke-ai.py --helper ./lumatv-helper [--actions] [--receiver] [--lan-check] [--human] [--probe-keys SECONDS]
    python3 smoke-ai.py --helper target/debug/lumatv-helper --fake-input /tmp/ai-dry     # a dry run on a host

It starts its own helper in a temporary data dir on its own ports (the production helper and its data are not touched), and in
the end leaves AI access off and nothing running. Checks, in order:

  A  off by default: the state says off and nothing listens on the AI port.
  N  network keys on the real devices: asking turns on a prompt; OK and Back from "Smart Remote RCU Input" and "LGE Network
     Input" (where Home Assistant's and this module's own keys arrive) do not answer it. These presses are real key events
     on the set: leave it on a neutral screen (Home).
  F  the whole chain, with "LGE Network Input" standing in for the physical remote in this test's own config only: the
     listener binds the TV's private address, pairing, the MCP tools, the ask flow (Back denies and pauses, a network OK
     does nothing), lock-out. Nothing here turns the TV off or changes what it shows unless --actions or --receiver is given.
  H  with --human: the real path. It asks for an OK on the physical remote (IR or Magic Remote) and passes when that turns the
     listener on.
  K  with --probe-keys N: prints every key event on every input device for N seconds (measure CEC and other remotes).

--actions    also opens the app that is already in front (a no-op) and checks that a playback key reaches the input device.
--receiver   also sets the receiver's mute and volume to the values it already has (a command goes out; nothing changes).
--lan-check  pairs a second assistant and waits for the host to reach the listener over the LAN (smoke-ai-tv.py does that).

Exit status 0 = VERDICT PASS.
"""
import argparse
import http.client
import json
import os
import select
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time

KEY_OK, KEY_BACK = 28, 412
EV_SYN, EV_KEY = 0, 1
EVENT = struct.Struct("@llHHi")  # native size: 16 bytes on a 32-bit python, 24 on a 64-bit one; the kernel converts for either
NETWORK = ["Smart Remote RCU Input", "LGE Network Input"]
STAND_IN = "LGE Network Input"
PROMPT_SECONDS = 25
HERE = os.path.dirname(os.path.abspath(__file__))


class Report:
    def __init__(self):
        self.passed = 0
        self.failed = 0
        self.skipped = 0

    def check(self, name, ok, detail=""):
        if ok:
            self.passed += 1
            print("PASS", name, flush=True)
        else:
            self.failed += 1
            print("FAIL", name + (": " + str(detail) if detail != "" else ""), flush=True)
        return bool(ok)

    def skip(self, name, why):
        self.skipped += 1
        print("SKIP", name + ": " + why, flush=True)

    @staticmethod
    def fact(name, value):
        print("FACT", name + ":", value, flush=True)


# ------------------------------------------------------------------------------------------------ input devices

class Devices:
    """The input devices by name: where they are, and key presses written to them."""

    def __init__(self, sys_dir, dev_dir, hold=False):
        self.sys_dir, self.dev_dir, self.hold = sys_dir, dev_dir, hold
        self.held = {}

    def names(self):
        found = {}
        try:
            entries = sorted(os.listdir(self.sys_dir))
        except OSError:
            return found
        for entry in entries:
            if not entry.startswith("event"):
                continue
            try:
                with open(os.path.join(self.sys_dir, entry, "device", "name")) as f:
                    name = f.read().strip()
            except OSError:
                continue
            found.setdefault(name, []).append(os.path.join(self.dev_dir, entry))
        return found

    def node(self, name):
        nodes = self.names().get(name)
        return nodes[0] if nodes else None

    def _write(self, node, data):
        if self.hold:
            fd = self.held.get(node)
            if fd is None:
                fd = self.held[node] = os.open(node, os.O_RDWR | os.O_NONBLOCK)
            os.write(fd, data)
            return
        fd = os.open(node, os.O_WRONLY)
        try:
            os.write(fd, data)
        finally:
            os.close(fd)

    def press(self, name, code, hold_seconds=0.03):
        """A person's tap on the device `name`: press, a moment, release."""
        node = self.node(name)
        if node is None:
            raise RuntimeError("no input device named %r" % name)
        press = EVENT.pack(0, 0, EV_KEY, code, 1) + EVENT.pack(0, 0, EV_SYN, 0, 0)
        release = EVENT.pack(0, 0, EV_KEY, code, 0) + EVENT.pack(0, 0, EV_SYN, 0, 0)
        self._write(node, press)
        time.sleep(hold_seconds)
        self._write(node, release)

    def probe(self, seconds):
        """Prints every key event on every device for `seconds`."""
        files = {}
        for name, nodes in self.names().items():
            for node in nodes:
                try:
                    files[os.open(node, os.O_RDONLY | os.O_NONBLOCK)] = (name, node, bytearray())
                except OSError as e:
                    print("NOTE could not open", node, name, e, flush=True)
        end = time.time() + seconds
        while time.time() < end and files:
            ready, _, _ = select.select(list(files), [], [], 0.2)
            for fd in ready:
                name, node, buf = files[fd]
                try:
                    buf.extend(os.read(fd, 4096))
                except BlockingIOError:
                    continue
                except OSError:
                    continue
                while len(buf) >= EVENT.size:
                    _, _, kind, code, value = EVENT.unpack(bytes(buf[:EVENT.size]))
                    del buf[:EVENT.size]
                    if kind == EV_KEY:
                        print("KEY", repr(name), os.path.basename(node), "code", code, "value", value, flush=True)
        for fd in files:
            os.close(fd)

    def close(self):
        for fd in self.held.values():
            os.close(fd)
        self.held = {}


def make_fake_input(root):
    """A tree of FIFOs and name files standing in for /dev/input and /sys/class/input (a dry run on a host)."""
    shutil.rmtree(root, ignore_errors=True)
    names = ["LGE RCU", "LGE M-RCU - Builtin [0]", "Smart Remote RCU Input", "LGE Network Input"]
    for n, name in enumerate(names):
        os.makedirs(os.path.join(root, "sys", "event%d" % n, "device"))
        os.makedirs(os.path.join(root, "dev"), exist_ok=True)
        with open(os.path.join(root, "sys", "event%d" % n, "device", "name"), "w") as f:
            f.write(name + "\n")
        os.mkfifo(os.path.join(root, "dev", "event%d" % n), 0o600)
    return os.path.join(root, "sys"), os.path.join(root, "dev")


def make_fake_luna(root):
    """A luna-send that answers from the recorded replies in app/tests/fixtures/luna (a dry run on a host)."""
    fixtures = os.path.join(HERE, "app", "tests", "fixtures", "luna")
    os.makedirs(os.path.join(root, "bin"), exist_ok=True)
    path = os.path.join(root, "bin", "luna-send")
    with open(path, "w") as f:
        f.write(
            "#!/bin/sh\n"
            'case "$3" in\n'
            "  *getForegroundAppInfo) printf '{\"returnValue\":true,\"appId\":\"netflix\"}\\n' ;;\n"
            "  *listApps) cat %(d)s/listApps.json ;;\n"
            "  *getAllInputStatus) cat %(d)s/getAllInputStatus.json ;;\n"
            "  *launch|*powerOff) cat %(d)s/ok.json ;;\n"
            '  *getPowerState) printf \'{"returnValue":true,"state":"Active"}\\n\' ;;\n'
            "  *) printf '{\"returnValue\":false}\\n' ;;\n"
            "esac\n" % {"d": fixtures}
        )
    os.chmod(path, 0o755)
    return path


# ------------------------------------------------------------------------------------------------ the run

def listening(port):
    """Listening TCP sockets on `port`, as (address, port) from /proc/net/tcp and tcp6."""
    found = []
    for path in ("/proc/net/tcp", "/proc/net/tcp6"):
        try:
            with open(path) as f:
                lines = f.read().splitlines()[1:]
        except OSError:
            continue
        for line in lines:
            fields = line.split()
            if len(fields) < 4 or fields[3] != "0A":
                continue
            host, hex_port = fields[1].split(":")
            if int(hex_port, 16) != port:
                continue
            if len(host) == 8:
                found.append((socket.inet_ntoa(bytes.fromhex(host)[::-1]), port))
            else:
                found.append(("v6:" + host, port))
    return found


def source_address():
    """The address the kernel would use to reach the LAN (a UDP connect sends nothing)."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("192.0.2.1", 9))
        return s.getsockname()[0]
    except OSError:
        return None
    finally:
        s.close()


def port_refuses(ip, port):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(2)
    try:
        s.connect((ip, port))
        return False
    except OSError:
        return True
    finally:
        s.close()


def wait_for(predicate, seconds, step=0.1):
    end = time.time() + seconds
    while time.time() < end:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return predicate()


class Run:
    def __init__(self, args):
        self.args = args
        self.report = Report()
        self.fake = args.fake_input is not None
        self.work = args.work
        self.data = os.path.join(self.work, "data")
        self.binary = os.path.abspath(args.helper)
        if self.fake:
            sys_dir, dev_dir = make_fake_input(args.fake_input)
            self.devices = Devices(sys_dir, dev_dir, hold=True)
            self.luna = make_fake_luna(args.fake_input)
            self.lan_ip = "127.0.0.1"
        else:
            self.devices = Devices("/sys/class/input", "/dev/input")
            self.luna = None
            self.lan_ip = source_address()
        self.token = None
        self.assistant = None
        self.denied_at = 0.0
        self.quiet = False
        os.makedirs(self.data, exist_ok=True)

    # ---- the helper process
    def write_config(self, physical):
        ai = {"port": self.args.ai_port, "promptSeconds": PROMPT_SECONDS}
        if physical:
            ai["physicalDevices"] = physical
            ai["networkDevices"] = [n for n in NETWORK if n not in physical]
        if self.fake:
            ai.update({"bind": "127.0.0.1", "inputDir": self.devices.dev_dir, "sysInputDir": self.devices.sys_dir})
        config = {"listen": {"port": self.args.port}, "nice": 10, "ai": ai}
        if self.fake:
            config["screen"] = {"source": "on"}
            config["luna"] = {"send": self.luna}
        os.makedirs(self.data, exist_ok=True)
        with open(os.path.join(self.data, "helper.json"), "w") as f:
            json.dump(config, f)

    def helper(self, verb):
        out = subprocess.run([self.binary, verb, "--data-dir", self.data], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             universal_newlines=True, timeout=40)
        text = out.stdout.strip()
        try:
            return json.loads(text.splitlines()[-1]) if text else {}
        except ValueError:
            return {"ok": False, "raw": text, "stderr": out.stderr[-300:]}

    def start(self, physical=None):
        self.write_config(physical)
        info = self.helper("start")
        started = bool(info.get("started") or info.get("already"))
        self.report.check("the helper starts (%s)" % ("a test config" if physical else "the default config"), started, info)
        if not started:
            raise RuntimeError("the helper did not start: %s" % self.log_tail())
        return info

    def stop(self):
        info = self.helper("stop")
        self.report.check("the helper stops", bool(info.get("ok")) and not self.helper("status").get("running"), info)

    # ---- the loopback API
    def api(self, method, path, body=None):
        conn = http.client.HTTPConnection("127.0.0.1", self.args.port, timeout=10)
        headers = {"X-Luma-Client": "smoke-ai", "Content-Type": "application/json"}
        try:
            conn.request(method, path, None if body is None else json.dumps(body), headers)
            response = conn.getresponse()
            raw = response.read()
            return response.status, (json.loads(raw) if raw else None)
        except (OSError, ValueError, http.client.HTTPException):
            return 0, None
        finally:
            conn.close()

    def doc(self):
        status, body = self.api("GET", "/v1/ai")
        return (body or {}).get("ai") if status == 200 else None

    def prompt(self):
        status, body = self.api("GET", "/v1/state/ai.prompt")
        return (body or {}).get("value") if status == 200 else None

    def wait_prompt(self, seconds=10):
        return wait_for(self.prompt, seconds)

    def wait_no_prompt(self, seconds=10):
        return wait_for(lambda: self.prompt() is None, seconds)

    def tap(self, device, code):
        """A key press after the prompt has been up long enough to count (the gate ignores a key pressed in its first 300 ms)."""
        time.sleep(0.5)
        self.devices.press(device, code)

    # ---- the LAN door
    def lan(self, method, path, token=None, body=None, protocol_version=None):
        conn = http.client.HTTPConnection(self.lan_ip, self.args.ai_port, timeout=10)
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        if protocol_version:
            headers["MCP-Protocol-Version"] = protocol_version
        try:
            conn.request(method, path, None if body is None else json.dumps(body), headers)
            response = conn.getresponse()
            raw = response.read()
            return response.status, (json.loads(raw) if raw else None)
        except (OSError, ValueError, http.client.HTTPException) as e:
            return 0, str(e)
        finally:
            conn.close()

    def rpc(self, method, params=None, token=None):
        status, body = self.lan(
            "POST", "/mcp", token or self.token,
            {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}},
            protocol_version="2025-06-18",
        )
        return status, body

    def call(self, tool, arguments=None):
        status, body = self.rpc("tools/call", {"name": tool, "arguments": arguments or {}})
        result = (body or {}).get("result") if isinstance(body, dict) else None
        return status, result

    @staticmethod
    def text(result):
        try:
            return result["content"][0]["text"]
        except (TypeError, KeyError, IndexError):
            return ""

    def call_asking(self, tool, arguments, answer):
        """Calls a tool that asks on the TV from a thread, lets `answer()` act while the prompt is up, returns the result."""
        box = {}
        worker = threading.Thread(target=lambda: box.update(result=self.call(tool, arguments)))
        worker.start()
        prompt = self.wait_prompt(10)
        if prompt:
            answer(prompt)
        worker.join(PROMPT_SECONDS + 10)
        return prompt, box.get("result", (0, None))[1]

    def power_state(self):
        if self.fake:
            return "Active"
        out = subprocess.run([self.luna or "luna-send", "-n", "1", "luna://com.webos.service.tvpower/power/getPowerState", "{}"],
                             stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             universal_newlines=True, timeout=10)
        try:
            return json.loads(out.stdout).get("state")
        except ValueError:
            return None

    def log_tail(self):
        # The helper keeps no log file: its lines go to the collector (/tmp/lumatv-log.sock), which retains them under /tmp/lumatv-logs.
        return "(no log file; see the collector's /tmp/lumatv-logs)"

    # ---- the phases
    def phase_environment(self):
        r = self.report
        r.fact("machine", os.uname().machine)
        r.fact("python", sys.version.split()[0])
        names = self.devices.names()
        r.fact("input devices", "; ".join("%s=%s" % (n, ",".join(os.path.basename(p) for p in v)) for n, v in sorted(names.items())))
        r.fact("LAN address (kernel's source for the default route)", self.lan_ip)
        r.check("the physical remote's devices exist", any(n == "LGE RCU" or n.startswith("LGE M-RCU") for n in names),
                "none of 'LGE RCU' or 'LGE M-RCU*' is present: no key could ever answer a prompt")
        r.check("the network-key devices exist", all(n in names for n in NETWORK), [n for n in NETWORK if n not in names])
        checked = self.helper("check-config")
        r.check("the binary runs here and reads its config", bool(checked.get("ok")), checked)

    def phase_off(self):
        r = self.report
        self.start()
        doc = self.doc() or {}
        listener = doc.get("listener", {})
        r.check("A: the state says off, unbound, nobody paired",
                listener.get("enabled") is False and listener.get("bound") is False and doc.get("assistants") == []
                and doc.get("pairing") is None, doc)
        r.check("A: nothing listens on the AI port", listening(self.args.ai_port) == [], listening(self.args.ai_port))
        for method, path, body in [("POST", "/v1/ai/enable", {"enabled": True}), ("PUT", "/v1/ai/listener", {"enabled": True}),
                                   ("POST", "/v1/settings/ai", {"enabled": True}), ("GET", "/v1/ai/listener", None),
                                   ("DELETE", "/v1/ai", None), ("POST", "/v1/ai", {"listener": {"enabled": True}})]:
            self.api(method, path, body)
        r.check("A: requests that look like 'turn it on' did not",
                (self.doc() or {}).get("listener", {}).get("enabled") is False and listening(self.args.ai_port) == [])

    def phase_network_keys(self):
        r = self.report
        status, body = self.api("POST", "/v1/ai/listener", {"enabled": True})
        r.check("N: asking to turn it on answers 202 and opens a prompt on the TV",
                status == 202 and bool(body and body.get("pending")), (status, body))
        prompt = self.wait_prompt()
        r.check("N: the prompt is in the ai.prompt topic for the screen to draw",
                bool(prompt) and prompt.get("kind") == "enable_listener", prompt)
        if not prompt:
            return
        time.sleep(0.6)
        for device in NETWORK:
            for code in (KEY_OK, KEY_BACK):
                self.devices.press(device, code)
                time.sleep(0.15)
        time.sleep(1.5)
        still = self.prompt()
        r.check("N: OK and Back from the network devices did not answer the prompt",
                bool(still) and still.get("id") == prompt.get("id")
                and (self.doc() or {}).get("listener", {}).get("enabled") is False
                and listening(self.args.ai_port) == [], still)
        status, body = self.api("POST", "/v1/ai/lockout", {})
        r.check("N: a lock-out withdraws the prompt", status == 200 and self.wait_no_prompt(5), (status, body))
        r.check("N: and AI access is still off", (self.doc() or {}).get("listener", {}).get("enabled") is False
                and listening(self.args.ai_port) == [])

    def pair_assistant(self, name):
        """Asks to pair, answers the prompt with the stand-in remote, presents the code on the LAN door."""
        status, body = self.api("POST", "/v1/ai/pairing", {})
        if not self.report.check("F: asking to pair answers 202 and opens a prompt", status == 202, (status, body)):
            return None, None
        if not self.wait_prompt():
            self.report.check("F: the pairing prompt appears", False, self.log_tail())
            return None, None
        time.sleep(0.8)
        before = (self.doc() or {}).get("pairing")
        self.devices.press("Smart Remote RCU Input", KEY_OK)
        time.sleep(1.0)
        during = (self.doc() or {}).get("pairing")
        self.devices.press(STAND_IN, KEY_OK)
        pairing = wait_for(lambda: (self.doc() or {}).get("pairing"), 8)
        self.report.check("F: the code appears only after the physical OK (not before, not after a network OK)",
                          before is None and during is None and bool(pairing), (before, during, pairing))
        if not pairing:
            return None, None
        status, body = self.lan("POST", "/pair", None, {"code": pairing["code"], "name": name})
        ok = status == 200 and isinstance(body, dict) and str(body.get("token", "")).startswith("ltv_")
        self.report.check("F: the code gives a token on the LAN door", ok, (status, body if not ok else "token"))
        if not ok:
            return None, None
        return body["token"], body.get("assistantId")

    def phase_full(self):
        r = self.report
        self.start(physical=[STAND_IN])
        status, body = self.api("POST", "/v1/ai/listener", {"enabled": True})
        r.check("F: asking to turn it on answers 202", status == 202, (status, body))
        if not self.wait_prompt():
            r.check("F: the prompt appears", False, self.log_tail())
            return
        self.tap(STAND_IN, KEY_OK)
        doc = wait_for(lambda: (self.doc() or {}).get("listener", {}).get("bound") and self.doc(), 10)
        r.check("F: the stand-in remote's OK turns the listener on and it binds", bool(doc), self.log_tail())
        if not doc:
            return
        bound = doc["listener"]["addr"]
        r.fact("bound", bound)
        ip, port = bound.rsplit(":", 1)
        sockets = listening(self.args.ai_port)
        r.check("F: it bound the TV's private address, the one the kernel uses for the LAN, not 0.0.0.0",
                ip == self.lan_ip and sockets == [(ip, int(port))], (ip, self.lan_ip, sockets))
        self.token, self.assistant = self.pair_assistant("smoke test")
        if not self.token:
            return
        status, _ = self.lan("POST", "/mcp", self.token[:-1] + ("1" if self.token[-1] == "0" else "0"),
                             {"jsonrpc": "2.0", "id": 1, "method": "ping"})
        status_none, _ = self.lan("POST", "/mcp", None, {"jsonrpc": "2.0", "id": 1, "method": "ping"})
        r.check("F: a wrong token and no token are both refused with 401", status == 401 and status_none == 401, (status, status_none))
        status, init = self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                               "clientInfo": {"name": "smoke-ai", "version": "1"}})
        r.check("F: MCP initialize answers", status == 200 and init["result"]["serverInfo"]["name"] == "lumatv", (status, init))
        status, listed = self.rpc("tools/list")
        names = [t["name"] for t in (listed or {}).get("result", {}).get("tools", [])]
        r.check("F: the default levels list all 13 tools", len(names) == 13, names)
        self.read_tools()
        self.ask_flow()
        if self.args.actions:
            self.actions()
        else:
            r.skip("F: open the app in front and press a playback key", "run with --actions")
        if self.args.receiver:
            self.receiver_same_values()
        else:
            r.skip("F: receiver mute and volume set to their current values", "run with --receiver (sends commands to the receiver)")
        if self.args.lan_check:
            self.lan_window()
        self.lock_out()

    def read_tools(self):
        r = self.report
        for tool in ("tv_get_power", "tv_get_foreground_app", "tv_list_inputs", "tv_list_apps", "tv_get_now_playing",
                     "receiver_get_status"):
            status, result = self.call(tool)
            text = self.text(result)
            r.fact(tool, text.replace("\n", " ")[:300])
            if tool == "receiver_get_status":
                ok = status == 200 and result is not None and (not result.get("isError") or text.startswith("unavailable"))
            else:
                ok = status == 200 and result is not None and not result.get("isError")
            r.check("F: %s answers" % tool, ok, (status, text))

    def ask_flow(self):
        r = self.report
        calls = {"n": 0}

        def network_ok_then_back(prompt):
            time.sleep(0.6)
            self.devices.press("Smart Remote RCU Input", KEY_OK)
            time.sleep(2.0)
            calls["ignored"] = self.prompt() is not None
            self.devices.press(STAND_IN, KEY_BACK)

        prompt, result = self.call_asking("tv_standby", {"reason": "smoke test, to be denied"}, network_ok_then_back)
        r.check("F: tv_standby asks on the TV, saying who and what", bool(prompt) and prompt.get("kind") == "call"
                and prompt.get("assistant", {}).get("name") == "smoke test", prompt)
        r.check("F: a network OK did not approve it", calls.get("ignored") is True)
        r.check("F: Back refused it and the answer says so", bool(result) and result.get("isError")
                and self.text(result).startswith("denied"), result)
        self.denied_at = time.time()
        state = self.power_state()
        r.check("F: the TV is still on (nothing was run)", state in ("Active", "Screen Saver"), state)
        doc = self.doc() or {}
        paused = [a for a in doc.get("assistants", []) if a.get("id") == self.assistant and a.get("paused")]
        r.check("F: and the assistant is paused", bool(paused), doc.get("assistants"))
        status, result = self.call("tv_get_power")
        r.check("F: a paused assistant is refused even for reads", bool(result) and result.get("isError")
                and self.text(result).startswith("paused"), result)
        status, body = self.api("POST", "/v1/ai/assistants/%s/resume" % self.assistant, {})
        r.check("F: resuming it answers 202 (it needs the remote)", status == 202, (status, body))
        if self.wait_prompt():
            self.tap(STAND_IN, KEY_OK)
        resumed = wait_for(lambda: all(not a.get("paused") for a in (self.doc() or {}).get("assistants", [{"paused": True}])), 8)
        r.check("F: the physical OK resumed it", bool(resumed), self.doc())

    def wait_out_cooldown(self):
        """After a refusal the module will not ask again for 10 s."""
        left = 10.5 - (time.time() - self.denied_at)
        if left > 0:
            time.sleep(left)

    def actions(self):
        r = self.report
        self.wait_out_cooldown()
        status, result = self.call("tv_get_foreground_app")
        text = self.text(result)
        app = None
        if result and not result.get("isError"):
            app = (result.get("structuredContent") or {}).get("appId")
        r.fact("foreground app", text[:200])
        if app:
            prompt, result = self.call_asking("tv_launch_app", {"app_id": app, "reason": "smoke test, the app already in front"},
                                              lambda p: self.tap(STAND_IN, KEY_OK))
            r.check("F: opening the app in front asks, and a physical OK runs it", bool(prompt)
                    and bool(result) and not result.get("isError"), (prompt, result))
        else:
            r.skip("F: opening the app in front", "no app is in front (Home); put an app in front and run again")
        node = self.devices.node("Smart Remote RCU Input")
        if node is None:
            r.skip("F: a playback key reaches the input device", "no 'Smart Remote RCU Input'")
            return
        fd = os.open(node, os.O_RDONLY | os.O_NONBLOCK)
        try:
            status, result = self.call("tv_media_key", {"key": "pause"})
            seen = []
            buf = bytearray()
            end = time.time() + 1.5
            while time.time() < end:
                ready, _, _ = select.select([fd], [], [], 0.1)
                if ready:
                    try:
                        buf.extend(os.read(fd, 4096))
                    except OSError:
                        break
                while len(buf) >= EVENT.size:
                    _, _, kind, code, value = EVENT.unpack(bytes(buf[:EVENT.size]))
                    del buf[:EVENT.size]
                    if kind == EV_KEY:
                        seen.append((code, value))
            r.fact("events read from 'Smart Remote RCU Input' while tv_media_key pause ran", seen)
            r.check("F: the pause key was reported done", bool(result) and not result.get("isError"), result)
            r.check("F: the pause key reached the input device (press then release)", (119, 1) in seen and (119, 0) in seen, seen)
            r.fact("a key pressed there is a network key: it must not have answered anything", (self.doc() or {}).get("listener"))
        finally:
            os.close(fd)

    def receiver_same_values(self):
        r = self.report
        status, result = self.call("receiver_get_status")
        structured = (result or {}).get("structuredContent") or {}
        volume, mute = (structured.get("volume") or {}).get("level"), structured.get("mute")
        r.fact("receiver now", structured)
        if volume is None or mute is None:
            r.skip("F: receiver mute and volume set to their current values", "no fresh reading: %s" % self.text(result)[:120])
            return
        status, result = self.call("receiver_set_mute", {"muted": bool(mute)})
        r.check("F: the receiver accepted mute set to what it already is", bool(result) and not result.get("isError"), self.text(result))
        status, result = self.call("receiver_set_volume", {"level": float(volume)})
        r.check("F: the receiver accepted the volume it already has", bool(result) and not result.get("isError"), self.text(result))
        time.sleep(1.5)
        status, after = self.call("receiver_get_status")
        again = (after or {}).get("structuredContent") or {}
        r.check("F: and nothing changed", (again.get("volume") or {}).get("level") == volume and again.get("mute") == mute,
                (structured, again))

    def lan_window(self):
        """Pairs a second assistant and waits for a host-side verifier without ever putting its bearer token in output."""
        token, _assistant = self.pair_assistant("smoke host")
        if not token:
            return
        request = os.path.join(self.work, "host-request.json")
        result = os.path.join(self.work, "host-result.json")
        with open(request, "w") as f:
            json.dump({"ip": self.lan_ip, "port": self.args.ai_port, "token": token}, f)
        os.chmod(request, 0o600)
        print("LAN_PAIR_READY", flush=True)
        got = wait_for(lambda: os.path.exists(result), 90, 0.5)
        answer = None
        if got:
            try:
                with open(result) as f:
                    answer = json.load(f)
            except (OSError, ValueError):
                answer = {"ok": False, "error": "the host result was not JSON"}
        self.report.check(
            "F: the host reached the listener over the LAN (it reports its own results)",
            bool(answer and answer.get("ok")),
            answer if got else "no answer from the host within 90 s",
        )

    def lock_out(self):
        r = self.report
        status, body = self.api("POST", "/v1/ai/lockout", {})
        r.check("F: lock-out answers", status == 200 and body is not None, (status, body))
        closed = wait_for(lambda: listening(self.args.ai_port) == [], 5)
        r.check("F: the port closes", bool(closed), listening(self.args.ai_port))
        r.check("F: and refuses connections", self.lan_ip is not None and port_refuses(self.lan_ip, self.args.ai_port))
        doc = self.doc() or {}
        r.check("F: nobody stays paired and the switch is off", doc.get("assistants") == []
                and doc.get("listener", {}).get("enabled") is False, doc.get("assistants"))

    def phase_human(self):
        r = self.report
        self.start()
        status, body = self.api("POST", "/v1/ai/listener", {"enabled": True})
        r.check("H: asking to turn it on answers 202", status == 202, (status, body))
        prompt = self.wait_prompt()
        if not prompt:
            r.check("H: the prompt appears", False, self.log_tail())
            return
        print("ACTION: press OK on the physical remote (IR or Magic Remote) within %d s; press Back to refuse" % PROMPT_SECONDS,
              flush=True)
        enabled = wait_for(lambda: (self.doc() or {}).get("listener", {}).get("enabled"), PROMPT_SECONDS + 3, 0.25)
        r.check("H: a press of OK on the physical remote turned the listener on", bool(enabled), self.log_tail())
        self.api("POST", "/v1/ai/lockout", {})
        r.check("H: lock-out turned it off again", wait_for(lambda: listening(self.args.ai_port) == [], 5) == True)  # noqa: E712

    def finish(self):
        r = self.report
        self.api("POST", "/v1/ai/lockout", {})
        self.helper("stop")
        self.devices.close()
        shutil.rmtree(self.work, ignore_errors=True)
        if self.fake:
            shutil.rmtree(self.args.fake_input, ignore_errors=True)
        if self.quiet:
            return 0
        verdict = "PASS" if r.failed == 0 and r.passed > 0 else "FAIL"
        print("VERDICT %s %d/%d (skipped %d)" % (verdict, r.passed, r.passed + r.failed, r.skipped), flush=True)
        return 0 if verdict == "PASS" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--helper", default="./lumatv-helper")
    parser.add_argument("--work", default="/tmp/lumatv-ai-smoke/run")
    parser.add_argument("--port", type=int, default=18741, help="the loopback API of the test helper")
    parser.add_argument("--ai-port", type=int, default=18743, help="the LAN listener of the test helper")
    parser.add_argument("--fake-input", help="a dry run: FIFOs under this directory stand in for the input devices")
    parser.add_argument("--actions", action="store_true")
    parser.add_argument("--receiver", action="store_true")
    parser.add_argument("--lan-check", action="store_true")
    parser.add_argument("--human", action="store_true")
    parser.add_argument("--probe-keys", type=int, metavar="SECONDS")
    args = parser.parse_args()
    shutil.rmtree(args.work, ignore_errors=True)
    os.makedirs(args.work, exist_ok=True)
    run = Run(args)
    try:
        if args.probe_keys:
            run.quiet = True
            print("Press keys on every remote now (%d s)." % args.probe_keys, flush=True)
            run.devices.probe(args.probe_keys)
            return 0
        run.phase_environment()
        run.phase_off()
        run.phase_network_keys()
        run.stop()
        run.phase_full()
        run.stop()
        if args.human:
            run.phase_human()
            run.stop()
    except Exception as e:  # a crash is a failure with its reason, and the helper still gets stopped
        run.report.check("the test itself ran to the end", False, repr(e))
    finally:
        code = run.finish()
    return code


if __name__ == "__main__":
    sys.exit(main())
