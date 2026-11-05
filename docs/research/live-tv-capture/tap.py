#!/usr/bin/env python3
"""P1b on the TV (32-bit python3, root): tap the demod's current RF channel through demux2 and read it from dvr2 for 10 s.
One cert write at most, no tune, no luna call. Stops at the first errno from a DVB node or the first error-looking line from
te/sdec/pvr/lxdvb in the kernel log. The seed key never leaves this process (only its SHA-256 prefix is printed)."""
import ctypes, hashlib, json, os, re, select, signal, subprocess, sys, time

DEMUX = '/dev/dvb/adapter0/demux2'
DVR = '/dev/dvb/adapter0/dvr2'
CAPTURE_S = 10.0
TICK_S = 30.0
ARGS = dict(a.split('=', 1) for a in sys.argv[1:] if '=' in a)
PREFER = ARGS.get('prefer', '2.4')      # virtual channel to tap, major.minor
HOLD = float(ARGS.get('hold', '0'))     # seconds to go on reading after the first CAPTURE_S, with streaming checks only
TAG = ARGS.get('tag', 'p1b')
STOPFILE = '/tmp/%s.stop' % TAG         # touch it to end the hold cleanly
RAW = '/tmp/%s-raw.bin' % TAG
TSF = '/tmp/%s.ts' % TAG

DMX_SET_FILTER = 0x403c6f2b
DMX_SET_PES_FILTER = 0x40146f2c
DMX_EXT_S_CTL = 0x40106f35
PVR_EXT_S_CTL = 0x40107000
DMX_CHECK_CRC, DMX_ONESHOT, DMX_IMMEDIATE_START = 1, 2, 4
DMX_OUT_TS_TAP, DMX_PES_OTHER, DMX_IN_FRONTEND = 2, 20, 0
RAND = bytes([0x6a, 0xb0, 0xad, 0x8c, 0x4e, 0x69, 0x40, 0x2c, 0x58, 0x2e, 0x1e, 0xc0, 0xf5, 0xbe, 0xc9, 0x83,
              0x21, 0xe1, 0x07, 0x46, 0x5b, 0xa9, 0x7c, 0xaa, 0x9b, 0x0f, 0x2b, 0x25, 0x80, 0x88, 0x3e, 0x6b])

u8, u16, u32, u64 = ctypes.c_uint8, ctypes.c_uint16, ctypes.c_uint32, ctypes.c_uint64


class ExtCtl(ctypes.Structure):
    _fields_ = [('id', u32), ('size', u32), ('val', u64)]


class ExtSource(ctypes.Structure):
    _fields_ = [('src', u32), ('port', u32), ('ptype', u32)]


class DmxFilter(ctypes.Structure):
    _fields_ = [('filter', u8 * 16), ('mask', u8 * 16), ('mode', u8 * 16)]


class SctParams(ctypes.Structure):
    _fields_ = [('pid', u16), ('filter', DmxFilter), ('timeout', u32), ('flags', u32)]


class PesParams(ctypes.Structure):
    _fields_ = [('pid', u16), ('input', u32), ('output', u32), ('pes_type', u32), ('flags', u32)]


assert ctypes.sizeof(ExtCtl) == 16 and ctypes.sizeof(SctParams) == 60 and ctypes.sizeof(PesParams) == 20 and ctypes.sizeof(ExtSource) == 12

libc = ctypes.CDLL(None, use_errno=True)
libc.ioctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_void_p]
libc.ioctl.restype = ctypes.c_int
T0 = time.time()
NOISE = re.compile(r'aud_clock|demod |wlan|hdmi|CPU\d|Detected PIPT|asoc|usb|bt_|battery')
TEMOD = re.compile(r'(lxdvb|sdec|pvr|\bte\b|te_|dvb|ree_kdrv)', re.I)
BAD = re.compile(r'(ERR|error|fail|WRN|warn|CRI|invalid|timeout)', re.I)
last_ts = [0.0]
open_fds = []


def p(msg):
    print('%6.2f %s' % (time.time() - T0, msg))
    sys.stdout.flush()


def ioctl(fd, req, arg=None):
    ctypes.set_errno(0)
    r = libc.ioctl(fd, req, ctypes.addressof(arg) if arg is not None else None)
    return r, ctypes.get_errno()


