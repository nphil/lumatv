import contextlib
import copy
import io
import json
import os
import tempfile
import unittest

import analyse_capture as ac

HZ = 90000
LOAD_NS = 1_000_000_000
FIRST_KEY = LOAD_NS + 50_000_000
STREAMER_FIELDS = ["epoch", "seq", "flags", "frame_index", "rtp_timestamp", "rtp_clock_hz", "bytes", "sink_entry_ns", "release_ns",
                   "wire_key_ns", "handover_done_ns", "held_at_entry", "in_flight_at_send"]
FEEDER_FIELDS = ["epoch", "seq", "wire_key_ns", "bytes", "key", "recv_ns", "first_call_ns", "last_return_ns", "attempts", "player_ms",
                 "play_rc", "queue", "wait_us", "reason"]
COUNTS = {"omitted": 0, "evicted": 2, "unmatched": 1, "resets": 3, "stored": 0}


def steps(count):
    """A varying cadence (not a constant one): 16.667 ms and 8.333 ms alternately, in RTP ticks."""
    return [1500 if i % 2 else 750 for i in range(count)]


def synthetic(count=30, constant=False):
    """The same frames at the sink (streamer) and at play() (feeder). Frame 7 repeats the id before it, frames 15 and 16 are one id apart by 3,
    frame 20 never reaches the feeder and the feeder holds one frame the streamer never recorded. The RTP timestamp crosses its 32-bit wrap."""
    ts, key, index = WRAP_START, FIRST_KEY, 100
    srows, frows = [], []
    for i in range(count):
        if i:
            step = 1500 if constant else steps(count)[i]
            ts = (ts + step) % (1 << 32)
            key += step * 1_000_000_000 // HZ
            index += 0 if i == 7 else 3 if i == 16 else 1
        srows.append([1, i, 1 | 2 | 32 | 128, index, ts, HZ, 4000 + i, key - 200_000, key - 150_000, key, key + 30_000, 0, 1])
        if i != 20:
            ms = (key - LOAD_NS + 999_999) // 1_000_000
            frows.append([5, len(frows), key, 4000 + i, 0, key + 400_000, key + 410_000, key + 600_000, 1, ms, 0, 1, 0, 0])
    frows.append([5, len(frows), key + 5_000_000_000, 100, 0, key + 5_000_000_100, key + 5_000_000_200, key + 5_000_000_300, 1, 9999, 0, 1, 0, 0])
    return (
        {"format": ac.FORMAT, "version": 1, "producer": "streamer", "complete": False, "incomplete": ["ring full: records omitted"],
         "header": {"build_id": "0123456789abcdef", "codec": "H265", "width": 3840, "height": 2160, "fps": 120, "rtp_clock_hz": HZ,
                    "frame_pacing": "Smooth", "start_ns": FIRST_KEY - 200_000, "epoch": 1, "counts": dict(COUNTS, stored=count),
                    "definitions": {"evicted": "in-flight entries discarded before the feeder answered for them"}},
         "fields": STREAMER_FIELDS, "records": srows},
        {"format": ac.FORMAT, "version": 1, "producer": "feeder", "complete": True, "incomplete": [],
         "header": {"build_id": "0123456789abcdef", "codec": "H265", "width": 3840, "height": 2160, "fps": 120, "rtp_clock_hz": None,
                    "frame_pacing": None, "epochs": [{"epoch": 5, "load_ns": LOAD_NS}], "counts": dict(COUNTS, evicted=0, unmatched=1, stored=len(frows))},
         "fields": FEEDER_FIELDS, "records": frows},
    )


WRAP_START = (1 << 32) - 3000


PREFIX_FIELDS = ["frame_index", "rtp_timestamp", "arrival_ns", "status", "byte0", "header_size", "byte2", "byte3", "capture_time_us", "sender_paced",
                 "presentation_time_us", "access_unit_length", "prefix_bytes", "stripped_bytes", "other_tags"]


