import re, sys
"""The close of Plex, from a messages file: the lines of the SDL2 INFO stream and SAM's, as ms after SAM's APP_CLOSE (the closeByAppId call)."""
def timeline(path, lo=None, hi=None, pid=None):
    lines = open(path, errors='replace').read().splitlines()
    ev = []
    for l in lines:
        m = re.search(r'\[(\d+\.\d+)\] user\.\w+ (\S+) \[\] (.*)', l)
        if not m: continue
        t = float(m.group(1)); who = m.group(2); rest = m.group(3)
        if lo and t < lo: continue
        if hi and t > hi: continue
        if who == 'plxnative' and 'SDL2 INFO' in rest:
            msg = re.sub(r'^SDL2 INFO \{\} ', '', rest)
            if re.search(r'SDL_Quit|Destroy wayland|WebOS_EGL|Unload|forceQuit|cb_RegistNativeApp|SetRelativeMouse|RegistNative|Quit', msg):
                ev.append((t, 'plxnative', msg[:110]))
        elif who == 'sam' and re.search(r'APP_CLOSE |APP_CLOSED|start_kill_timer|cancel_kill_timer|FOREGROUND_INFO', rest) and ('plxnative' in rest or 'FOREGROUND_INFO' in rest):
            ev.append((t, 'sam', re.sub(r'^SAM ', '', rest)[:110]))
    return ev
def show(ev, label):
    t0 = next((t for t, w, m in ev if w == 'sam' and m.startswith('APP_CLOSE {') and 'caller' in m), ev[0][0] if ev else 0)
    print('==', label, '(ms after the closeByAppId call reached SAM; monotonic %.3f)' % t0)
    for t, w, m in ev:
        print('  %+9.1f  %-8s %s' % ((t - t0) * 1000, w, m))
if __name__ == '__main__':
    for p in sys.argv[1:]:
        path, _, rng = p.partition(':')
        lo = hi = None
        if rng:
            lo, hi = [float(x) for x in rng.split('-')]
        show(timeline(path, lo, hi), p)
