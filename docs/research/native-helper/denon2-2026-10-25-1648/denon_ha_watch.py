#!/usr/bin/env python3
"""Watches media_player.living_room_avr through the HA API every 5 s for SECS seconds; at 20 s and 35 s steps the
volume up and down through HA (so HA's own link has to carry a command and receive the receiver's event while the second
connection is held). Prints one line per poll. Never prints the token."""
import json, subprocess, sys, time, urllib.request
HA, AVR = "http://192.168.1.146", "media_player.living_room_avr"
secs = int(sys.argv[1]) if len(sys.argv) > 1 else 70
subprocess.run(["sh", "/data/home/tmp/ha-token-fetch.sh"], capture_output=True)
tok = open("/data/home/tmp/ha-token").read().strip()
H = {"Authorization": "Bearer " + tok, "Content-Type": "application/json"}
def state():
    d = json.loads(urllib.request.urlopen(urllib.request.Request(HA + "/api/states/" + AVR, headers=H), timeout=8).read())
    a = d["attributes"]
    return d["state"], a.get("volume_level"), a.get("is_volume_muted"), d["last_updated"][11:23]
def call(svc):
    urllib.request.urlopen(urllib.request.Request(HA + "/api/services/media_player/" + svc, data=json.dumps({"entity_id": AVR}).encode(), headers=H), timeout=10).read()
t0 = time.time(); did = set()
while time.time() - t0 < secs:
    t = time.time() - t0
    for at, svc in ((20, "volume_up"), (35, "volume_down")):
        if t >= at and at not in did:
            did.add(at); call(svc); print("t=%5.1f ha: %s sent" % (t, svc), flush=True)
    try:
        print("t=%5.1f ha: %s" % (t, state()), flush=True)
    except Exception as e:
        print("t=%5.1f ha: read failed %r" % (t, e), flush=True)
    time.sleep(5)