def add_prefix(streamer, drift_ppm=0, paced=lambda i: None, missing=2):
    """The transport's rows for the streamer's frames, each a TV prefix read in full (81 08 01 02, 20 bytes). The capture clock runs drift_ppm fast
    against RTP, the last `missing` frames are not in the log, and one earlier picture has no streamer record. Returns the rows to edit."""
    records = streamer["records"][:len(streamer["records"]) - missing]
    table = [[records[0][3] - 1, (records[0][4] - 750) % ac.WRAP, 1, "read", 0x81, 8, 1, 2, 1_000_000 - round(750 * 1e6 / HZ * (1 + drift_ppm / 1e6)),
              None, None, 7800, 20, 20, None]]
    for i, record in enumerate(records):
        capture_us = round(1_000_000 + (record[4] - records[0][4]) % ac.WRAP * 1e6 / HZ * (1 + drift_ppm / 1e6))
        table.append([record[3], record[4], record[7], "read", 0x81, 8, 1, 2, capture_us, paced(i), None, 7800 + i, 20, 20, None])
    streamer["frame_prefix"] = {"fields": list(PREFIX_FIELDS), "rows": table, "capacity": 2048, "window_ns": 12_000_000_000, "definitions": {},
                                "counts": {"seen": len(table), "stored": len(table), "omitted": 0, "read": len(table), "absent": 0, "malformed": 0}}
    return table


def report_track(clock_hz=None, ssrc=None, header_only=0, read=0, usable=0, first=None, last=None):
    """One track of the transport's sender_reports section."""
    def cadence(count):
        return {"count": count, "every_us": 5_000_000 if count > 1 else None, "max_gap_us": None, "first_ns": None, "last_ns": None}
    return {"clock_hz": clock_hz, "ssrc": ssrc, "wire": cadence(header_only),
            "read": dict(cadence(read), usable=usable, via="srtcp" if read else None, first=first, last=last)}


def report_sample(ssrc, ntp_us, rtp):
    return {"ssrc": ssrc, "ntp_us": ntp_us, "rtp": rtp, "packets": 10, "octets": 1000, "arrival_ns": 5, "via": "srtcp"}


def add_reports(streamer, video, audio):
    heard = video["wire"]["count"] + video["read"]["count"] > 0 and audio["wire"]["count"] + audio["read"]["count"] > 0
    streamer["sender_reports"] = {
        "video": video, "audio": audio, "unmapped": report_track(),
        "coverage": {"both_heard": heard, "both_read": video["read"]["count"] > 0 and audio["read"]["count"] > 0,
                     "both_usable": video["read"]["usable"] > 0 and audio["read"]["usable"] > 0},
    }


