#!/usr/bin/env python3
"""Render independent LG UI scenes through their host runners."""
import argparse
import glob
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
sys.path.insert(0, os.path.join(ROOT, 'tests'))
sys.path.insert(0, HERE)

READY = 7000
SCENES = {}
META = {}
LOAD_ERRORS = []


def scene(name, runner=False, materials=('dark', 'light')):
    def register(fn):
        if name in SCENES:
            raise ValueError('scene %r is registered twice' % name)
        SCENES[name] = fn
        META[name] = {'runner': runner, 'materials': tuple(materials)}
        return fn
    return register


def png_path(out_dir, name, material):
    return os.path.join(out_dir, name + ('' if material == 'dark' else '-' + material) + '.png')


def run_scene(name, out_dir, material='dark', theme=None, audit=False):
    if not META[name]['runner']:
        raise ValueError('scene %r has no independent runner' % name)
    return SCENES[name](out_dir, material)


def load_modules(directory=HERE):
    sys.modules.setdefault('scenes', sys.modules[__name__])
    for path in sorted(glob.glob(os.path.join(directory, 'scenes_*.py'))):
        name = os.path.splitext(os.path.basename(path))[0]
        try:
            spec = importlib.util.spec_from_file_location(name, path)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            sys.modules[name] = module
        except Exception as error:
            LOAD_ERRORS.append('%s: %s: %s' % (name, type(error).__name__, error))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--out', default=os.path.join(ROOT, 'docs', 'mocks', 'host'))
    ap.add_argument('--only', help='comma-separated scene names')
    ap.add_argument('--theme', choices=['dark', 'light', 'both'], default='both')
    ap.add_argument('--dark', action='store_true')
    ap.add_argument('--light', action='store_true')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--audit', action='store_true')
    args = ap.parse_args()
    load_modules()
    for error in LOAD_ERRORS:
        print('scene module failed: ' + error, file=sys.stderr)
    if args.list:
        print('\n'.join(SCENES))
        return int(bool(LOAD_ERRORS))
    materials = [m for m, on in (('dark', args.dark or args.theme == 'dark'), ('light', args.light or args.theme == 'light')) if on] or ['dark', 'light']
    os.makedirs(args.out, exist_ok=True)
    bad = len(LOAD_ERRORS)
    for name in args.only.split(',') if args.only else SCENES:
        if name not in SCENES:
            print('%-34s unknown scene' % name)
            bad += 1
            continue
        for material in (m for m in materials if m in META[name]['materials']):
            rc, warnings, _ = run_scene(name, args.out, material, audit=args.audit)
            print('%-34s %-5s %s' % (name, material, 'ok' if rc == 0 else 'FAILED (%d warnings)' % len(warnings)), flush=True)
            for warning in warnings[:40]:
                print('   ' + warning[:240])
            bad += rc != 0
    return int(bool(bad))


if __name__ == '__main__':
    sys.exit(main())