def dmesg_new(label, init=False):
    """Print kernel lines newer than the last call, minus known noise; return the list of lines from the DVB/TE modules that look like errors."""
    out = subprocess.run(['dmesg'], capture_output=True, text=True).stdout.splitlines()
    new = []
    for ln in out:
        m = re.match(r'\[\s*(\d+\.\d+)\]', ln)
        if m and float(m.group(1)) > last_ts[0]:
            new.append((float(m.group(1)), ln))
    if new:
        last_ts[0] = new[-1][0]
    if init:
        return []
    shown = [ln for _, ln in new if not NOISE.search(ln)]
    bad = [ln for ln in shown if TEMOD.search(ln) and BAD.search(ln)]
    for ln in shown[-25:]:
        p('  dmesg[%s] %s' % (label, ln[:200]))
    return bad


def snap(label):
    for f in ('/proc/lxdvb/demux/status', '/proc/lxdvb/dvr/status'):
        try:
            txt = open(f).read()
        except OSError as e:
            txt = 'unreadable: %s' % e
        keep = [l for l in txt.splitlines() if l.strip() and not set(l.strip()) <= set('=')]
        p('  proc[%s] %s: %s' % (label, f.split('/proc/lxdvb/')[1], ' | '.join(keep)[:1500]))
    try:
        sig = open('/proc/lg/demod/signal_status').read()
        p('  demod[%s] %s' % (label, ' '.join(re.findall(r'(Demod status\s*=\s*\w+|OperMode\s*=\s*\w+|Packet Error\s*=\s*\d+|BER\s*=\s*\d+|Frequency Offset=\s*-?\d+)', sig))))
    except OSError as e:
        p('  demod[%s] unreadable %s' % (label, e))


def demod_short():
    try:
        sig = open('/proc/lg/demod/signal_status').read()
    except OSError:
        return 'demod?'
    return ' '.join(re.sub(r'\s+', ' ', x) for x in re.findall(r'(Demod status\s*=\s*\w+|Packet Error\s*=\s*\d+|Frequency Offset=\s*-?\d+)', sig))


def dvr_used():
    try:
        m = re.search(r'used:(\d+)\(([^)]*)\)', open('/proc/lxdvb/dvr/status').read())
    except OSError:
        return 'dvr?'
    return m.group(2) if m else 'none'


def in2():
    try:
        m = re.search(r'^\s*2\s+(\S+)\s+(\d+)\s+(\S+)\s+(-?\d+)\s+(\S+)\s+(\S+)', open('/proc/lxdvb/demux/status').read(), re.M)
    except OSError:
        return 'in2?'
    return '/'.join(m.groups()) if m else 'none'


class Stream:
    """Counts over every 192-byte unit read: sync byte at offset 4 (after the 4-byte timestamp), PID, scrambling, TEI, continuity."""

    def __init__(self, off, vid_pid):
        self.off, self.vid, self.rem = off, vid_pid, b''
        self.lc, self.per, self.gap_log = {}, {}, []
        self.units = self.bad = self.scr = self.tei = self.gaps = self.bytes = 0
        self.last_vid_t = 0.0
        self.vid_seen = 0

    def feed(self, d, now):
        buf = self.rem + d if self.rem else d
        n = len(buf) // 192
        self.rem = buf[n * 192:]
        self.bytes += len(d)
        lc, per = self.lc, self.per
        for b in range(self.off, n * 192, 192):
            self.units += 1
            if buf[b] != 0x47:
                self.bad += 1
                continue
            hi = buf[b + 1]
            pid = ((hi & 0x1f) << 8) | buf[b + 2]
            c3 = buf[b + 3]
            per[pid] = per.get(pid, 0) + 1
            if hi & 0x80:
                self.tei += 1
            if c3 & 0xc0:
                self.scr += 1
            if c3 & 0x10:
                cc = c3 & 0xf
                last = lc.get(pid)
                if last is not None and cc != last and cc != ((last + 1) & 0xf):
                    self.gaps += 1
                    if len(self.gap_log) < 12:
                        self.gap_log.append((self.units, pid, last, cc))
                lc[pid] = cc
        v = per.get(self.vid, 0)
        if v > self.vid_seen:
            self.vid_seen = v
            self.last_vid_t = now