class AnalyseCaptureTests(unittest.TestCase):
    def test_the_join_keeps_unmatched_records_on_both_sides_and_maps_the_epochs_by_load_time(self):
        report = ac.analyse(*synthetic())
        join = report["join"]
        self.assertEqual((join["matched"], join["streamer_unmatched"], join["feeder_unmatched"]), (29, 1, 1))
        self.assertEqual(join["streamer_unmatched_seq"], [20])
        self.assertEqual(report["epochs"], {"streamer": 1, "feeder": [5]})

    def test_a_key_in_an_earlier_feeder_epoch_is_not_a_match_but_a_reload_inside_the_session_is(self):
        streamer, feeder = synthetic()
        feeder["header"]["epochs"] = [{"epoch": 4, "load_ns": LOAD_NS - 5_000_000_000}, {"epoch": 5, "load_ns": LOAD_NS}]
        for row in feeder["records"][:3]:
            row[0] = 4
        join = ac.analyse(streamer, feeder)["join"]
        self.assertEqual((join["matched"], join["feeder_unmatched"]), (26, 4), "epoch 4 loaded before this session")
        feeder["header"]["epochs"] = [{"epoch": 5, "load_ns": LOAD_NS}, {"epoch": 6, "load_ns": LOAD_NS + 1_000_000_000}]
        for row in feeder["records"][:3]:
            row[0] = 6
        join = ac.analyse(streamer, feeder)["join"]
        self.assertEqual((join["matched"], join["feeder_unmatched"]), (29, 1), "a reload inside the capture continues the session")

    def test_the_three_spacings_are_kept_apart_across_the_rtp_wrap(self):
        spacing = ac.analyse(*synthetic())["spacing"]
        self.assertAlmostEqual(spacing["sender_ms"]["min"], 750 * 1000 / HZ, places=6)
        self.assertAlmostEqual(spacing["sender_ms"]["max"], 1500 * 1000 / HZ, places=6)
        self.assertGreater(spacing["sender_ms"]["min"], 0, "the 32-bit wrap is not a negative step")
        self.assertEqual(spacing["sender_ms"]["distinct"], 2)
        self.assertAlmostEqual(spacing["handover_ms"]["min"], 750 * 1000 / HZ, places=2)
        self.assertGreaterEqual(spacing["player_ms"]["distinct"], 2)
        self.assertFalse(spacing["sender_constant"])
        self.assertEqual(spacing["player_follows_sender_within_1ms"], 1.0)
        self.assertEqual(spacing["frame_index"], {"repeated": 1, "gaps": 1, "missing": 2, "without_index": 0})

    def test_a_constant_stream_is_reported_as_constant_and_a_player_jump_is_seen(self):
        streamer, feeder = synthetic(constant=True)
        report = ac.analyse(streamer, feeder)
        self.assertTrue(report["spacing"]["sender_constant"] and report["spacing"]["handover_constant"])
        self.assertTrue(any("constant" in note for note in report["notes"]))
        feeder["records"][5][9] += 5
        follows = ac.analyse(streamer, feeder)["spacing"]["player_follows_sender_within_1ms"]
        self.assertTrue(0.8 < follows < 1.0)

    def test_the_report_carries_the_header_the_counts_per_producer_and_what_it_cannot_show(self):
        report = ac.analyse(*synthetic())
        text = ac.render(report)
        for needle in ("0123456789abcdef", "H265", "3840x2160", "120 fps", "90000 Hz", "Smooth"):
            self.assertIn(needle, text)
        self.assertIn("streamer: 0 / 2 / 1 / 3", text)
        self.assertIn("feeder: 0 / 0 / 1 / 3", text)
        self.assertIn("cannot prove variable refresh", text)
        for missing in ("decoder completion", "selected-frame id", "release", "optical times"):
            self.assertIn(missing, text)
        self.assertIn("incomplete", text)
        self.assertEqual(report["header"], {"build_id": "0123456789abcdef", "codec": "H265", "width": 3840, "height": 2160, "fps": 120,
                                            "rtp_clock_hz": HZ, "frame_pacing": "Smooth"})

    def test_a_file_without_the_header_fields_is_refused_by_name(self):
        for producer, path, name in [
            ("streamer", ("header", "build_id"), "build id"), ("streamer", ("header", "codec"), "codec"), ("streamer", ("header", "width"), "width"),
            ("streamer", ("header", "height"), "height"), ("streamer", ("header", "fps"), "fps"), ("streamer", ("header", "rtp_clock_hz"), "RTP clock"),
            ("streamer", ("header", "frame_pacing"), "frame pacing"), ("streamer", ("header", "counts", "evicted"), "evicted"),
            ("streamer", ("header", "counts", "resets"), "resets"), ("feeder", ("header", "counts", "unmatched"), "unmatched"),
            ("feeder", ("header", "counts", "omitted"), "omitted"), ("feeder", ("header", "build_id"), "build id"),
            ("feeder", ("header", "epochs"), "epochs"),
        ]:
            with self.subTest(producer=producer, path=path):
                documents = list(synthetic())
                doc = documents[0 if producer == "streamer" else 1]
                holder = doc
                for part in path[:-1]:
                    holder = holder[part]
                del holder[path[-1]]
                with self.assertRaises(ac.CaptureError) as caught:
                    ac.analyse(*documents)
                self.assertIn(name, str(caught.exception))

    def test_a_pacing_word_the_app_does_not_use_and_a_record_off_the_field_list_are_refused(self):
        streamer, feeder = synthetic()
        streamer["header"]["frame_pacing"] = "Fast"
        with self.assertRaises(ac.CaptureError):
            ac.analyse(streamer, feeder)
        streamer, feeder = synthetic()
        feeder["records"][0].pop()
        with self.assertRaises(ac.CaptureError):
            ac.analyse(streamer, feeder)
        streamer, feeder = synthetic()
        feeder["header"]["build_id"] = "other"
        with self.assertRaises(ac.CaptureError):
            ac.analyse(streamer, feeder)

    def test_the_command_line_reads_two_files_and_prints_text_or_json(self):
        streamer, feeder = synthetic()
        with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as directory:
            paths = []
            for name, doc in (("7-streamer.json", streamer), ("7-feeder.json", feeder)):
                paths.append(os.path.join(directory, name))
                with open(paths[-1], "w", encoding="utf-8") as handle:
                    json.dump(doc, handle)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(ac.main(paths), 0)
            self.assertIn("Join by epoch + unrounded wire key", out.getvalue())
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(ac.main(paths + ["--json"]), 0)
            self.assertEqual(json.loads(out.getvalue())["header"]["frame_pacing"], "Smooth")
            broken = copy.deepcopy(streamer)
            del broken["header"]["build_id"]
            with open(paths[0], "w", encoding="utf-8") as handle:
                json.dump(broken, handle)
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(ac.main(paths), 2)
            self.assertIn("build id", err.getvalue())

    def test_the_prefix_rows_give_the_capture_interval_the_paced_share_and_the_drift_against_rtp(self):
        streamer, feeder = synthetic()
        table = add_prefix(streamer, drift_ppm=500, paced=lambda i: [None, 0, 1][i % 3])
        table[6][12] = 25
        table[3][14] = {"total": 6, "records": [[0x203, 3, 0x03_0201], [0x204, 1, 7]]}
        table[1][10], table[2][10] = 1_000_000, 1_016_667
        table.append([200, 77, 9, "absent", 0x01, 0, None, None, None, None, None, None, None, None, None])
        table.append([201, 78, 10, "malformed", 0x81, 3, None, None, None, None, None, None, None, None, None])
        report = ac.analyse(streamer, feeder)
        prefix = report["frame_prefix"]
        self.assertEqual(prefix["status"], {"read": 29, "absent": 1, "malformed": 1})
        self.assertEqual(prefix["present"], {"capture_time_us": 29, "sender_paced": 18, "presentation_time_us": 2, "access_unit_length": 29})
        interval = prefix["capture_interval"]
        self.assertEqual((interval["pairs"], interval["skipped"], interval["not_forward"], interval["constant"]), (26, 4, 0, False))
        self.assertAlmostEqual(interval["ms"]["min"], 750 * 1000 / HZ * 1.0005, delta=0.002)
        self.assertAlmostEqual(interval["ms"]["max"], 1500 * 1000 / HZ * 1.0005, delta=0.002)
        self.assertEqual(interval["buckets_ms"], [[8.25, 13], [16.5, 13]])
        self.assertAlmostEqual(interval["median_vs_expected"], 1.5, delta=0.01)
        paced = prefix["sender_paced"]
        self.assertEqual((paced["paced"], paced["unpaced"], paced["unknown"], paced["unknown_without_prefix"]), (9, 9, 13, 2))
        self.assertEqual(paced["raw"], {"unknown": 13, "0": 9, "1": 9})
        self.assertEqual(paced["paced_share_of_known"], 0.5)
        drift = prefix["capture_vs_rtp"]
        self.assertEqual(drift["error_ms"]["n"], 26)
        self.assertGreater(drift["error_ms"]["min"], 0)
        self.assertAlmostEqual(drift["error_ms"]["max"], 1500 * 1000 / HZ * 500e-6, delta=0.0015)
        self.assertEqual(drift["end_to_end"]["pictures"], 29)
        self.assertAlmostEqual(drift["end_to_end"]["rtp_ms"], 350.0, places=6)
        self.assertAlmostEqual(drift["end_to_end"]["drift_ppm"], 500, delta=10)
        self.assertEqual(prefix["presentation_interval"]["pairs"], 1)
        self.assertAlmostEqual(prefix["presentation_interval"]["ms"]["median"], 16.667, places=3)
        self.assertEqual(prefix["other_tags"], {"rows": 1, "total": 6, "kept_by_tag": {"0x203": 1, "0x204": 1}})
        self.assertEqual(prefix["shape"], {"byte0": {"0x01": 1, "0x81": 30}, "header_size": {"0": 1, "3": 1, "8": 29}, "byte2": {"0x01": 29},
                                           "byte3": {"0x02": 29}})
        self.assertEqual(prefix["lengths"], {"compared": 29, "differ": 1, "stripped_unknown": 2})
        self.assertEqual(prefix["coverage"], {"streamer_records": 30, "without_index": 0, "with_prefix_row": 28, "without_prefix_row": 2,
                                              "rows_without_record": 3})
        json.dumps(report)
        text = ac.render(report)
        for needle in ("29 read, 1 absent, 1 malformed", "26 consecutive pairs (4 skipped)", "9 paced, 9 unpaced, 13 unknown (2 without a prefix)",
                       "paced share of the known 50.0%", "ppm)", "does not show that the game's pixels changed", "NVST cloud-gsync log line"):
            self.assertIn(needle, text)
        self.assertFalse(any("flag is unknown for every picture" in note for note in report["notes"]))

    def test_a_stream_that_never_sends_the_paced_record_reports_the_flag_as_unknown_not_off(self):
        streamer, feeder = synthetic()
        add_prefix(streamer)
        report = ac.analyse(streamer, feeder)
        prefix = report["frame_prefix"]
        paced = prefix["sender_paced"]
        self.assertEqual((paced["paced"], paced["unpaced"], paced["unknown"], paced["paced_share_of_known"]), (0, 0, 29, None))
        self.assertEqual(paced["raw"], {"unknown": 29})
        self.assertIn("paced share of the known unknown", ac.render(report))
        self.assertTrue(any("flag is unknown for every picture, not off" in note for note in report["notes"]))
        self.assertLess(abs(prefix["capture_vs_rtp"]["end_to_end"]["drift_ppm"]), 10)
        self.assertLess(abs(prefix["capture_vs_rtp"]["error_ms"]["max"]), 0.0015)

    def test_pictures_the_prefix_log_omitted_are_named_in_the_notes(self):
        streamer, feeder = synthetic()
        add_prefix(streamer)
        streamer["frame_prefix"]["counts"]["omitted"] = 7
        self.assertTrue(any("omitted 7 pictures" in note for note in ac.analyse(streamer, feeder)["notes"]))

    def test_a_capture_without_the_sender_sections_says_not_recorded_and_never_zero(self):
        streamer, feeder = synthetic()
        written_null = copy.deepcopy(streamer)
        written_null["sender_reports"] = written_null["frame_prefix"] = None
        for doc in (streamer, written_null):
            report = ac.analyse(doc, feeder)
            for key in ("report_headers", "timestamp_pairs", "frame_prefix"):
                self.assertEqual(report[key], {"recorded": False})
            text = ac.render(report)
            for line in ("Sender-report headers heard: not recorded", "Usable audio and video timestamp pairs: not recorded",
                         "Per-picture sender prefix: not recorded"):
                self.assertIn(line, text)
            self.assertFalse(any("anchors no clock" in note or "pixels changed" in note for note in report["notes"]))

    def test_report_headers_alone_are_heard_but_make_no_pair(self):
        streamer, feeder = synthetic()
        video = report_track(90_000, 111, header_only=4)
        audio = report_track(48_000, 222, read=3, first=report_sample(222, None, 7), last=report_sample(222, None, 9))
        add_reports(streamer, video, audio)
        report = ac.analyse(streamer, feeder)
        headers, anchors = report["report_headers"], report["timestamp_pairs"]
        self.assertEqual((headers["tracks"]["video"]["heard"], headers["tracks"]["video"]["header_only"], headers["tracks"]["video"]["read"]), (4, 4, 0))
        self.assertEqual((headers["tracks"]["audio"]["heard"], headers["tracks"]["audio"]["read"]), (3, 3))
        self.assertTrue(headers["both_heard"])
        self.assertEqual((anchors["both_read"], anchors["both_usable"]), (False, False))
        for name in ("video", "audio"):
            track = anchors["tracks"][name]
            self.assertEqual((track["first"], track["last"], track["rate_ratio"]), (None, None, None))
        text = ac.render(report)
        for needle in ("both heard: yes", "both usable: no", "0 usable of 3 read", "first none; last none", "RTP over NTP unknown"):
            self.assertIn(needle, text)
        self.assertTrue(any("anchors no clock" in note for note in report["notes"]))

    def test_usable_pairs_on_both_tracks_are_listed_with_the_rate_of_each_clock(self):
        streamer, feeder = synthetic()
        t0, wrap = 3_900_000_000_000_000, ac.WRAP
        video = report_track(90_000, 111, header_only=1, read=3, usable=3, first=report_sample(111, t0, 1000),
                             last=report_sample(111, t0 + 2_000_000, 1000 + 180_000))
        audio = report_track(48_000, 222, read=3, usable=3, first=report_sample(222, t0 + 100, wrap - 1000),
                             last=report_sample(222, t0 + 100 + 2_000_000, (wrap - 1000 + 96_960) % wrap))
        add_reports(streamer, video, audio)
        report = ac.analyse(streamer, feeder)
        anchors = report["timestamp_pairs"]
        self.assertEqual((anchors["both_read"], anchors["both_usable"]), (True, True))
        self.assertAlmostEqual(anchors["tracks"]["video"]["rate_ratio"], 1.0, places=9)
        self.assertAlmostEqual(anchors["tracks"]["audio"]["rate_ratio"], 1.01, places=9)
        self.assertEqual(anchors["tracks"]["audio"]["first"]["rtp"], wrap - 1000)
        text = ac.render(report)
        for needle in ("both tracks read: yes, both usable: yes", f"ntp_us={t0} rtp=1000", "RTP over NTP 1.0100"):
            self.assertIn(needle, text)

    def test_a_prefix_section_off_its_field_list_is_refused_by_name(self):
        base, feeder = synthetic()
        add_prefix(base)

        def refused(edit):
            streamer = copy.deepcopy(base)
            edit(streamer["frame_prefix"])
            with self.assertRaises(ac.CaptureError) as caught:
                ac.analyse(streamer, feeder)
            return str(caught.exception)

        def short_row(section):
            section["rows"][3].pop()

        def no_paced_column(section):
            index = section["fields"].index("sender_paced")
            del section["fields"][index]
            for row in section["rows"]:
                del row[index]

        def odd_status(section):
            section["rows"][3][3] = "weird"

        def read_without_capture_time(section):
            section["rows"][3][8] = None

        self.assertIn("frame_prefix row does not match its field list", refused(short_row))
        self.assertIn("frame_prefix lacks the columns sender_paced", refused(no_paced_column))
        self.assertIn("status other than read, absent or malformed", refused(odd_status))
        self.assertIn("read has no capture time", refused(read_without_capture_time))


if __name__ == "__main__":
    unittest.main()
