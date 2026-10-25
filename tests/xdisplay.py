"""One X display for the tests that need the offline QML host, started once and stopped at exit.

`DISPLAY` from the environment is used as it is; otherwise an Xvfb is started on a free number.
`DISPLAY` is None when neither exists, and the tests that need it skip.
"""
import atexit
import os
import shutil
import subprocess
import time


def _start():
    if os.environ.get("DISPLAY"):
        return os.environ["DISPLAY"]
    if not shutil.which("Xvfb"):
        return None
    for n in range(90, 120):
        if os.path.exists(f"/tmp/.X11-unix/X{n}"):
            continue
        p = subprocess.Popen(["Xvfb", f":{n}", "-screen", "0", "1920x1080x24", "-nolisten", "tcp"],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(50):
            if os.path.exists(f"/tmp/.X11-unix/X{n}"):
                atexit.register(p.kill)
                return f":{n}"
            time.sleep(0.1)
        p.kill()
    return None


DISPLAY = _start()
try:
    import PyQt5  # noqa: F401
    HAVE_QT = True
except ImportError:
    HAVE_QT = False
