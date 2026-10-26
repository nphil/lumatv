"""Run with: python3 -m unittest tests.test_framerate_probe

docs/research/framerate/probe.py's parsers and report, against text in the exact formats LG's kernel prints (de_proc.c
`_DE_WIN_PrintStatus`, `_proc_vsc_status_show`; vdec vds/proc.c, vrate.c, vsync.c, vo.c) and lines from the X4 bus capture, plus a
made-up run in which the DE's vsync count goes from 120/s to 100/s under the app's own video mute. It says the parsers read those
formats and the verdict names the switch; nothing about the TV.
"""
import json
import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(ROOT, "docs", "research", "framerate"))
import probe  # noqa: E402

INTERRUPTS = """           CPU0       CPU1       CPU2       CPU3
 104:    4821930          0          0          0     GICv2 136 Level     osd_irq
 111:          0          0          0          0     GICv2  79 Level     JpgDec
 175:    4821871          0          0          0     GICv2 207 Level     DE2VSYNC
 176:     812345       1234          0          0     GICv2 110 Level     mali_job
"""

WIN0_STATUS = """localmute=0
lowdelaymode=0
h/v active : 1920 / 1080
scanType   : 1
vFreq      : 239
svp        : 1
hdr_type   : 2
colordomain: 0
hdmi444mode: 0
phyMode    : 0
"""

SCALER = """[main.p0]
open=1
connect_src=VDEC
connect_index=0
connect_attr=0
output_mode=DISPLAY
mute=0
hdr=2
source_x=0
source_y=0
source_w=1920
source_h=1080
crop_x=0
crop_y=0
crop_w=1920
crop_h=1080
out_x=0
out_y=0
out_w=3840
out_h=2160
rotation=0
freeze=0
rgb444=0
adaptive=1
frame_delay=0
"""

VSYNC = """=== VSYNC PORT[0] ===
shared_mode        : 0
framerate          : 24000/1001(23.97)
interlaced         : 0
phase_shift        : 0
next_phase_shift   : 0
intv               : 1126125(41708us)
base_intv          : 1125000
min_intv           : 1125125
max_intv           : 1127125
dco_intv           : 0
max_phase_shift    : 0
    -- VSYNC[0] --
    shared_mode        : 0
    no_phase_shift     : 0
    fixed_frame_rate   : 0
    framerate          : 24000/1001(23.97)
    interlaced         : 0
    -----------------------------


=== VSYNC PORT[5] ===
shared_mode        : 2
framerate          : 120000/1000(120.00)
interlaced         : 0
phase_shift        : 0
next_phase_shift   : 0
intv               : 225000(8333us)
base_intv          : 0
min_intv           : 0
max_intv           : 0
dco_intv           : 0
max_phase_shift    : 0
    -- VSYNC[7] --
    shared_mode        : 2
    no_phase_shift     : 1
    fixed_frame_rate   : 1
    framerate          : 120/1(120.00)
    interlaced         : 0
    -----------------------------


"""

VRATE = """===  VRATE[0] ===
speed              : 1000
intr_out_type      : 0
framerate_max      : 120
framerate_min      : 10

- CONFIG -
    -- original --
    framerate          : 24000/1001(23.97)
    duration           : 3753
    -- current --
    framerate          : 24000/1001(23.97)
    duration           : 3753

- CODEC -
    -- original --
    framerate          : 24000/1001(23.97)
    duration           : 3753
    -- current --
    framerate          : 24000/1001(23.97)
    duration           : 3753

- PTS -
    prev_stc           : 123456
    changed            : 0
    buf_idx            : 3
    diff_stc_sum       : 0
    buf_cnt            : 8
    framerate          : 24000/1001(23.97)
    duration           : 3753
    stable             : 1

- FIXED -
    use_fixed          : 0
    framerate          : 0/0(0.00)
    duration           : 0

- STATUS -
    rate_src           : CODEC
    framerate          : 24000/1001(23.97)
    duration           : 3753


"""

