"""One X display for the tests that need the offline QML host, started once and stopped at exit.

`DISPLAY` from the environment is used as it is; otherwise an Xvfb is started on a free number.
`DISPLAY` is None when neither exists, and the tests that need it skip.
"""
import atexit
import os
import shutil
import socket
import subprocess
import time


def _stale(path):
    """A socket left by an Xvfb that is gone: nothing listens on it. (A killed Xvfb leaves its file, and a run of the
    suite used one number each, so after thirty runs there was no free number and every test skipped.)"""
    c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        c.connect(path)
        return False
    except (ConnectionRefusedError, FileNotFoundError):
        return True
    except OSError:
        return False
    finally:
        c.close()


def _stop(p, path):
    p.kill()
    p.wait()
    try:
        os.unlink(path)
    except OSError:
        pass


def _start():
    if os.environ.get("DISPLAY"):
        return os.environ["DISPLAY"]
    if not shutil.which("Xvfb"):
        return None
    for n in range(90, 120):
        path = f"/tmp/.X11-unix/X{n}"
        if os.path.exists(path):
            if not _stale(path):
                continue
            try:
                os.unlink(path)
            except OSError:
                continue
        p = subprocess.Popen(["Xvfb", f":{n}", "-screen", "0", "1920x1080x24", "-nolisten", "tcp"],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(50):
            if os.path.exists(path):
                atexit.register(_stop, p, path)
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