class Stop(Exception):
    pass


def check(label, r, e, what):
    if r < 0:
        raise Stop('%s: ioctl failed errno=%d (%s) at %s' % (label, e, os.strerror(e), what))


def guard(label):
    bad = dmesg_new(label)
    if bad:
        raise Stop('kernel log shows an error from a TE/DVB module after %s: %s' % (label, bad[0][:200]))


def open_dmx():
    fd = os.open(DEMUX, os.O_RDWR | os.O_NONBLOCK)
    open_fds.append(fd)
    return fd


def close_fd(fd):
    if fd in open_fds:
        open_fds.remove(fd)
        os.close(fd)


def sct_filter(fd, pid, tid, flags):
    q = SctParams()
    q.pid = pid
    if tid is not None:
        q.filter.filter[0] = tid
        q.filter.mask[0] = 0xFF
    q.flags = flags
    r, e = ioctl(fd, DMX_SET_FILTER, q)
    check('sct', r, e, 'DMX_SET_FILTER pid=0x%x' % pid)


def read_sections(fd, secs, want):
    po = select.poll()
    po.register(fd, select.POLLIN)
    end = time.time() + secs
    got = []
    while time.time() < end:
        if not po.poll(200):
            continue
        try:
            d = os.read(fd, 4096)
        except BlockingIOError:
            continue
        if d:
            got.append(d)
            if want(got):
                break
    return got


def desc_list(b):
    out, i = [], 0
    while i + 2 <= len(b):
        out.append((b[i], bytes(b[i + 2:i + 2 + b[i + 1]])))
        i += 2 + b[i + 1]
    return out


def parse_pat(sec):
    slen = ((sec[1] & 0xf) << 8) | sec[2]
    prog = {}
    for i in range(8, 3 + slen - 4, 4):
        n = (sec[i] << 8) | sec[i + 1]
        prog[n] = ((sec[i + 2] & 0x1f) << 8) | sec[i + 3]
    return (sec[3] << 8) | sec[4], prog


def parse_pmt(sec):
    slen = ((sec[1] & 0xf) << 8) | sec[2]
    pil = ((sec[10] & 0xf) << 8) | sec[11]
    info = {'program': (sec[3] << 8) | sec[4], 'pcr': ((sec[8] & 0x1f) << 8) | sec[9],
            'prog_desc': [t for t, _ in desc_list(sec[12:12 + pil])], 'es': []}
    i = 12 + pil
    while i + 5 <= 3 + slen - 4:
        eil = ((sec[i + 3] & 0xf) << 8) | sec[i + 4]
        info['es'].append({'type': sec[i], 'pid': ((sec[i + 1] & 0x1f) << 8) | sec[i + 2],
                           'desc': [t for t, _ in desc_list(sec[i + 5:i + 5 + eil])]})
        i += 5 + eil
    return info


def parse_vct(sec):
    out = []
    n = sec[9]
    i = 10
    for _ in range(n):
        name = bytes(sec[i:i + 14]).decode('utf-16-be', 'replace').strip('\x00')
        major = ((sec[i + 14] & 0xf) << 6) | (sec[i + 15] >> 2)
        minor = ((sec[i + 15] & 3) << 8) | sec[i + 16]
        tsid = (sec[i + 22] << 8) | sec[i + 23]
        prog = (sec[i + 24] << 8) | sec[i + 25]
        flags = sec[i + 26]
        stype = sec[i + 27] & 0x3f
        dl = ((sec[i + 30] & 3) << 8) | sec[i + 31]
        out.append({'name': name, 'ch': '%d.%d' % (major, minor), 'tsid': tsid, 'program': prog,
                    'access_controlled': bool(flags & 0x20), 'hidden': bool(flags & 0x10), 'service_type': stype})
        i += 32 + dl
    return out