VDISP = """=== VDISP[0],00000000c0ffee00 ===
paused             : 0
port               : 0(MAIN)
clk_src            : 0x101(STC1)
prev.pts           : 90090
prev.compensated_pts : 90090
phase_shift.errors : 0
phase_shift.val    : 0
is_film_mode       : 1
freeze.status      : 0
qlen               : 3
frame_cnt          : 1440
- FRAME -
    idx                : 7
    y_addr             : 0x80000000
    c_addr             : 0x80400000
    y_addr_bottom      : 0x00000000
    c_addr_bottom      : 0x00000000
    stride             : 1920
    codec              : 2(HEVC)
    dpb_map_mode       : 3(TILED)
    comp_mode          : 0(NONE)
    frame_rate         : 24000/1001(23.97)
    variable_framerate : 0
    aspect_ratio       : 0
    addr_extr_hdr      : 0x00000000
    width              : 1920
    height             : 1080
    crop(l,r,t,b)      : 0,0,0,0
    scan_type          : 0(PROG)
    frame_format       : 0
    discontinuity      : 0
    pts                : 90090
    enq_gstc           : 1
    uid                : 44
    display_period     : 1
    afd                : 0
    par(w,h)           : 1,1
    disp_result        : 0


"""

VO_STATUS = """proc_name=vo0
vdisp=00000000c0ffee00
flushed=0
vdisp_id=0
width=1920
height=1080
stride=1920
container_width=1920
container_height=1080
framerate_es_num=24000
framerate_es_den=1001
framerate_container_num=24000
framerate_container_den=1001
frame_format=0
queue_size=16
speed=1000
basetime.base_stc=0
basetime.base_pts=0
display_offset=0
flags=svp|hfr
constant_delay=0
buf_threshold=-1
par_w=1
par_h=1
frames_written=1440
frames_matched=1430
frames_dropped=2
frames_freerun=8
force_framerate_num=0
force_framerate_den=0
queued=3
displayed_queue_size=0
displayed_queue_head=0
displayed_queue_tail=0
displayed_queue_last_written_id=0
calling_read=0
calling_ioctl=0
last_ioctl_cmd=0x00001805

"""

# X4 capture, 2026-10-25: videooutputd's own video mute around a YouTube start (uptime seconds in timeStart).
MONITOR = [
    {"up": 27874.49, "line": json.dumps({"method": "setAvMute", "serial": 1, "timeStart": 27874.49107894, "transport": "TX", "type": "call",
                                         "sender": "com.webos.service.videooutput", "destination": "com.webos.service.avmutemanager",
                                         "payload": {"muteOn": True, "contextId": "_0mXDZk7eQBALU4", "muteType": "videoMute"}})},
    {"up": 27874.49, "line": json.dumps({"method": "setAvMute", "serial": 2, "timeStart": 27874.49107894, "transport": "RX", "type": "call",
                                         "payload": {"muteOn": True, "contextId": "_0mXDZk7eQBALU4", "muteType": "videoMute"}})},
    {"up": 27874.66, "line": json.dumps({"method": "setAvMute", "serial": 3, "timeStart": 27874.663, "transport": "TX", "type": "call",
                                         "sender": "com.webos.service.audio", "payload": {"muteOn": True, "muteType": "audioMute", "muteOwner": "audiod"}})},
    {"up": 27874.70, "line": json.dumps({"method": "setAvMute", "serial": 4, "timeStart": 27874.70422, "transport": "TX", "type": "call",
                                         "sender": "com.webos.service.videooutput", "payload": {"muteOn": False, "contextId": "_0mXDZk7eQBALU4", "muteType": "videoMute"}})},
]

VIDEOOUTPUT_LINE = json.dumps({"subscribed": True, "returnValue": True, "video": [
    {"sink": "MAIN", "frameRate": 50, "muted": False, "connected": True, "appId": "youtube.leanback.v4", "contentType": "movie", "width": 1920},
    {"sink": "SUB", "frameRate": 0, "muted": False, "connected": False, "appId": "unknown", "contentType": "unknown"}]})


