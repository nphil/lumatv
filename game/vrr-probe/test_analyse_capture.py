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


if __name__ == "__main__":
    unittest.main()