def analyse(units, want_pids, expect):
    """units: list of 192-byte units. Returns a dict of findings."""
    res = {'units': len(units)}
    offs = {0: 0, 4: 0}
    for u in units[:200]:
        for o in (0, 4):
            if u[o] == 0x47:
                offs[o] += 1
    off = 4 if offs[4] >= offs[0] else 0
    res['sync_offset_votes'] = offs
    res['sync_offset'] = off
    bad_sync = 0
    per = {}
    scr = 0
    tei = 0
    gaps = []
    last_cc = {}
    pat_ok = pmt_ok = False
    vid_pes = aud_pes = 0
    pcr_seen = set()
    stamps = []
    for u in units:
        ts = u[off:off + 188]
        if ts[0] != 0x47:
            bad_sync += 1
            continue
        stamps.append(int.from_bytes(u[0:4], 'big') if off == 4 else 0)
        pid = ((ts[1] & 0x1f) << 8) | ts[2]
        afc = (ts[3] >> 4) & 3
        cc = ts[3] & 0xf
        per[pid] = per.get(pid, 0) + 1
        if ts[1] & 0x80:
            tei += 1
        if (ts[3] >> 6) & 3:
            scr += 1
        pl = 4
        if afc & 2:
            alen = ts[4]
            if alen >= 7 and (ts[5] & 0x10):
                pcr_seen.add(pid)
            pl = 5 + alen
        if afc & 1:
            if pid in last_cc and pid != 0x1fff:
                exp = (last_cc[pid] + 1) & 0xf
                if cc != exp and not (cc == last_cc[pid]):
                    gaps.append((pid, last_cc[pid], cc))
            last_cc[pid] = cc
            if (ts[1] & 0x40) and pl + 4 <= 188 and ts[pl:pl + 3] == b'\x00\x00\x01':
                sid = ts[pl + 3]
                if 0xe0 <= sid <= 0xef:
                    vid_pes += 1
                elif 0xc0 <= sid <= 0xdf or sid == 0xbd:
                    aud_pes += 1
        elif pid in last_cc:
            pass
        if pid == 0 and (ts[1] & 0x40) and afc & 1 and pl + 1 < 188 and ts[pl + 1] == 0x00:
            pat_ok = True
        if pid == expect.get('pmt') and (ts[1] & 0x40) and afc & 1 and pl + 1 < 188 and ts[pl + 1] == 0x02:
            pmt_ok = True
    res.update(bad_sync=bad_sync, per_pid={('0x%04x' % k): v for k, v in sorted(per.items())}, scrambled_packets=scr,
               transport_errors=tei, cc_gaps=len(gaps), cc_gap_first=gaps[:6], pat_seen=pat_ok, pmt_seen=pmt_ok,
               video_pes_starts=vid_pes, audio_pes_starts=aud_pes, pcr_pids=['0x%04x' % x for x in sorted(pcr_seen)],
               unexpected_pids=['0x%04x' % k for k in per if k not in want_pids])
    if off == 4 and len(stamps) > 10:
        res['stamp_first_last'] = (stamps[0], stamps[-1])
    return res


