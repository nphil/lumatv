"""Python port of helper/app/src/ambient_z2m_frame.rs (Bifrost's arithmetic, same order), checked against the fixture."""
import json, struct

CLUSTER = 0xFC01
VENDOR = 0x100B
RGB_TO_XYZ = [0.7164, 0.1010, 0.1468, 0.2587, 0.7247, 0.0166, 0.0000, 0.0512, 0.7740]
XYZ_TO_RGB = [1.4625, -0.1845, -0.2734, -0.5229, 1.4479, 0.0681, 0.0346, -0.0958, 1.2875]
MAX_X, MAX_Y = 0.7347, 0.8264
D50 = (0.34567, 0.35850)
EPS = 2.220446049250313e-16
MODE_DEVICE, MODE_SEGMENT = 0b01011, 0b00000


def mult(m, d):
    return [d[0] * m[0] + d[1] * m[1] + d[2] * m[2],
            d[0] * m[3] + d[1] * m[4] + d[2] * m[5],
            d[0] * m[6] + d[1] * m[7] + d[2] * m[8]]


def find_maximum_y(x, y):
    bri = 1.0
    for _ in range(10):
        z = 1.0 - x - y
        r, g, b = mult(XYZ_TO_RGB, [(bri / y) * x, bri, (bri / y) * z])
        bri /= max(r, g, b)
    return bri


def xy_of(red, green, blue):
    cx, cy, cz = mult(RGB_TO_XYZ, [red / 255.0, green / 255.0, blue / 255.0])
    s = cx + cy + cz
    if s == 0:
        return D50[0], D50[1], 0.0
    x, y = cx / s, cy / s
    max_y = find_maximum_y(x, y)
    if max_y > EPS:
        return x, y, min(cy / max_y * 255.0, 255.0)
    return D50[0], D50[1], 0.0


def quantise(x, y):
    xq = int((x * 0xFFF) / MAX_X)
    yq = int((y * 0xFFF) / MAX_Y)
    return bytes([xq & 0xFF, ((xq >> 8) & 0x0F) | ((yq & 0x0F) << 4), (yq >> 4) & 0xFF])


def record(addr, rgb16, mode=MODE_DEVICE):
    x, y, bright = xy_of(rgb16[0] // 256, rgb16[1] // 256, rgb16[2] // 256)
    brightness = int(min(max(bright / 255.0 * 2047.0, 1.0), 2047.0))
    field = (min(brightness, 2047) << 5) | mode
    return struct.pack("<HH", addr, field) + quantise(x, y)


def frame(counter, smoothing, records):
    return struct.pack("<IH", counter, smoothing) + b"".join(records)


def reset(counter):
    return b"\x00\x01" + struct.pack("<I", counter)


def smoothing_for_ms(ms):
    us = min(ms * 1000, 2_560_000 - 1)
    return (us * 0x10000 // 2_560_000) & 0xFFFF


def zclcommand(command, data):
    return {"zclcommand": {"cluster": CLUSTER, "command": command, "payload": {"data": list(data)}, "frametype": 1,
                           "options": {"manufacturerCode": VENDOR, "disableDefaultResponse": True, "direction": 0, "timeout": 100.0}}}


def light_balance(value=0xFE):
    return {"write": {"cluster": CLUSTER, "payload": {"5": {"manufacturerCode": VENDOR, "type": 0x20, "value": value}}}}


def check_fixture(path):
    bad = 0
    for line in open(path):
        v = json.loads(line)
        got = record(0xD297, v["rgb"]).hex()
        if got != v["device"]:
            bad += 1
            print("MISMATCH", v["rgb"], got, v["device"])
    return bad


if __name__ == "__main__":
    import sys
    print("mismatches:", check_fixture(sys.argv[1]))
