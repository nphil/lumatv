"""Round 8 TV slot helpers (workstation side). Evidence under OUT; every call logs to OUT/log.txt."""
import json, os, re, subprocess, sys, time, urllib.request
import paramiko

TV = "192.168.1.242"
APP = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv"
TOOL = APP + "/tools/lumatv"
HA = "http://192.168.1.146"
AVR = "media_player.living_room_avr"
OUT = os.environ.get("R8_OUT", "/data/home/lumaglass/docs/research/round8/tv-2026-10-25")
os.makedirs(OUT, exist_ok=True)
_log = open(os.path.join(OUT, "log.txt"), "a")


def log(msg):
    line = "%s %s" % (time.strftime("%H:%M:%S"), msg)
    print(line, flush=True)
    _log.write(line + "\n"); _log.flush()


def _client():
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(TV, username="root", password="alpine", timeout=10, banner_timeout=20, auth_timeout=20, look_for_keys=False, allow_agent=False)
    return c


def tv(cmd, timeout=120):
    c = _client()
    try:
        _, out, err = c.exec_command(cmd, timeout=timeout)
        o = out.read().decode("utf-8", "replace"); e = err.read().decode("utf-8", "replace")
        return o + (("\n[stderr]\n" + e) if e.strip() else "")
    finally:
        c.close()


def pty(cmd, timeout=600):
    c = _client()
    try:
        _, out, _ = c.exec_command(cmd, timeout=timeout, get_pty=True)
        return out.read().decode("utf-8", "replace").replace("\r", "")
    finally:
        c.close()


def put(local, remote, mode=None):
    t = paramiko.Transport((TV, 22)); t.connect(username="root", password="alpine")
    s = paramiko.SFTPClient.from_transport(t); s.put(local, remote)
    if mode is not None:
        s.chmod(remote, mode)
    s.close(); t.close()


def get(remote, local):
    t = paramiko.Transport((TV, 22)); t.connect(username="root", password="alpine")
    s = paramiko.SFTPClient.from_transport(t); s.get(remote, local); s.close(); t.close()


def shot(name, w=960, h=540):
    """A DISPLAY capture to OUT/<name>.png; returns the local path (about 1 s on the set)."""
    local = os.path.join(OUT, name + ".png")
    for attempt in range(3):
        out = tv("rm -f /tmp/r8shot.png; luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{\"path\":\"/tmp/r8shot.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":%d,\"height\":%d}'" % (w, h), 30)
        if '"returnValue":true' in out:
            get("/tmp/r8shot.png", local)
            log("shot %s" % name)
            return local
        log("shot %s: capture answered %s (attempt %d)" % (name, out.strip()[:200], attempt + 1))
        time.sleep(1.5)
    return None


def key(tokens, timeout=60):
    """kb.py tokens on the set; returns its output (epoch stamps when it prints them)."""
    return tv("python3 /tmp/kb.py %s" % tokens, timeout)


def luna(uri, payload="{}", n=1, timeout=30):
    return tv("luna-send -n %d -f %s '%s'" % (n, uri, payload), timeout)


def jl(text):
    """The first JSON object in a luna-send answer."""
    m = re.search(r"\{.*\}", text, re.S)
    try:
        return json.loads(m.group(0)) if m else {}
    except ValueError:
        return {}


def fg():
    return jl(luna("luna://com.webos.applicationManager/getForegroundAppInfo")).get("appId", "?")


def power():
    return jl(luna("luna://com.webos.service.tvpower/power/getPowerState")).get("state", "?")


def picture_mode():
    m = re.search(r'"pictureMode"\s*:\s*"([^"]+)"', luna("luna://com.webos.settingsservice/getSystemSettings", '{"category":"picture","keys":["pictureMode"]}'))
    return m.group(1) if m else "?"


def launch(app_id, params=None):
    p = {"id": app_id}
    if params:
        p["params"] = params
    return luna("luna://com.webos.applicationManager/launch", json.dumps(p))


def close_app(app_id):
    return luna("luna://com.webos.applicationManager/closeByAppId", json.dumps({"id": app_id}))


def home(wait=6):
    launch("com.webos.app.home"); time.sleep(wait)
    return fg()


def log_since(pattern, since_mark=None, n=40):
    """Lines of /var/log/messages matching the regex (grep -E), the last n."""
    return tv("grep -aE '%s' /var/log/messages | tail -n %d" % (pattern, n), 30)


def log_lines():
    return int(tv("wc -l < /var/log/messages").strip() or 0)


def log_tail_from(line_no, pattern=None):
    cmd = "tail -n +%d /var/log/messages" % (line_no + 1)
    if pattern:
        cmd += " | grep -aE '%s'" % pattern
    return tv(cmd, 30)


# ---- the receiver through Home Assistant -----------------------------------------------------------------------------

def ha_token():
    subprocess.run(["sh", "/data/home/tmp/ha-token-fetch.sh"], capture_output=True)
    return open("/data/home/tmp/ha-token").read().strip()


def ha_state():
    req = urllib.request.Request(HA + "/api/states/" + AVR, headers={"Authorization": "Bearer " + ha_token()})
    j = json.loads(urllib.request.urlopen(req, timeout=10).read())
    a = j.get("attributes", {})
    return {"state": j.get("state"), "volume_level": a.get("volume_level"), "is_volume_muted": a.get("is_volume_muted"),
            "sound_mode": a.get("sound_mode"), "source": a.get("source")}


def ha_call(service, data):
    req = urllib.request.Request(HA + "/api/services/media_player/" + service, data=json.dumps(dict(data, entity_id=AVR)).encode(),
                                 headers={"Authorization": "Bearer " + ha_token(), "Content-Type": "application/json"})
    return urllib.request.urlopen(req, timeout=15).read()


def helper_receiver():
    try:
        return json.loads(tv("curl -s -m 5 http://127.0.0.1:18731/v1/receiver", 15)).get("value", {})
    except Exception:
        return {}


def avr_mute(on):
    ha_call("volume_mute", {"is_volume_muted": bool(on)})
    time.sleep(2)
    r = helper_receiver()
    log("receiver mute %s -> helper reads mute=%s vol=%s" % (on, r.get("mute"), r.get("volumeText")))
    return r