def main():
    summary = {'gate': {}}
    fdA = fdD = None
    taps = []
    def on_term(signum, frame):
        raise Stop('terminated by signal %d' % signum)
    signal.signal(signal.SIGTERM, on_term)
    try:
        dmesg_new('init', init=True)
        snap('before')
        # key (device secret, memory only)
        import ctypes as ct
        lib = ct.CDLL('libhal_lg115x.so.2')
        gk = lib.HAL_PVR_CRYPTO_GetSecureKey
        gk.restype = ct.c_int
        gk.argtypes = [ct.c_void_p, ct.c_void_p]
        kb = (ct.c_ubyte * 256)(*([0xA5] * 256))
        rc = gk(None, ct.addressof(kb))
        seed = bytes(kb[:16])
        p('key: rc=%d allzero=%s sha256_prefix=%s' % (rc, seed == bytes(16), hashlib.sha256(seed).hexdigest()[:8]))
        if rc != 0 or seed == bytes(16):
            raise Stop('HAL key call failed')
        cert = bytes(seed[i % 16] ^ RAND[i] for i in range(32))
        del kb, seed

        # A: demux2 input = the demod's port, same values as LG's own example
        fdA = open_dmx()
        src = ExtSource(0, 0, 1)
        ctl = ExtCtl(7, ctypes.sizeof(src), ctypes.addressof(src))
        r, e = ioctl(fdA, DMX_EXT_S_CTL, ctl)
        check('A', r, e, 'DMX_EXT_S_CTL INPUTSOURCE')
        p('A: demux2 input set (IN_DEMOD 0, PARALLEL)')
        guard('A')
        snap('A')

        # B: PAT
        fd = open_dmx()
        sct_filter(fd, 0, 0x00, DMX_CHECK_CRC | DMX_ONESHOT | DMX_IMMEDIATE_START)
        got = read_sections(fd, 6, lambda g: len(g) >= 1)
        close_fd(fd)
        guard('B')
        if not got:
            raise Stop('no PAT within 6 s')
        tsid, progs = parse_pat(got[0])
        progs = {k: v for k, v in progs.items()}
        p('B: PAT tsid=0x%04x programs=%s' % (tsid, {k: '0x%x' % v for k, v in progs.items()}))
        summary['pat'] = {'tsid': tsid, 'programs': {str(k): v for k, v in progs.items()}}

        # C: TVCT (names, numbers, access_controlled), best effort
        vct = []
        fd = open_dmx()
        sct_filter(fd, 0x1ffb, 0xc8, DMX_CHECK_CRC | DMX_IMMEDIATE_START)

        def vct_done(g):
            last = g[0][7]
            return len({s[6] for s in g}) >= last + 1
        got = read_sections(fd, 6, vct_done)
        close_fd(fd)
        for s in {s[6]: s for s in got}.values():
            vct += parse_vct(s)
        p('C: TVCT %s' % [(v['ch'], v['name'], v['program'], 'AC' if v['access_controlled'] else '-') for v in vct])
        summary['vct'] = vct
        guard('C')

        # D: every PMT
        pmts = {}
        for n, pid in progs.items():
            if n == 0:
                continue
            fd = open_dmx()
            sct_filter(fd, pid, 0x02, DMX_CHECK_CRC | DMX_ONESHOT | DMX_IMMEDIATE_START)
            got = read_sections(fd, 4, lambda g: len(g) >= 1)
            close_fd(fd)
            if got:
                pmts[n] = parse_pmt(got[0])
                pmts[n]['pmt_pid'] = pid
            else:
                p('D: program %d pmt pid 0x%x: no PMT in 4 s' % (n, pid))
        guard('D')
        for n, pm in pmts.items():
            p('D: program %d pmt=0x%x pcr=0x%x prog_desc=%s es=%s' % (n, pm['pmt_pid'], pm['pcr'], ['0x%02x' % t for t in pm['prog_desc']],
                                                                [('0x%02x' % s['type'], '0x%x' % s['pid'], ['0x%02x' % t for t in s['desc']]) for s in pm['es']]))
        summary['pmts'] = {str(n): pm for n, pm in pmts.items()}
        rc_desc = sorted({t for pm in pmts.values() for t in pm['prog_desc'] + [d for s in pm['es'] for d in s['desc']] if t in (0xaa, 0x09)})
        summary['copy_flags'] = {'descriptor_tags_aa_or_09': ['0x%02x' % t for t in rc_desc],
                                 'vct_access_controlled': [v['ch'] for v in vct if v['access_controlled']]}
        p('D: copy-protection descriptors (0xAA redistribution / 0x09 CA) seen: %s; VCT access_controlled: %s' % (
            summary['copy_flags']['descriptor_tags_aa_or_09'], summary['copy_flags']['vct_access_controlled']))

        # choose the program: 2.4 if present, else the first with video
        vmap = {v['program']: v for v in vct}
        pick = None
        for n, pm in pmts.items():
            if vmap.get(n, {}).get('ch') == PREFER and any(s['type'] in (0x02, 0x1b) for s in pm['es']):
                pick = n
        if pick is None and ARGS.get('require') == '1':
            raise Stop('virtual channel %s is not in this multiplex (VCT: %s): not the RF channel that was asked for' % (PREFER, [v['ch'] for v in vct]))
        if pick is None:
            for n, pm in pmts.items():
                if any(s['type'] in (0x02, 0x1b) for s in pm['es']):
                    pick = n
                    break
        if pick is None:
            raise Stop('no program with a video stream')
        pm = pmts[pick]
        summary['pick'] = {'program': pick, 'ch': vmap.get(pick, {}).get('ch'), 'name': vmap.get(pick, {}).get('name')}
        p('E: picked program %d (%s %s)' % (pick, summary['pick']['ch'], summary['pick']['name']))
        want = {0, pm['pmt_pid'], pm['pcr']} | {s['pid'] for s in pm['es']}
        want = sorted(want)

        # F: dvr2 download + cert (one write)
        fdD = os.open(DVR, os.O_RDONLY | os.O_NONBLOCK)
        open_fds.append(fdD)
        p('F: dvr2 opened read-only')
        guard('F-open')
        cbuf = (ctypes.c_ubyte * 32).from_buffer_copy(cert)
        pc = ExtCtl(3, 32, ctypes.addressof(cbuf))
        r, e = ioctl(fdD, PVR_EXT_S_CTL, pc)
        del cert, cbuf
        p('F: cert ioctl r=%d errno=%d' % (r, e))
        summary['gate']['cert'] = (r == 0)
        guard('F-cert')
        check('F', r, e, 'PVR_EXT_S_CTL cert')

        # G: TS taps
        for pid in want:
            fd = open_dmx()
            q = PesParams()
            q.pid = pid
            q.input = DMX_IN_FRONTEND
            q.output = DMX_OUT_TS_TAP
            q.pes_type = DMX_PES_OTHER
            q.flags = DMX_IMMEDIATE_START
            r, e = ioctl(fd, DMX_SET_PES_FILTER, q)
            taps.append(fd)
            check('G', r, e, 'DMX_SET_PES_FILTER TS_TAP pid=0x%x' % pid)
        p('G: TS_TAP filters on %s' % ['0x%x' % x for x in want])
        guard('G')
        snap('G')

        # H: the first CAPTURE_S seconds go to RAM for analysis; with hold=N the read goes on N seconds more, counting only
        po = select.poll()
        po.register(fdD, select.POLLIN)
        vid_pid = next(s['pid'] for s in pm['es'] if s['type'] in (0x02, 0x1b))
        stream = Stream(4, vid_pid)
        t_start = time.time()
        st = {'first': None, 'last': None, 'errs': [], 'next_tick': 0.0, 'prev_bytes': 0, 'prev_t': 0.0, 'prev_vid': 0, 'max_gap': 0.0}

        def tick(now):
            win = max(now - st['prev_t'], 0.001)
            vid_now = stream.per.get(vid_pid, 0)
            p('H: T+%ds bytes=%d win_kBps=%d vid_pkts_win=%d gaps=%d bad=%d scr=%d tei=%d maxgap_s=%.2f cpu_s=%.1f' % (
                now - st['first'], stream.bytes, (stream.bytes - st['prev_bytes']) / win / 1000, vid_now - st['prev_vid'],
                stream.gaps, stream.bad, stream.scr, stream.tei, st['max_gap'], sum(os.times()[:2])))
            p('H:   demod: %s | dvr_used: %s | in2: %s | load: %s' % (demod_short(), dvr_used(), in2(), ' '.join(open('/proc/loadavg').read().split()[:3])))
            st['prev_bytes'], st['prev_t'], st['prev_vid'], st['max_gap'] = stream.bytes, now, vid_now, 0.0
            own = [ln for ln in dmesg_new('hold') if '(%d:' % os.getpid() in ln]
            if own:
                raise Stop('kernel log shows an error from this process: %s' % own[0][:200])

        def pump(until, store):
            chunks = []
            while True:
                now = time.time()
                if st['first'] is None:
                    if now > t_start + 5.0:
                        return chunks, 'no data from dvr2 within 5 s with the cert accepted'
                else:
                    if now - st['first'] >= until:
                        return chunks, None
                    if now - st['last'] > 5.0:
                        return chunks, 'no data for %.1f s' % (now - st['last'])
                    if now - stream.last_vid_t > 5.0:
                        return chunks, 'video PID 0x%x silent for %.1f s' % (vid_pid, now - stream.last_vid_t)
                    if os.path.exists(STOPFILE):
                        return chunks, 'stop file'
                    if now >= st['next_tick']:
                        st['next_tick'] = now + TICK_S
                        tick(now)
                if not po.poll(300):
                    continue
                try:
                    d = os.read(fdD, 192 * 2048)
                except BlockingIOError:
                    continue
                except OSError as ex:
                    st['errs'].append((round(time.time() - t_start, 2), ex.errno, os.strerror(ex.errno)))
                    return chunks, 'dvr read error errno=%d (%s)' % (ex.errno, os.strerror(ex.errno))
                if not d:
                    continue
                now = time.time()
                if st['first'] is None:
                    st['first'] = st['last'] = st['prev_t'] = now
                    stream.last_vid_t = now
                    st['next_tick'] = now + TICK_S
                    p('H: first data after %.2f s (%d bytes)' % (now - t_start, len(d)))
                    snap('H-first')
                else:
                    st['max_gap'] = max(st['max_gap'], now - st['last'])
                st['last'] = now
                stream.feed(d, now)
                if store:
                    chunks.append(d)

        chunks, why = pump(CAPTURE_S, True)
        raw = b''.join(chunks)
        n_reads = len(chunks)
        del chunks
        dur = (time.time() - st['first']) if st['first'] else 0
        p('H: first window: %d bytes in %.2f s (%.0f kB/s) in %d reads; errors=%s; ended: %s' % (
            len(raw), dur, (len(raw) / dur / 1000) if dur else 0, n_reads, st['errs'], why))
        snap('H-end')
        guard('H')
        summary['capture'] = {'bytes': len(raw), 'seconds': round(dur, 2), 'errors': st['errs'], 'ended': why,
                              'first_data_s': round(st['first'] - t_start, 2) if st['first'] else None}
        if why:
            raise Stop('first window ended early: %s' % why)
        open(RAW, 'wb').write(raw)
        os.chmod(RAW, 0o600)
        units = [raw[i:i + 192] for i in range(0, len(raw) - 191, 192)]
        summary['capture']['tail_bytes_not_multiple_of_192'] = len(raw) % 192
        res = analyse(units, set(want), {'pmt': pm['pmt_pid']})
        off = res['sync_offset']
        with open(TSF, 'wb') as f:
            for u in units:
                f.write(u[off:off + 188])
        os.chmod(TSF, 0o600)
        del units, raw
        summary['analysis'] = res
        p('I: %s' % json.dumps(res))
        if HOLD > 0:
            p('K: holding %d s more (touch %s to end it)' % (HOLD, STOPFILE))
            _, why = pump(CAPTURE_S + HOLD, False)
            summary['hold'] = {'seconds': round(time.time() - st['first'], 1), 'bytes': stream.bytes, 'units': stream.units, 'bad_sync': stream.bad,
                               'scrambled': stream.scr, 'tei': stream.tei, 'cc_gaps': stream.gaps, 'gap_first': stream.gap_log,
                               'per_pid': {'0x%04x' % k: v for k, v in sorted(stream.per.items())}, 'read_errors': st['errs'], 'ended': why,
                               'cpu_s': round(sum(os.times()[:2]), 1)}
            p('K: hold ended (%s) after %.1f s: %s' % (why or 'time reached', summary['hold']['seconds'], json.dumps(summary['hold'])))
        for fd in taps:
            close_fd(fd)
        taps.clear()
    except Stop as ex:
        p('STOP: %s' % ex)
        summary['stop'] = str(ex)
    except Exception as ex:  # unexpected: report, then clean up
        p('EXC: %r' % (ex,))
        summary['exc'] = repr(ex)
    finally:
        # J: teardown in the order that cannot overflow: stop routing, stop the download, put the input back, close the rest
        for fd in taps:
            try:
                close_fd(fd)
            except OSError as ex:
                p('close tap: %s' % ex)
        snap('J-taps-closed')
        if fdD is not None:
            try:
                close_fd(fdD)
            except OSError as ex:
                p('close dvr: %s' % ex)
        if fdA is not None:
            try:
                src = ExtSource(5, 0, 3)  # DMX_EXT_SRC_TYPE_NULL, port 0, DMX_EXT_PORT_TYPE_NONE: the state the channel started in
                ctl = ExtCtl(7, ctypes.sizeof(src), ctypes.addressof(src))
                r, e = ioctl(fdA, DMX_EXT_S_CTL, ctl)
                p('J: input back to NULL: r=%d errno=%d' % (r, e))
            except Exception as ex:
                p('J: input reset failed %r' % (ex,))
        for fd in list(open_fds):
            try:
                close_fd(fd)
            except OSError:
                pass
        dmesg_new('J')
        snap('after')
        print('SUMMARY ' + json.dumps(summary, default=str))
        sys.stdout.flush()


if __name__ == '__main__':
    main()
