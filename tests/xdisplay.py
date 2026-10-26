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


def own_tmp(case):
    """A /tmp of this test's own for the QML host (`QMLHOST_TMP`: its `/tmp/<name>` is `<dir>/<name>`), for the compositor's doors that other
    runs on the machine write at the real path. Returns the directory; the environment is put back when the test ends."""
    import tempfile
    d = tempfile.mkdtemp(prefix="qmlhost-tmp-")
    before = os.environ.get("QMLHOST_TMP")
    os.environ["QMLHOST_TMP"] = d

    def restore():
        shutil.rmtree(d, ignore_errors=True)
        if before is None:
            os.environ.pop("QMLHOST_TMP", None)
        else:
            os.environ["QMLHOST_TMP"] = before
    case.addCleanup(restore)
    return d
