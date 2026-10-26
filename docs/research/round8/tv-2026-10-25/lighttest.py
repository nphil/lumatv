"""The helper's own Zigbee entertainment frames (ambient_z2m_frame port, checked against the Bifrost vectors) to the Plant Room
Fan Light through Home Assistant's mqtt.publish: dim (about 5-10 % brightness), 25 s, colours sampled from the TV's Home picture.
The bulb is put back as found (OFF, brightness 1, color_temp 366) at the end."""
import json, sys, time, asyncio, urllib.request
import websockets
from PIL import Image
sys.path.insert(0, "/tmp/r8")
import z2mframe as z

HA = "http://192.168.1.146"; TOKEN = open("/data/home/tmp/ha-token").read().strip()
NAME = "Plant Room Fan Light"; ADDR = 65525
SET = "zigbee2mqtt/%s/set" % NAME
FPS = 20; SECONDS = 25; DIM = 0.3
FOUND = {"state": "OFF", "brightness": 1, "color_temp": 366}
out = open("/data/home/lumaglass/docs/research/round8/tv-2026-10-25/light-test.txt", "a")


def say(s):
    line = "%s %s" % (time.strftime("%H:%M:%S"), s); print(line, flush=True); out.write(line + "\n"); out.flush()


def publish(topic, payload):
    req = urllib.request.Request(HA + "/api/services/mqtt/publish", data=json.dumps({"topic": topic, "payload": json.dumps(payload)}).encode(),
                                 headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=10).read()


def ha_light():
    req = urllib.request.Request(HA + "/api/states/light.plant_room_fan_light", headers={"Authorization": "Bearer " + TOKEN})
    j = json.loads(urllib.request.urlopen(req, timeout=10).read())
    a = j["attributes"]
    return j["state"], {k: a.get(k) for k in ("brightness", "color_mode", "xy_color", "color_temp_kelvin")}


def picture_colours(path, n):
    """n colours sampled left to right across the middle band of the TV's picture, dimmed."""
    im = Image.open(path).convert("RGB")
    w, h = im.size
    cols = []
    for i in range(n):
        x0 = int(w * i / n); x1 = max(x0 + 1, int(w * (i + 1) / n))
        box = im.crop((x0, int(h * 0.3), x1, int(h * 0.7))).resize((1, 1), Image.BOX).getpixel((0, 0))
        cols.append(tuple(int(c * 257 * DIM) for c in box))
    return cols


async def watch(seconds, sink):
    async with websockets.connect("ws://192.168.1.146/api/websocket", max_size=50_000_000) as ws:
        await ws.recv(); await ws.send(json.dumps({"type": "auth", "access_token": TOKEN})); await ws.recv()
        await ws.send(json.dumps({"id": 3, "type": "mqtt/subscribe", "topic": "zigbee2mqtt/%s" % NAME}))
        end = time.time() + seconds
        while time.time() < end:
            try:
                m = json.loads(await asyncio.wait_for(ws.recv(), 1))
            except asyncio.TimeoutError:
                continue
            if m.get("id") == 3 and m.get("type") == "event":
                sink.append((time.time(), m["event"]["payload"]))


def stream():
    cols = picture_colours("/data/home/lumaglass/docs/research/round8/tv-2026-10-25/01-home-after-apply.png", 40)
    say("as found: HA %s; stored %s" % (ha_light(), FOUND))
    say("start: light balance 0xFE, then command 3 (counter 0)")
    publish(SET, z.light_balance()); time.sleep(0.3)
    counter = 0
    publish(SET, z.zclcommand(3, z.reset(counter))); time.sleep(0.5)
    smoothing = z.smoothing_for_ms(1000 // FPS)
    t0 = time.time(); sent = 0; late = 0
    n = FPS * SECONDS
    for i in range(n):
        rgb = cols[(i // 10) % len(cols)]         # a new picture colour every half second, as a slow pan would give
        rec = z.record(ADDR, rgb)
        publish(SET, z.zclcommand(1, z.frame(counter, smoothing, [rec])))
        counter += 1; sent += 1
        due = t0 + (i + 1) / FPS
        now = time.time()
        if now < due:
            time.sleep(due - now)
        else:
            late += 1
        if i in (40, 200, 400):
            say("frame %d: rgb16 %s record %s; HA reads %s" % (i, rgb, rec.hex(), ha_light()))
    took = time.time() - t0
    say("sent %d frames in %.1f s (%.1f fps asked %d; %d frames late)" % (sent, took, sent / took, FPS, late))
    say("end: black twice, command 3, then the bulb back as found")
    black = z.record(ADDR, (0, 0, 0))
    for _ in range(2):
        publish(SET, z.zclcommand(1, z.frame(counter, smoothing, [black]))); counter += 1; time.sleep(0.05)
    publish(SET, z.zclcommand(3, z.reset(counter))); time.sleep(0.3)
    publish(SET, {"state": "ON", "brightness": FOUND["brightness"], "color_temp": FOUND["color_temp"]}); time.sleep(1.0)
    publish(SET, {"state": "OFF"}); time.sleep(1.5)
    publish("zigbee2mqtt/%s/get" % NAME, {"state": "", "brightness": "", "color": "", "color_temp": ""})
    time.sleep(1.5)
    say("after: HA %s" % (ha_light(),))


if __name__ == "__main__":
    import threading
    seen = []
    t = threading.Thread(target=lambda: asyncio.run(watch(SECONDS + 12, seen)), daemon=True)
    t.start(); time.sleep(1.0)
    stream()
    t.join(timeout=15)
    say("Zigbee2MQTT state messages during the test: %d" % len(seen))
    for ts, p in seen[-6:]:
        say("  %s %s" % (time.strftime("%H:%M:%S", time.localtime(ts)), p[:300]))
