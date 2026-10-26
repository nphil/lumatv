"""One X display for the tests that need the offline QML host, started once and stopped at exit.

`DISPLAY` from the environment is used as it is; otherwise an Xvfb is started on a display number it chooses.
`DISPLAY` is None when neither exists, and the tests that need it skip.
"""
import atexit
import os
import shutil
import subprocess


def _stop(p):
    p.kill()
    p.wait()


def _start():
    if os.environ.get("DISPLAY"):
        return os.environ["DISPLAY"]
    if not shutil.which("Xvfb"):
        return None
    # Xvfb picks the number itself (-displayfd): two test runs started together on one machine each get their own server. Choosing a
    # free number from here is a race, and a run that exits kills the server of another that had picked the same one.
    r, w = os.pipe()
    p = subprocess.Popen(["Xvfb", "-displayfd", str(w), "-screen", "0", "1920x1080x24", "-nolisten", "tcp"],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, pass_fds=(w,))
    os.close(w)
    atexit.register(_stop, p)
    with os.fdopen(r) as f:
        number = f.readline().strip()               # empty when Xvfb ended without a display
    if not number.isdigit():
        return None
    return ":" + number


DISPLAY = _start()
try:
    import PyQt5  # noqa: F401
    HAVE_QT = True
except ImportError:
    HAVE_QT = False
