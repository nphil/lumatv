#!/usr/bin/env python3
"""The Bluetooth proxy end to end on a host: the real helper binary, a fake luna-send, and Home Assistant's own client library.

    python3 helper/smoke-ble.py [--bin helper/target/debug/lumatv-helper]

Needs `aioesphomeapi` (pip install aioesphomeapi, in a virtualenv is fine); without it the script says so and exits 0 so a gate that
runs every smoke script does not fail on a machine without it. What it checks: the helper takes up the link, answers as an ESPHome
Bluetooth proxy (feature flags: passive scan, raw advertisements, state and mode), tells the scanner state, passes the advertisements
of one window and none in the rest, and a helper that is killed (not stopped) takes its scan child with it.
"""

import asyncio
import json
import os
import signal
import subprocess
import sys
import tempfile
import time

try:
    from aioesphomeapi import APIClient
except ImportError:
    print("smoke-ble: aioesphomeapi is not installed; skipped")
    sys.exit(0)

ROOT = os.path.dirname(os.path.abspath(__file__))
PORT = 16053
WINDOW, REST = 6, 10

FAKE_LUNA = """#!/bin/sh
# Answers like the set did, and advertises an ATC sensor five times a second for as long as it lives.
printf '%s\\n' "$*" >> "$DIR/args"
case "$*" in
*adapter/getStatus*)
  echo '{"adapters":[{"powered":true,"adapterAddress":"84:a3:29:b9:4d:43","name":"Living Room TV"}],"returnValue":true,"subscribed":false}' ;;
*device/getStatus*)
  echo '{"subscribed":false,"returnValue":true,"devices":[{"name":"LGE MR24","connectedProfiles":[],"typeOfDevice":"ble","address":"30:3f:5d:ef:ca:49","scanRecord":[],"manufacturerData":{}}]}' ;;
*le/startScan*)
  echo $$ > "$DIR/pid"
  echo '{"subscribed":true,"adapterAddress":"84:a3:29:b9:4d:43","returnValue":true}'
  n=0
  while :; do
    n=$((n+1)); r=$((-60 - (n % 7)))
    echo "{\\"device\\":{\\"address\\":\\"a4:c1:38:12:34:56\\",\\"name\\":\\"ATC_123456\\",\\"rssi\\":$r,\\"typeOfDevice\\":\\"ble\\",\\"scanRecord\\":[2,1,6,9,9,65,84,67,95,49,50,51,52,13,22,26,24,164,193,56,18,52,86,0,213,45,88,11,184,0,0,0,0],\\"manufacturerData\\":{}},\\"returnValue\\":true,\\"subscribed\\":true,\\"devices\\":[]}"
    sleep 0.2
  done ;;
esac
"""


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout


async def exercise(host, key):
    client = APIClient(host, PORT, None, noise_psk=key)
    await client.connect(login=True)
    info = await client.device_info()
    flags = info.bluetooth_proxy_feature_flags_compat(client.api_version)
    assert flags == 1 | 32 | 64, f"feature flags {flags}"
    assert info.bluetooth_mac_address == "84:A3:29:B9:4D:43", info.bluetooth_mac_address
    states, adverts = [], []
    client.subscribe_bluetooth_scanner_state(lambda s: states.append((s.state.name, s.mode.name)))
    unsubscribe = client.subscribe_bluetooth_le_raw_advertisements(adverts.append)
    await asyncio.sleep(WINDOW + 2)
    in_window = len(adverts)
    # 5 a second for WINDOW seconds, less what the start costs.
    assert WINDOW * 5 * 0.7 <= in_window <= WINDOW * 5 + 5, f"{in_window} advertisements in a window"
    await asyncio.sleep(REST - 4)
    assert len(adverts) == in_window, "advertisements arrived during the rest"
    assert ("RUNNING", "PASSIVE") in states, states
    first = adverts[0].advertisements[0]
    assert first.address == 0xA4C138123456 and bytes(first.data)[:3] == b"\x02\x01\x06", first
    unsubscribe()
    await asyncio.sleep(1)
    await client.disconnect()
    return in_window


def main():
    binary = os.path.join(ROOT, "target", "debug", "lumatv-helper")
    if "--bin" in sys.argv:
        binary = sys.argv[sys.argv.index("--bin") + 1]
    with tempfile.TemporaryDirectory() as tmp:
        data = os.path.join(tmp, "data")
        os.makedirs(data)
        luna = os.path.join(tmp, "luna-send")
        with open(luna, "w") as f:
            f.write(FAKE_LUNA.replace("$DIR", tmp))
        os.chmod(luna, 0o755)
        with open(os.path.join(data, "helper.json"), "w") as f:
            json.dump({"luna": {"send": luna}, "screen": {"source": "on"}, "listen": {"port": 0}}, f)
        linked = json.loads(
            run([binary, "ble-link", "--data-dir", data, "--port", str(PORT), "--scan-seconds", "5", "--rest-seconds", str(REST)])
        )
        # 5 s is the shortest window the limits allow; the script's own window is that.
        global WINDOW
        WINDOW = 5
        host, key = linked["host"], linked["key"]
        helper = subprocess.Popen([binary, "run", "--data-dir", data, "--stderr"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(100):
                time.sleep(0.1)
                if os.path.exists(os.path.join(data, "log-helper")) and "ble: listening" in open(os.path.join(data, "log-helper")).read():
                    break
            else:
                raise SystemExit("smoke-ble: the helper never listened")
            count = asyncio.run(exercise(host, key))
            print(f"smoke-ble: {count} advertisements in one window, none in the rest")
            # A scan that is on when the helper is killed does not outlive it.
            task = subprocess.Popen([sys.executable, "-c", "import asyncio,sys;from aioesphomeapi import APIClient\n"
                "async def m():\n c=APIClient(sys.argv[1],int(sys.argv[2]),None,noise_psk=sys.argv[3]);await c.connect(login=True);"
                "c.subscribe_bluetooth_le_raw_advertisements(lambda a:None);await asyncio.sleep(30)\nasyncio.run(m())",
                host, str(PORT), key])
            pid_file = os.path.join(tmp, "pid")
            before = open(pid_file).read()

            def alive(pid):
                return os.path.exists(f"/proc/{pid}") and ") Z " not in open(f"/proc/{pid}/stat").read()

            # The next window opens when the rest the first one owed is over.
            child = None
            for _ in range(300):
                time.sleep(0.1)
                now = open(pid_file).read()
                if now != before and alive(int(now)):
                    child = int(now)
                    break
            assert child, "a second window never opened"
            os.kill(helper.pid, signal.SIGKILL)
            helper.wait()
            time.sleep(1)
            still = alive(child)
            task.kill()
            assert not still, "the scan child outlived a killed helper"
            print("smoke-ble: a killed helper took its scan with it")
        finally:
            if helper.poll() is None:
                helper.send_signal(signal.SIGTERM)
                helper.wait(timeout=10)


if __name__ == "__main__":
    main()
