"""10 s at 10 fps, two colours alternating every 2 s (dim red, dim blue), camera snapshots during; the bulb put back as found."""
import json, sys, time, urllib.request
sys.path.insert(0, "/tmp/r8")
import z2mframe as z

HA = "http://192.168.1.146"; TOKEN = open("/data/home/tmp/ha-token").read().strip()
NAME = "Plant Room Fan Light"; ADDR = 65525
SET = "zigbee2mqtt/%s/set" % NAME
OUT = "/data/home/lumaglass/docs/research/round8/tv-2026-10-25"
FPS = 10; SECONDS = 10
RED = (12000, 0, 0); BLUE = (0, 0, 14000)      # 16-bit, about 15 % brightness after Bifrost's conversion
FOUND = {"brightness": 1, "color_temp": 366}
out = open(OUT + "/light-test.txt", "a")


def say(s):
    line = "%s %s" % (time.strftime("%H:%M:%S"), s); print(line, flush=True); out.write(line + "\n"); out.flush()


def publish(topic, payload):
    req = urllib.request.Request(HA + "/api/services/mqtt/publish", data=json.dumps({"topic": topic, "payload": json.dumps(payload)}).encode(),
                                 headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=10).read()


def snapshot(name):
    req = urllib.request.Request(HA + "/api/camera_proxy/camera.plant_room_cat_feeder", headers={"Authorization": "Bearer " + TOKEN})
    data = urllib.request.urlopen(req, timeout=20).read()
    open("%s/%s.jpg" % (OUT, name), "wb").write(data)
    say("camera %s: %d bytes" % (name, len(data)))


say("second check: %d s at %d fps, red/blue every 2 s; records red %s blue %s" % (SECONDS, FPS, z.record(ADDR, RED).hex(), z.record(ADDR, BLUE).hex()))
publish(SET, z.light_balance()); time.sleep(0.3)
counter = 0
publish(SET, z.zclcommand(3, z.reset(counter))); time.sleep(0.5)
smoothing = z.smoothing_for_ms(1000 // FPS)
t0 = time.time(); late = 0
for i in range(FPS * SECONDS):
    rgb = RED if (i // (2 * FPS)) % 2 == 0 else BLUE
    publish(SET, z.zclcommand(1, z.frame(counter, smoothing, [z.record(ADDR, rgb)]))); counter += 1
    if i == 3 * FPS:
        snapshot("light-cam-during-red")
    if i == 7 * FPS:
        snapshot("light-cam-during-blue")
    due = t0 + (i + 1) / FPS; now = time.time()
    if now < due:
        time.sleep(due - now)
    else:
        late += 1
say("sent %d frames in %.1f s, %d late" % (counter, time.time() - t0, late))
black = z.record(ADDR, (0, 0, 0))
for _ in range(2):
    publish(SET, z.zclcommand(1, z.frame(counter, smoothing, [black]))); counter += 1; time.sleep(0.1)
publish(SET, z.zclcommand(3, z.reset(counter))); time.sleep(0.3)
publish(SET, {"state": "ON", "brightness": FOUND["brightness"], "color_temp": FOUND["color_temp"]}); time.sleep(1.0)
publish(SET, {"state": "OFF"}); time.sleep(1.5)
snapshot("light-cam-after")
say("bulb told: ON brightness 1 color_temp 366, then OFF")