def make_samples(t0=1000.0, period=0.05, secs=12.0, switch_at=5.0, mute_from=None, mute_to=None, gap_from=None, gap_to=None,
                 connect_at=0.0, unmute_at=None, close_at=None, at_100=()):
    """A run of ticks: the panel vsync (osd_irq) at 120/s, then 100/s from `switch_at` and inside each `at_100` span (from, to);
    DE2VSYNC at twice it; the main window's source connected from `connect_at` until `close_at`, its mute off from `unmute_at`
    (default `connect_at`); optional localmute and vsync gap spans."""
    rows, de, osd, t = [], 1000, 2000, 0.0
    unmute_at = connect_at if unmute_at is None else unmute_at
    while t <= secs:
        hz = 100.0 if t >= switch_at or any(a <= t < b for a, b in at_100) else 120.0
        in_gap = gap_from is not None and gap_from <= t < gap_to
        if not in_gap:
            de += int(round(hz * period * 2))
            osd += int(round(hz * period))
        mute = 56 if mute_from is not None and mute_from <= t < mute_to else 0
        connected = t >= connect_at and (close_at is None or t < close_at)
        rows.append({"t": t0 + t, "up": t0 + t, "irq": {"DE2VSYNC": de, "osd_irq": osd}, "flip": "12",
                     "win0": {"vFreq": 500 if t >= switch_at - 0.04 else 0, "localmute": mute, "lowdelaymode": 0, "hdr_type": 0},
                     "scaler": {"connect_src": "VDEC" if connected else "NONE", "mute": 0 if connected and t >= unmute_at else 1},
                     "vsync": probe.parse_vsync(VSYNC) if t >= switch_at else [],
                     "vrate": [], "vdisp": [], "vo": [{"flags": ["svp"]}]})
        t = round(t + period, 3)
    return rows


class Parsers(unittest.TestCase):
    def test_interrupts_sum_every_cpu_column(self):
        d = probe.parse_interrupts(INTERRUPTS)
        self.assertEqual(d, {"DE2VSYNC": 4821871, "osd_irq": 4821930})
        self.assertEqual(probe.parse_interrupts(INTERRUPTS, ("mali_job",)), {"mali_job": 813579})

    def test_win0_status_reads_both_line_styles(self):
        d = probe.parse_kv(WIN0_STATUS)
        self.assertEqual((d["localmute"], d["lowdelaymode"], d["vFreq"], d["hdr_type"], d["svp"]), (0, 0, 239, 2, 1))
        self.assertEqual(d["h/v active"], "1920 / 1080")

    def test_scaler_status(self):
        d = probe.parse_kv(SCALER)
        self.assertEqual((d["connect_src"], d["mute"], d["adaptive"], d["out_w"], d["output_mode"]), ("VDEC", 0, 1, 3840, "DISPLAY"))

    def test_vsync_ports_and_their_vsyncs(self):
        ports = probe.parse_vsync(VSYNC)
        self.assertEqual([p["ch"] for p in ports], [0, 5])
        self.assertEqual(ports[0]["framerate"], (24000, 1001, 23.97))
        self.assertEqual(ports[0]["intv_us"], 41708)
        self.assertEqual(ports[0]["interlaced"], 0)
        self.assertEqual(ports[0]["vsyncs"], [{"id": 0, "fixed_frame_rate": 0, "framerate": (24000, 1001, 23.97)}])
        self.assertEqual(ports[1]["vsyncs"][0]["fixed_frame_rate"], 1)
        self.assertEqual(ports[1]["framerate"], (120000, 1000, 120.0))

    def test_vrate_status_and_fixed(self):
        v = probe.parse_vrate(VRATE)
        self.assertEqual(len(v), 1)
        self.assertEqual((v[0]["id"], v[0]["rate_src"], v[0]["status"], v[0]["duration"]), (0, "CODEC", (24000, 1001, 23.97), 3753))
        self.assertEqual((v[0]["use_fixed"], v[0]["fixed"], v[0]["framerate_max"]), (0, (0, 0, 0.0), 120))

    def test_vdisp_instance_and_frame(self):
        v = probe.parse_vdisp(VDISP)
        self.assertEqual((v[0]["id"], v[0]["port"], v[0]["is_film_mode"], v[0]["frame_cnt"], v[0]["paused"], v[0]["qlen"]), (0, "MAIN", 1, 1440, 0, 3))
        self.assertEqual(v[0]["frame"], {"frame_rate": (24000, 1001, 23.97), "width": 1920, "height": 1080, "variable_framerate": 0})

    def test_vo_status_flags(self):
        d = probe.parse_vo_status(VO_STATUS)
        self.assertEqual(d["flags"], ["svp", "hfr"])
        self.assertEqual((d["framerate_es_num"], d["framerate_es_den"], d["frames_dropped"], d["queued"]), (24000, 1001, 2, 3))
        self.assertEqual(probe.parse_vo_status("flags=none\n")["flags"], [])

    def test_capture_reply(self):
        self.assertEqual(probe.parse_capture_reply('{"returnValue":true,"writtenBytes":17321,"path":"/tmp/x.png"}'), 17321)
        self.assertIsNone(probe.parse_capture_reply("luna-send: timeout"))

    def test_json_objects_from_pretty_and_compact_output(self):
        text = '{"a": 1}\n{\n "b": {\n  "c": [1, 2]\n }\n}\nnoise {"d": 3}'
        self.assertEqual(probe.json_objects(text), [{"a": 1}, {"b": {"c": [1, 2]}}, {"d": 3}])

    def test_marks_and_kernel_lines(self):
        self.assertEqual(probe.parse_marks("1234.56 launch\n1250.1 keys\nbad line\n"), [(1234.56, "launch"), (1250.1, "keys")])
        log = ("[ 100.1] de: set_frate fr_rate:50, isForceFreeRun:0\n[ 100.2] usb 3-1: new device\n"
               "[ 100.3] >>>>>>>  timing_changed[0] = 3  <<<<<<<\n[ 100.4] 4k HFR mode [ON]  fr[1200/10Hz] w/h[3840x2160]\n"
               "[ 100.5] mute-off win[0]\n[ 100.6] vdisp: [VDISP0] FrameDuration90k: 3753(CODEC)--> 1800(CODEC)\n")
        self.assertEqual(len(probe.kernel_lines(log)), 5)
        self.assertNotIn("usb", "".join(probe.kernel_lines(log)))


