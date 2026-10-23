#!/usr/bin/env python3
"""Drive scripted workloads on the set and capture a frame trace for each.

    run.py [--tag TAG] [--repeat N] [--workloads idle,dpad,qs,apphome,video]
           [--apply CMD --revert CMD]

Needs: paramiko; the HA token in /data/home/tmp/ha-token (fetched by
/data/home/tmp/ha-token-fetch.sh); capture.sh at /tmp/perf/capture.sh on
the set. Traces and analyses land in /tmp/perf on the workstation as
<tag>-<workload>-<n>.{trace,sys,json,txt}; a summary table is printed and
appended to /tmp/perf/summary.txt.

Workloads (phase markers in brackets):
  idle     Home still for 10 s                              [idle]
  dpad     8 x RIGHT, 8 x LEFT, UP, DOWN at 350 ms on the dock [dpad]
  qs       Quick Settings open (MENU) and close (BACK), 3 x [qs-open-N, qs-close-N]
  apphome  LG Settings app launched, then HOME, 3 x          [app-N, home-N]
  video    YouTube playing, HOME over it, d-pad over video   [video-idle, video-dpad]
"""
import argparse
import json
import os
import subprocess
import sys
import time
import urllib.request

import paramiko

TV = ('192.168.1.242', 'root', 'alpine')
HA = 'http://192.168.1.146'
LOCAL = '/tmp/perf'
HERE = os.path.dirname(os.path.abspath(__file__))


class Set:
    def __init__(self):
        self.cli = None
        self.mk = None
        with open('/data/home/tmp/ha-token') as f:
            self.tok = f.read().strip()
        self.connect()
        self.sm_pid = int(self.sh('pidof surface-manager')[1].strip())

    def connect(self):
        c = paramiko.SSHClient()
        c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
        c.connect(TV[0], username=TV[1], password=TV[2], timeout=20, look_for_keys=False, allow_agent=False)
        c.get_transport().set_keepalive(15)
        self.cli = c
        self.mk = None

    def sh(self, cmd, timeout=120, pty=False):
        for attempt in range(3):
            try:
                if not self.cli.get_transport() or not self.cli.get_transport().is_active():
                    self.connect()
                _, out, err = self.cli.exec_command(cmd, timeout=timeout, get_pty=pty)
                o = out.read().decode('utf-8', 'replace')
                e = err.read().decode('utf-8', 'replace')
                return out.channel.recv_exit_status(), o, e
            except Exception as ex:  # channel closed, reconnect
                print('ssh retry', ex, file=sys.stderr)
                self.connect()
        raise RuntimeError('ssh failed')

    def luna(self, uri, payload='{}'):
        return self.sh(f"luna-send -n 1 {uri} '{payload}'", pty=True)[1].strip()

    def mark(self, text):
        if self.mk is None or self.mk.closed:
            self.mk = self.cli.invoke_shell(width=200)
            self.mk.send("export PS1=''; stty -echo 2>/dev/null\n")
            time.sleep(0.5)
            while self.mk.recv_ready():
                self.mk.recv(65536)
        self.mk.send(f"echo '{text}' > /sys/kernel/debug/tracing/trace_marker\n")

    def key(self, button):
        req = urllib.request.Request(f'{HA}/api/services/webostv/button',
                                     data=json.dumps({'entity_id': 'media_player.living_room_tv', 'button': button, 'wait': False}).encode(),
                                     headers={'Authorization': f'Bearer {self.tok}', 'Content-Type': 'application/json'}, method='POST')
        with urllib.request.urlopen(req, timeout=15) as r:
            r.read()

    def keys(self, button, n, gap):
        for i in range(n):
            self.key(button)
            time.sleep(gap)

    def get(self, remote, local):
        s = self.cli.open_sftp()
        s.get(remote, local)
        s.close()

    def capture(self, name, mode='full'):
        rc, out, err = self.sh(f'/tmp/perf/capture.sh start {name} {mode}')
        if err.strip():
            print('capture start:', err.strip(), file=sys.stderr)

    def stop(self, name):
        self.sh(f'/tmp/perf/capture.sh stop {name}', timeout=180)
        os.makedirs(LOCAL, exist_ok=True)
        self.get(f'/tmp/perf/{name}.trace', f'{LOCAL}/{name}.trace')
        self.get(f'/tmp/perf/{name}.sys', f'{LOCAL}/{name}.sys')
        self.sh(f'rm -f /tmp/perf/{name}.trace /tmp/perf/{name}.sys')

    def foreground(self):
        return self.luna('luna://com.webos.applicationManager/getForegroundAppInfo')


