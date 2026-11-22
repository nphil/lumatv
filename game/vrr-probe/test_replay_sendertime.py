import contextlib
import io
import json
import os
import tempfile
import unittest

import analyse_capture as ac
import replay_sendertime as rs
from test_analyse_capture import LOAD_NS, synthetic


def ideal(lines):
    """A stand-in for the feeder's mapping: anchored at the first picture's own stamp, then the sender's time rounded up."""
    first_rtp, first_local = lines[0][0], lines[0][2]
    return [(first_local + -(-((rtp - first_rtp) % ac.WRAP) * 1000 // hz), 1, int(i == 0)) for i, (rtp, hz, _, _) in enumerate(lines)]


class ReplaySenderTimeTests(unittest.TestCase):
    def test_the_mapping_follows_the_sender_where_todays_stamps_carry_the_handover_jitter(self):
        streamer, feeder = synthetic()
        for row in feeder["records"][:-1:2]:
            row[9] += 4  # the player stamp of every other picture is 4 ms late
        report = rs.replay(streamer, feeder, ideal)
        self.assertEqual((report["pictures"], report["intervals"]), (29, 27))
        self.assertEqual(report["mapped"]["within_1ms"], 27)
        self.assertLess(report["mapped"]["error_ms"]["max"], 1.0)
        self.assertLess(report["today"]["within_1ms"], 27)
        self.assertEqual((report["mapped"]["went_back"], report["epochs"]["count"]), (0, 1))
        self.assertLessEqual(report["offset_to_today_ms"]["max"], 5, "mapped minus today's stamp")

    def test_a_reported_gap_reaches_the_mapping_and_a_reload_starts_it_again(self):
        streamer, feeder = synthetic()
        streamer["records"][5][2] &= ~ac.F_CONTIGUOUS
        feeder["header"]["epochs"] = [{"epoch": 5, "load_ns": LOAD_NS}, {"epoch": 6, "load_ns": LOAD_NS + 1_000_000_000}]
        for row in feeder["records"][:3]:
            row[0] = 6
        calls = []

        def recording(lines):
            calls.append(lines)
            return ideal(lines)

        rs.replay(streamer, feeder, recording)
        self.assertEqual([len(lines) for lines in calls], [3, 26], "one run per feeder epoch")
        self.assertEqual([[i for i, line in enumerate(lines) if line[3]] for lines in calls], [[], [2]], "frame 5 is the third of the second run")

    def test_a_picture_without_a_player_stamp_is_left_out_and_counted(self):
        streamer, feeder = synthetic()
        feeder["records"][4][9] = None
        report = rs.replay(streamer, feeder, ideal)
        self.assertEqual((report["without_player_stamp"], report["pictures"], report["intervals"]), (1, 28, 25))

    def test_the_report_names_the_epochs_the_mapping_opened(self):
        streamer, feeder = synthetic()
        text = rs.render(rs.replay(streamer, feeder, lambda lines: [(local, 1 + i, 4 if i == 3 else 0) for i, (_, _, local, _) in enumerate(lines)]))
        for needle in ("29 pictures", "27 intervals", "sender-time mapping 27/27 (100.0%)", "picture 3: offset bound", "Not shown: A/V alignment"):
            self.assertIn(needle, text)

    def test_the_mapper_is_run_with_one_line_per_picture_and_its_failures_are_named(self):
        lines = [(595241, 90000, 1219, 0), (595991, 90000, 1220, 1)]
        with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as directory:
            def stub(body):
                path = os.path.join(directory, "vclock_test")
                with open(path, "w", encoding="utf-8") as handle:
                    handle.write("#!/bin/sh\n" + body)
                os.chmod(path, 0o755)
                return path

            echo = stub('test "$1" = --replay || exit 9\nawk \'{ print $1 + $4, $2, $3 }\'\n')
            self.assertEqual(rs.run_mapper(echo)(lines), [(595241, 90000, 1219), (595992, 90000, 1220)])
            with self.assertRaisesRegex(ac.CaptureError, "exited 3"):
                rs.run_mapper(stub("exit 3\n"))(lines)
            with self.assertRaisesRegex(ac.CaptureError, "answered 1 lines for 2"):
                rs.run_mapper(stub("echo 1 1 0\n"))(lines)
            with self.assertRaisesRegex(ac.CaptureError, "build it"):
                rs.run_mapper(os.path.join(directory, "absent"))(lines)

    def test_the_command_line_reads_two_files_and_names_a_missing_mapper(self):
        streamer, feeder = synthetic()
        with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as directory:
            paths = []
            for name, doc in (("7-streamer.json", streamer), ("7-feeder.json", feeder)):
                paths.append(os.path.join(directory, name))
                with open(paths[-1], "w", encoding="utf-8") as handle:
                    json.dump(doc, handle)
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(rs.main(paths + ["--mapper", os.path.join(directory, "absent")]), 2)
            self.assertIn("build it", err.getvalue())


if __name__ == "__main__":
    unittest.main()
