#!/usr/bin/env python3
"""Uploads the AI smoke test to the TV and drives its optional LAN check.

    python3 helper/smoke-ai-tv.py --run --confirm-neutral-screen
    python3 helper/smoke-ai-tv.py --run --confirm-neutral-screen --human
    python3 helper/smoke-ai-tv.py --run --confirm-neutral-screen --actions --receiver --lan-check

`--run` and `--confirm-neutral-screen` are both deliberate gates: this is an on-set test. The normal run presses OK and Back
through the TV's *network input* while checking that the production physical-remote rules ignore them. Leave the TV at Home
with no dialog, playback, camera, alarm or other important UI open. `--actions` opens the app already in front and sends
Pause; `--receiver` sends the Denon its current mute and volume values; `--human` stops to ask for one physical-remote OK.

Everything goes under /tmp/lumatv-ai-smoke and is removed at the end. The production helper, its data and its ports are not
touched. The temporary bearer token for the optional LAN check is transferred in a 0600 temporary file and is never printed,
saved in this repository or included in the output artifact. Exit status 0 = VERDICT PASS.
"""
import argparse
import http.client
import json
import os
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "docs", "research", "arch"))
import tv  # noqa: E402

REMOTE = "/tmp/lumatv-ai-smoke"


def remote(command, timeout=120):
    return tv.tv(command, timeout=timeout)


def read_remote(path):
    fd, local = tempfile.mkstemp(prefix="lumatv-ai-")
    os.close(fd)
    try:
        tv.get(path, local)
        with open(local) as f:
            return f.read()
    finally:
        try:
            os.unlink(local)
        except OSError:
            pass


def post_mcp(ip, port, token, method, params):
    """The host's part of an optional LAN verification. Do not put `token` in errors or output."""
    conn = http.client.HTTPConnection(ip, port, timeout=10)
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    headers = {
        "Authorization": "Bearer " + token,
        "Content-Type": "application/json",
        "MCP-Protocol-Version": "2025-06-18",
    }
    try:
        conn.request("POST", "/mcp", body, headers)
        response = conn.getresponse()
        raw = response.read()
        return response.status, json.loads(raw) if raw else None
    finally:
        conn.close()


def lan_check():
    """Uses the token that lives only in REMOTE/run/host-request.json, then returns a safe result to the test."""
    try:
        request = json.loads(read_remote(REMOTE + "/run/host-request.json"))
        ip, port, token = request["ip"], int(request["port"]), request["token"]
        initialized_status, initialized = post_mcp(
            ip,
            port,
            token,
            "initialize",
            {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "smoke host", "version": "1"}},
        )
        listed_status, listed = post_mcp(ip, port, token, "tools/list", {})
        tools = ((listed or {}).get("result") or {}).get("tools") or []
        ok = (
            initialized_status == 200
            and ((initialized or {}).get("result") or {}).get("serverInfo", {}).get("name") == "lumatv"
            and listed_status == 200
            and len(tools) == 13
        )
        result = {"ok": ok, "initializeStatus": initialized_status, "toolsStatus": listed_status, "toolCount": len(tools)}
    except Exception as e:
        result = {"ok": False, "error": type(e).__name__ + ": " + str(e)[:180]}
    fd, local = tempfile.mkstemp(prefix="lumatv-ai-result-")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(result, f)
        os.chmod(local, 0o600)
        tv.put(local, REMOTE + "/run/host-result.json")
        remote("chmod 600 %s/run/host-result.json" % REMOTE)
    finally:
        try:
            os.unlink(local)
        except OSError:
            pass
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--run", action="store_true", help="perform the on-set test")
    parser.add_argument("--confirm-neutral-screen", action="store_true", help="confirm that the TV is on Home with no important UI open")
    parser.add_argument("--actions", action="store_true")
    parser.add_argument("--receiver", action="store_true")
    parser.add_argument("--lan-check", action="store_true")
    parser.add_argument("--human", action="store_true")
    parser.add_argument("--probe-keys", type=int, metavar="SECONDS", help="only measure raw key events for this many seconds")
    parser.add_argument("--port", type=int, default=18741)
    parser.add_argument("--ai-port", type=int, default=18743)
    args = parser.parse_args()
    if not args.run:
        parser.error("refusing to touch the TV without --run")
    if not args.confirm_neutral_screen and not args.probe_keys:
        parser.error("refusing to inject test keys without --confirm-neutral-screen")

    binary = os.path.join(HERE, "dist", "lumatv-helper")
    script = os.path.join(HERE, "smoke-ai.py")
    for path in (binary, script):
        if not os.path.exists(path):
            parser.error("missing %s (run helper/build.sh first)" % path)

    output = ""
    exit_code = None
    seen = 0
    checked_lan = False
    command = ["python3", "smoke-ai.py", "--helper", "./lumatv-helper", "--work", REMOTE + "/run", "--port", str(args.port), "--ai-port", str(args.ai_port)]
    for flag, enabled in (("--actions", args.actions), ("--receiver", args.receiver), ("--lan-check", args.lan_check), ("--human", args.human)):
        if enabled:
            command.append(flag)
    if args.probe_keys:
        command += ["--probe-keys", str(args.probe_keys)]
    quoted = " ".join("'%s'" % part.replace("'", "'\\"'\\"'") for part in command)
    try:
        remote("rm -rf {r} && mkdir -p {r} && chmod 700 {r}".format(r=REMOTE))
        for local, name in ((binary, "lumatv-helper"), (script, "smoke-ai.py")):
            print("put", name, flush=True)
            tv.put(local, REMOTE + "/" + name)
        remote("chmod 700 {r}/lumatv-helper {r}/smoke-ai.py && cd {r} && ( {cmd} >run.out 2>&1; echo $? >exit ) & echo $! >pid".format(r=REMOTE, cmd=quoted))
        deadline = time.time() + (180 if args.probe_keys else 300)
        while time.time() < deadline:
            current = remote("test -f {r}/run.out && cat {r}/run.out || :".format(r=REMOTE), timeout=30)
            if len(current) > seen:
                print(current[seen:], end="", flush=True)
                seen = len(current)
            output = current
            if args.lan_check and not checked_lan and "LAN_PAIR_READY" in current:
                checked_lan = True
                result = lan_check()
                print("host LAN check:", "PASS" if result.get("ok") else "FAIL", flush=True)
            finished = remote("test -f {r}/exit && cat {r}/exit || :".format(r=REMOTE), timeout=30).strip()
            if finished:
                exit_code = int(finished) if finished.isdigit() else 1
                break
            time.sleep(1)
        if exit_code is None:
            print("FAIL timeout waiting for the TV smoke test", flush=True)
            remote("test -f {r}/pid && kill $(cat {r}/pid) 2>/dev/null || :".format(r=REMOTE))
            exit_code = 1
        output = remote("test -f {r}/run.out && cat {r}/run.out || :".format(r=REMOTE), timeout=30)
        if len(output) > seen:
            print(output[seen:], end="", flush=True)
        os.makedirs(os.path.join(HERE, "dist"), exist_ok=True)
        saved = os.path.join(HERE, "dist", "smoke-ai-%s.txt" % time.strftime("%Y%m%d-%H%M%S"))
        with open(saved, "w") as f:
            f.write(output)
        print("saved", saved)
        return 0 if exit_code == 0 and "VERDICT PASS" in output else 1
    finally:
        remote("rm -rf %s" % REMOTE)


if __name__ == "__main__":
    sys.exit(main())