# ----------------------------------------------------------------- workloads

def wl_idle(s):
    s.mark('P:idle')
    time.sleep(10)


def wl_dpad(s):
    s.mark('P:dpad')
    s.keys('RIGHT', 8, 0.35)
    s.keys('LEFT', 8, 0.35)
    s.key('UP'); time.sleep(0.5)
    s.key('DOWN'); time.sleep(0.5)
    s.mark('P:settle')
    time.sleep(1)


def wl_qs(s):
    for i in range(3):
        s.mark(f'P:qs-open-{i}')
        s.key('MENU')
        time.sleep(2.5)
        s.mark(f'P:qs-close-{i}')
        s.key('BACK')
        time.sleep(2.5)


def wl_apphome(s, app='com.palm.app.settings'):
    for i in range(3):
        s.mark(f'P:app-{i}')
        s.luna('luna://com.webos.applicationManager/launch', json.dumps({'id': app}))
        time.sleep(7)
        s.mark(f'P:home-{i}')
        s.key('HOME')
        time.sleep(4)
    s.luna('luna://com.webos.applicationManager/closeByAppId', json.dumps({'id': app}))
    time.sleep(1)


def wl_video(s, video='https://www.youtube.com/tv?v=LXb3EKWsInQ'):
    s.luna('luna://com.webos.applicationManager/launch',
           json.dumps({'id': 'youtube.leanback.v4', 'params': {'contentTarget': video}}))
    time.sleep(15)
    s.mark('P:video-home')
    s.key('HOME')
    time.sleep(1)
    s.mark('P:video-idle')
    time.sleep(6)
    s.mark('P:video-dpad')
    s.keys('RIGHT', 5, 0.35)
    s.keys('LEFT', 5, 0.35)
    time.sleep(1)
    s.mark('P:video-end')
    s.luna('luna://com.webos.applicationManager/closeByAppId', json.dumps({'id': 'youtube.leanback.v4'}))
    time.sleep(2)


WORKLOADS = {'idle': wl_idle, 'dpad': wl_dpad, 'qs': wl_qs, 'apphome': wl_apphome, 'video': wl_video}
GAP = {'idle': 34, 'dpad': 17.5, 'qs': 17.5, 'apphome': 17.5, 'video': 34}


def analyse(name, sm_pid, gap):
    r = subprocess.run([sys.executable, os.path.join(HERE, 'analyse.py'), f'{LOCAL}/{name}.trace',
                        '--sm', str(sm_pid), '--gap', str(gap), '--json', f'{LOCAL}/{name}.json', '--top', '6'],
                       capture_output=True, text=True)
    with open(f'{LOCAL}/{name}.txt', 'w') as f:
        f.write(r.stdout + r.stderr)
    with open(f'{LOCAL}/{name}.json') as f:
        return json.load(f)


def summary_rows(name, rep):
    rows = []
    for w in rep['windows']:
        rows.append(f"{name:28s} {w['name']:14s} {w['dur_s']:6.2f}s {w['frames']:5d}f {w['fps']:5.1f}fps >17:{w['over17']:3d} >33:{w['over33']:3d} >50:{w['over50']:3d} worst {w['worst_ms']:6.1f}ms  smrun {w['sm_run_ms']:7.1f}ms lat {w['sm_max_sched_latency_ms']:5.2f} gpu {w['gpu_busy_ms']:7.1f}ms hp {w['cpuhp']}")
    return rows


def run(s, tag, workloads, repeat, apply=None, revert=None):
    rows = []
    if apply:
        print('apply:', s.sh(apply)[1].strip())
    try:
        for n in range(repeat):
            for wl in workloads:
                name = f'{tag}-{wl}-{n}'
                s.capture(name, 'full')
                time.sleep(1)
                try:
                    WORKLOADS[wl](s)
                finally:
                    s.stop(name)
                rep = analyse(name, s.sm_pid, GAP[wl])
                rows += summary_rows(name, rep)
                print('\n'.join(rows[-len(rep['windows']):]))
    finally:
        if revert:
            print('revert:', s.sh(revert)[1].strip())
    with open(f'{LOCAL}/summary.txt', 'a') as f:
        f.write('\n'.join(rows) + '\n')
    return rows


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--tag', default='base')
    ap.add_argument('--repeat', type=int, default=3)
    ap.add_argument('--workloads', default='idle,dpad,qs,apphome')
    ap.add_argument('--apply')
    ap.add_argument('--revert')
    a = ap.parse_args()
    s = Set()
    run(s, a.tag, a.workloads.split(','), a.repeat, a.apply, a.revert)
