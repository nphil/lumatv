"""Scenes of LumaTV's own on-screen keyboard (panels/Keyboard.qml, drawn by the shell client over an app): the letters, the symbols and the
number pad, dark and light, at 100 % and at 200 % text (`-200`), over a still of video, which is what the keyboard is drawn over on the set.

    python3 tests/qmlhost/scenes.py --only keyboard-letters,keyboard-symbols,keyboard-numbers --out docs/mocks/keyboard

Not Home's surfaces: the shell client (compositor/shell/Shell.qml) runs against the host's stand-in of the compositor's channel
(tests/qmlhost/shellhost.py), the compositor "activates" a field by publishing the `ime` topic and "types" on the remote by sending `imekey`
events, exactly as server/ime.rs does. Each render also runs the layout audit of the finished picture (words cut, outside their key, on top of
each other, too faint): every kind at 100 %, the universal one (overlap) at 200 %.
"""
import json
import os
import subprocess
import sys

from scenes import scene, png_path, ROOT, HERE, merge, doubled_type
import xdisplay

STILL = os.path.join(ROOT, "docs", "mocks", "img", "still.jpg")
HOST = os.path.join(HERE, "shellhost.py")
READY = 7000
APP = {"app": "netflix", "home": False}

# What the compositor tells the shell about a field (`ime` topic, server/ime.rs `own_publish`): a search box, a field for an e-mail address and a PIN.
FIELD = {"active": True, "show": True, "model": 3, "app": "com.webos.app.voice", "hint": 7, "purpose": 0, "enter": 5, "max": 0, "text": False}
NUMBER = dict(FIELD, purpose=3, enter=2, hint=0)

# Where the remote takes the focus from the first key (the middle of the letters) to the one a render looks at
TO_SYMBOLS = ["up", "up"] + ["left"] * 5 + ["ok"]                 # the 5 of the digit row, along it to ?123, and OK: the symbols
IN_SYMBOLS = ["down"] + ["right"] * 4                              # then down one row and along to the dollar sign


def press(at, name):
    """Two `imekey` pushes: the key down and up, as the compositor sends them."""
    return ["--push", "%d:imekey:%s" % (at, json.dumps({"k": name, "d": True, "r": False})),
            "--push", "%d:imekey:%s" % (at + 40, json.dumps({"k": name, "d": False, "r": False}))]


def render(name, out_dir, material, field, keys, text=100, voice=False):
    """One picture of the system layer with the keyboard up for `field`, after the remote pressed `keys`. Returns (exit code, warnings, output)."""
    path = png_path(out_dir, name + ("" if text == 100 else "-200"), material)
    audit = path[:-4] + ".layout.json"
    for p in (path, audit):
        if os.path.exists(p):
            os.remove(p)
    theme = {"perf": {"syncResidents": True}, "material": material}
    if text == 200:
        theme = merge(theme, doubled_type())
    start = READY + 800
    args = [sys.executable, HOST, "--backdrop", STILL, "--theme", json.dumps(theme),
            "--push", "500:foreground:" + json.dumps(APP), "--push", "500:screen:" + json.dumps({"on": True}),
            "--push", "%d:ime:%s" % (start, json.dumps(field))]
    if voice:
        # the TV says it has voice recognition (the shell asks `com.webos.service.config` once its channel is up): the microphone key is offered
        args += ["--luna", "1500:/getConfigs:" + json.dumps({"returnValue": True, "configs": {"system.supportVoiceRecognition": True}})]
    at = start + 700
    for k in keys:
        args += press(at, k)
        at += 140
    end = at + 900
    args += ["--snap-sys", "%d:%s" % (end, path), "--audit-sys", "%d:%s%s" % (end, audit, "" if text == 100 else ":overlap"), "--wait", str(end + 500)]
    env = dict(os.environ, DISPLAY=xdisplay.DISPLAY, QT_QPA_PLATFORM="offscreen")
    r = subprocess.run(args, env=env, capture_output=True, text=True, timeout=300)
    out = r.stdout + r.stderr
    warn = [ln for ln in out.splitlines() if (ln.startswith("[warning]") or ln.startswith("[critical]")) and "Implicitly defined onFoo" not in ln
            and "QSslSocket" not in ln and "OpenSSL" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    if os.path.exists(audit):
        import layoutaudit
        with open(audit) as f:
            warn += layoutaudit.lines(json.load(f), "layout" if text == 100 else "layout 200%")
        os.remove(audit)
    else:
        warn.append("[audit] not written: %s" % audit)
    return (r.returncode or (1 if warn else 0)), warn, out


SCENES_OF = (("keyboard-letters", FIELD, [], False, (100, 200)), ("keyboard-symbols", FIELD, TO_SYMBOLS + IN_SYMBOLS, False, (100, 200)),
             ("keyboard-numbers", NUMBER, [], False, (100, 200)), ("keyboard-letters-voice", FIELD, [], True, (100,)))
for _name, _field, _keys, _voice, _sizes in SCENES_OF:
    for _text in _sizes:
        def _make(name=_name, field=_field, keys=_keys, voice=_voice, text=_text):
            @scene(name + ("" if text == 100 else "-200"), runner=True)
            def _(out_dir, material):
                return render(name, out_dir, material, field, keys, text, voice)
        _make()