class Report(unittest.TestCase):
    def test_rates_and_bands(self):
        rows = make_samples(secs=3.0, switch_at=99)
        r = probe.rates(rows, probe.OSD_IRQ)
        self.assertIsNone(r[0][1])
        self.assertAlmostEqual(r[-1][1], 120.0, delta=4.0)
        self.assertEqual(probe.rate_band(r[-1][1]), 120)
        self.assertEqual(probe.rate_band(101.2), 100)
        self.assertEqual(probe.rate_band(59.4), 60)
        self.assertIsNone(probe.rate_band(80))
        de = probe.rates(rows, probe.DE_IRQ)
        self.assertIsNone(probe.rate_band(de[-1][1]))

    def test_switch_is_found_once(self):
        rows = make_samples(secs=12.0, switch_at=5.0)
        sw = probe.switches(rows)
        self.assertEqual(len(sw), 1)
        up, a, b = sw[0]
        self.assertEqual((a, b), (120, 100))
        self.assertTrue(1004.7 <= up <= 1005.3, up)

    def test_a_fifth_of_a_second_at_100_hz_shows_as_two_switches(self):
        # The kernel applies the previous stream's rate at a connect until the new stream's first frame (about 0.2 s in round 11).
        sw = probe.switches(make_samples(secs=6.0, switch_at=99, at_100=[(3.0, 3.2)]))
        self.assertEqual([(a, b) for _, a, b in sw], [(120, 100), (100, 120)])
        self.assertAlmostEqual(sw[0][0], 1003.0, delta=0.1)
        self.assertAlmostEqual(sw[1][0], 1003.2, delta=0.1)
        self.assertEqual(probe.switches(make_samples(secs=6.0, switch_at=99)), [])

    def test_gaps_and_mutes(self):
        rows = make_samples(secs=8.0, switch_at=99, mute_from=2.0, mute_to=2.3, gap_from=4.0, gap_to=4.2)
        gaps = probe.vsync_gaps(rows)
        self.assertEqual(len(gaps), 1)
        self.assertAlmostEqual(gaps[0][0], 1004.0, delta=0.06)
        self.assertAlmostEqual(gaps[0][1], 1004.2, delta=0.06)
        mutes = probe.mute_windows(rows)
        self.assertEqual(len(mutes), 1)
        self.assertEqual(mutes[0][2], 56)
        self.assertAlmostEqual(mutes[0][0], 1002.0, delta=0.01)
        self.assertEqual(probe.vsync_gaps(make_samples(secs=2.0, switch_at=99)), [])
        self.assertEqual(probe.mute_windows(make_samples(secs=2.0, switch_at=99)), [])

    def test_source_and_decoder_rates_change_once(self):
        rows = make_samples(secs=8.0, switch_at=5.0)
        src = probe.source_rates(rows)
        self.assertEqual([v for _, v in src], [0, 500])
        dec = probe.decoder_rates(rows)
        self.assertEqual([(hz, f) for _, hz, f in dec], [(23.97, 0)])

    def test_app_mute_windows_from_the_bus(self):
        spans = probe.app_mute_windows(MONITOR)
        self.assertEqual(len(spans), 1)
        self.assertAlmostEqual(spans[0][0], 27874.491, places=2)
        self.assertAlmostEqual(spans[0][1], 27874.704, places=2)
        self.assertEqual(probe.app_mute_windows(MONITOR[:1]), [(27874.49107894, None)])

    def test_videooutput_events(self):
        ev = probe.videooutput_events([{"up": 5.0, "line": VIDEOOUTPUT_LINE}, {"up": 5.1, "line": "not json"}])
        self.assertEqual(len(ev), 1)
        self.assertEqual(ev[0][0], 5.0)
        self.assertEqual(ev[0][1][0]["frameRate"], 50)
        self.assertEqual(ev[0][1][0]["appId"], "youtube.leanback.v4")

    def test_shot_spans_from_sizes(self):
        shots = [{"n": i, "up": 1000 + i * 0.2, "bytes": 3000 if 3 <= i <= 5 else 18000} for i in range(10)]
        spans = probe.shot_spans(shots)
        self.assertEqual(spans["black"], [(1000.6, 1001.0)])
        self.assertEqual(spans["frozen"], [])

    def test_shots_to_keep(self):
        shots = [{"n": i, "up": 1000 + i * 0.25} for i in range(100)]
        keep = probe.shots_to_keep(shots, [1010.0], every=20, around=1.0)
        self.assertEqual(sorted(k for k in keep if k % 20 != 0), [36, 37, 38, 39, 41, 42, 43, 44])

    def test_a_switch_at_a_start_before_the_window_is_shown_is_clean(self):
        # The X4 start: the source connects at 27874.3, videooutputd's mute (the bus) ends at 27874.70 and the scaler's at 27874.75;
        # the switch lands at 27874.55, before the window is on screen.
        rows = make_samples(t0=27870.0, secs=12.0, switch_at=4.55, connect_at=4.3, unmute_at=4.75)
        shots = [{"n": i, "up": 27870 + i * 0.25, "bytes": 3000 if 17 <= i <= 19 else 18000} for i in range(48)]
        v = probe.verdict(rows, shots, [{"up": 27875.0, "line": VIDEOOUTPUT_LINE}], MONITOR, [(27870.5, "launch"), (27882.0, "end")])
        self.assertEqual(len(v["switches"]), 1)
        self.assertEqual(v["switches"][0][1:], (120, 100))
        self.assertEqual(v["switch_where"], ["start"])
        self.assertTrue(v["clean"])
        self.assertIn("at a video start, before its window was shown (shown at 4.75 s)", v["words"][0])
        self.assertEqual(v["starts"], [(4.75, 4.75, 0.0)])
        self.assertEqual(v["app_video_mute"], [(4.49, 4.7)])
        self.assertEqual(v["marks"], [(0.5, "launch"), (12.0, "end")])
        self.assertEqual(v["black"], [(4.25, 4.75)])

    def test_a_switch_in_the_middle_of_playback_fails(self):
        shots = [{"n": i, "up": 1000 + i * 0.25, "bytes": 18000} for i in range(48)]
        v = probe.verdict(make_samples(secs=12.0, switch_at=5.0, mute_from=5.0, mute_to=5.4), shots, [], [], [])
        self.assertFalse(v["clean"])
        self.assertTrue(v["evidence"])
        self.assertEqual(v["switch_where"], ["showing"])
        self.assertIn("WHILE VIDEO WAS SHOWING; firmware mute [(5.0, 5.35, 56)]", v["words"][0])
        # With no mute at all it still fails: the picture was on screen when the rate changed.
        v = probe.verdict(make_samples(secs=12.0, switch_at=5.0), shots, [], [], [])
        self.assertFalse(v["clean"])
        self.assertIn("WHILE VIDEO WAS SHOWING; firmware mute none", v["words"][0])

    def test_the_helpers_return_with_no_window_is_clean(self):
        # A 50 fps start (the switch before the window is shown), the window closing at 4.0 with the panel left at 100 Hz, and the
        # helper's return at 6.0 with no source connected.
        rows = make_samples(secs=9.0, switch_at=99, connect_at=1.0, unmute_at=1.5, close_at=4.0, at_100=[(1.3, 6.0)])
        for r in rows:
            if r["scaler"]["connect_src"] == "VDEC":
                r["win0"]["vFreq"] = 500
        shots = [{"n": i, "up": 1000 + i * 0.25, "bytes": 18000} for i in range(36)]
        v = probe.verdict(rows, shots, [], [], [])
        self.assertEqual([(a, b) for _, a, b in v["switches"]], [(120, 100), (100, 120)])
        self.assertEqual(v["switch_where"], ["start", "no window"])
        self.assertTrue(v["clean"])
        self.assertIn("with no video window (no source connected)", v["words"][1])

    def test_round_11_reads_as_two_switches_before_the_window_was_shown(self):
        def run(name):
            d = os.path.join(ROOT, "docs", "research", "framerate", name)
            meta = json.loads(probe.read(os.path.join(d, "run.json")) or "{}")
            return probe.verdict(*(probe.load_jsonl(os.path.join(d, f)) for f in ("samples.jsonl", "shots.jsonl", "videooutput.jsonl",
                                                                                   "monitor.jsonl")),
                                 probe.parse_marks(probe.read(os.path.join(d, "marks.txt"))), None, bool(meta.get("protected")))
        v = run("youtube50-20261026-0424")
        self.assertEqual([(a, b) for _, a, b in v["switches"]], [(120, 100), (100, 120)])
        self.assertAlmostEqual(v["switches"][0][0], 6.9, delta=0.1)
        self.assertAlmostEqual(v["switches"][1][0], 19.69, delta=0.1)
        self.assertEqual(v["switch_where"], ["start", "start"])
        self.assertEqual([d for _, _, d in v["starts"]], [0.15, 0.51])
        self.assertTrue(v["clean"])
        # LG's start mute at that night's six starts without a switch is what START_MUTE_S says.
        quiet = [d for name in ("netflix24-20261026-0409", "plex25after24-20261026-0414", "youtube50-20261026-0411",
                                "youtube50-20261026-0412") for _, _, d in run(name)["starts"]]
        self.assertEqual(len(quiet), 6)
        self.assertEqual((min(quiet), max(quiet)), probe.START_MUTE_S)

    def test_verdict_without_a_switch(self):
        shots = [{"n": i, "up": 27870 + i * 0.25, "bytes": 18000} for i in range(8)]
        rows = make_samples(secs=6.0, switch_at=99)
        for r in rows:
            r["win0"]["vFreq"] = 240
            r["vsync"] = probe.parse_vsync(VSYNC)
        v = probe.verdict(rows, shots, [], [], [])
        self.assertTrue(v["clean"])
        self.assertIn("no output-rate switch", v["words"][0])
        self.assertIn("120 Hz band", v["words"][0])

    def test_a_run_where_no_video_played_or_the_mute_never_cleared_fails(self):
        shots = [{"n": i, "up": 27870 + i * 0.25, "bytes": 18000} for i in range(8)]
        v = probe.verdict(make_samples(secs=6.0, switch_at=99), shots, [], [], [])
        self.assertFalse(v["clean"])
        self.assertFalse(v["evidence"])
        self.assertIn("no video", " ".join(v["words"]))
        rows = make_samples(secs=6.0, switch_at=0.0, mute_from=0.0, mute_to=99.0)
        v = probe.verdict(rows, shots, [], [], [])
        self.assertFalse(v["clean"])
        self.assertIn("never cleared", " ".join(v["words"]))

    def test_a_run_with_no_samples_or_no_shots_fails_for_want_of_evidence(self):
        v = probe.verdict([], [], [], [], [])
        self.assertFalse(v["clean"])
        self.assertIn("no samples", " ".join(v["words"]))
        v = probe.verdict(make_samples(secs=6.0, switch_at=99), [], [], [], [])
        self.assertFalse(v["clean"])
        self.assertIn("no screen shots", " ".join(v["words"]))
        d = tempfile.mkdtemp()
        try:
            self.assertIn("VERDICT: FAIL: no evidence", probe.report(d))
        finally:
            shutil.rmtree(d)

    def test_a_log_with_bytes_that_are_not_utf8_is_read(self):
        d = tempfile.mkdtemp()
        try:
            path = os.path.join(d, "messages")
            with open(path, "wb") as f:
                f.write(b"kernel: vsync \xf7 ok\n")
            self.assertIn("vsync", probe.read(path))
        finally:
            shutil.rmtree(d)

    def test_report_writes_summary_and_verdict_and_prunes(self):
        d = tempfile.mkdtemp()
        try:
            rows = make_samples(secs=12.0, switch_at=5.0, connect_at=4.6, unmute_at=5.3)
            with open(os.path.join(d, "samples.jsonl"), "w") as f:
                f.write("".join(json.dumps(r) + "\n" for r in rows))
            shots = [{"n": i, "up": 1000 + i * 0.25, "bytes": 18000} for i in range(48)]
            with open(os.path.join(d, "shots.jsonl"), "w") as f:
                f.write("".join(json.dumps(s) + "\n" for s in shots))
            for s in shots:
                with open(os.path.join(d, "shot-%04d.png" % s["n"]), "wb") as f:
                    f.write(b"\x89PNG")
            with open(os.path.join(d, "marks.txt"), "w") as f:
                f.write("1000.5 launch\n1012.0 end\n")
            with open(os.path.join(d, "run.json"), "w") as f:
                json.dump({"protected": True}, f)
            with open(os.path.join(d, "helper.txt"), "w") as f:
                f.write("2026-10-27T14:00:01Z info frame rate: Home was left at 100.0 Hz with no video window open; set back to 120 Hz\n")
            text = probe.report(d)
            self.assertIn("VERDICT: PASS", text)
            self.assertIn("switches: 120 -> 100 Hz at ", text)
            self.assertIn(" s (start)", text)
            self.assertIn("starts (videooutputd's unmute -> window on screen): 5.3 -> 5.3 s (+0.0 s)", text)
            self.assertIn("helper: 2026-10-27T14:00:01Z info frame rate: Home was left at 100.0 Hz", text)
            self.assertIn("protected video: black by design", text)
            self.assertTrue(os.path.exists(os.path.join(d, "summary.txt")))
            v = json.load(open(os.path.join(d, "verdict.json")))
            self.assertEqual(v["switches"][0][1:], [120, 100])
            removed = probe.prune_shots(d)
            left = sorted(int(n[5:9]) for n in os.listdir(d) if n.startswith("shot-"))
            self.assertEqual(removed + len(left), 48)
            self.assertIn(0, left)
            self.assertTrue(any(abs(1000 + n * 0.25 - (1000 + v["switches"][0][0])) <= 1.5 for n in left))
        finally:
            shutil.rmtree(d)


class Plan(unittest.TestCase):
    def test_runs_and_plan(self):
        for name, run in probe.RUNS.items():
            self.assertIn("id", run["launch"], name)
            self.assertGreater(probe.total_seconds(run), run["secs"], name)
            steps = [s for s, _ in probe.plan(run)]
            self.assertEqual(steps[:6], ["preflight", "upload", "mute", "sample", "launch", "keys"], name)
            self.assertEqual(steps[-4:], ["close", "unmute", "fetch", "standby"], name)
        self.assertIn("home", [s for s, _ in probe.plan(probe.RUNS["youtube50"])])
        self.assertIn("home", [s for s, _ in probe.plan(probe.RUNS["youtube50home"])])
        self.assertEqual(probe.RUNS["youtube50home"]["launch"]["params"]["contentTarget"], probe.YOUTUBE_50_CLIP)
        self.assertIn("second", [s for s, _ in probe.plan(probe.RUNS["plex25after24"])])

    def test_main_plan_is_the_default_and_the_fr_rate_write_is_refused(self):
        self.assertEqual(probe.main(["--run", "netflix24"]), 0)
        with self.assertRaises(SystemExit):
            probe.main(["--go", "--allow-fr-rate-write"])


if __name__ == "__main__":
    unittest.main()
