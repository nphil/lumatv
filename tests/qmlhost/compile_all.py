#!/usr/bin/env python3
"""Compile every QML file of the surface, with the set's import layout, and fail on any error.

    python3 tests/qmlhost/compile_all.py [--src DIR] [--keep DIR]

Each file under payload/compositor/lumatv is staged as the packager stages it and compiled on its own with a QQmlComponent (no window, no
display: Qt's offscreen platform). A syntax error, an unknown type or property, a module the set does not have, an import path that does
not exist or a `LS` that is used without its import all fail here before they fail on the TV, where a surface that does not compile is
a layer that never builds. Nothing is instantiated, so a file that needs its parent's context is compiled, not run; the scenes
(`tests/qmlhost`) run the surfaces.

Exit status 0 when every file compiled, 1 otherwise (the files and their errors are printed), 2 when PyQt5 is not installed.
"""
import argparse
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
SRC = os.path.join(ROOT, 'payload', 'compositor', 'lumatv')

os.environ['QT_QPA_PLATFORM'] = 'offscreen'
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)

try:
    from PyQt5 import QtCore, QtGui, QtQml
except ImportError:
    print('compile_all: PyQt5 is not installed (pip install PyQt5), so the QML cannot be checked', file=sys.stderr)
    sys.exit(2)

sys.path.insert(0, HERE)
import qmlhost  # noqa: E402  (the host's stand-ins: TextFile, the staging)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--src', default=SRC, help='the surface tree to compile (default: payload/compositor/lumatv)')
    ap.add_argument('--keep', help='stage the tree here and leave it (default: a temp dir)')
    a = ap.parse_args(argv)

    data = os.path.abspath(a.keep) if a.keep else tempfile.mkdtemp(prefix='qmlcompile-')
    if a.keep and os.path.exists(data):
        shutil.rmtree(data)
    os.makedirs(data, exist_ok=True)
    mod = qmlhost.stage(data, os.path.abspath(a.src))
    try:
        app = QtGui.QGuiApplication(sys.argv[:1])
        QtQml.qmlRegisterType(qmlhost.TextFile, 'TextFile', 1, 0, 'TextFile')
        eng = QtQml.QQmlEngine()
        eng.addImportPath(os.path.join(HERE, 'stubs'))
        eng.addImportPath(os.path.join(data, 'qml'))
        files = sorted(os.path.join(d, n) for d, _, names in os.walk(mod) for n in names if n.endswith('.qml'))
        failed = 0
        for path in files:
            comp = QtQml.QQmlComponent(eng, QtCore.QUrl.fromLocalFile(path))
            while comp.status() == QtQml.QQmlComponent.Loading:
                app.processEvents()
            if comp.status() != QtQml.QQmlComponent.Ready:
                failed += 1
                print('FAIL %s' % os.path.relpath(path, mod))
                for e in comp.errors():
                    print('  %d:%d %s' % (e.line(), e.column(), e.description()))
        print('%d QML file(s) compiled, %d failed' % (len(files), failed))
        return 1 if failed else 0
    finally:
        if not a.keep:
            shutil.rmtree(data, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
