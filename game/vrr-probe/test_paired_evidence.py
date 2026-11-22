#!/usr/bin/env python3
"""Tests for paired_evidence: the arithmetic on labelled fixtures, and the historical captures, which hold neither a link witness nor picture ids.

Every fixture time is made up to exercise the arithmetic (each source carries a `:fixture` name). None of it is a measurement of any panel. No real
link-witness name is qualified, so the fixtures qualify their own name through `analyse` below.
"""
import contextlib
import io
import json
import os
import tempfile
import unittest
from fractions import Fraction

import paired_evidence as pe

TICK, TICK2, H = 17.5, 17.0, 0.02
MODE = "3840x2160@120 SDR"
REAL = "/data/agent/cody-data-scratch/7bfa4e8c2bde/vrr-irq-edges"
FIXTURE = frozenset({"fixture"})


def delays(*, ref=2.0, wait=2.0, link=2.0, tick=TICK, drift=0.0, unsplit_ref=0.0, unsplit_link=0.0, held_ref=2.0, held_link=0.0):
    """Delays (ms from a trial's previous link frame start) for the six trials of a pair. Each keyword is how far that step moves from the +11 arm
    to the +13 arm; `drift` moves the next fixed tick itself."""
    out = {}
    for i, arm in enumerate(pe.ARMS):
        t = tick + drift * i
        out[("split", arm)] = {"ready": 11 + 2 * i, "tick": t, "ref": 11.4 + ref * i, "wait": 12.1 + wait * i, "link": 13.6 + link * i}
        out[("unsplit", arm)] = {"ready": 11 + 2 * i, "tick": t, "ref": t + unsplit_ref * i, "wait": t + 0.9, "link": 25.0 + unsplit_link * i}
        out[("held", arm)] = {"ready": None, "tick": t, "ref": 11.4 + held_ref * i, "wait": t + 0.9, "link": 25.0 + held_link * i}
    return out


def stamp(origin, delay, half, kind):
    return {"ms": [round(origin + delay - half, 4), round(origin + delay + half, 4)], "src": kind + ":fixture"}


def build(n, pid, kind, arm, d, h, link_h):
    """One trial. `n` is its place in the file: it sets the picture ids, the retire cookie and the trial's own origin on the shared clock."""
    origin = 1000.0 * n
    previous = retained = 5 if kind == "held" else None
    if previous is None:
        previous, retained = 10 * n, 10 * n + 1
    scans, grid = [(0.0, previous)], 8.333
    while grid < d["link"] - 1.0:          # repeats of the previous picture on a 120 Hz grid
        scans.append((round(grid, 3), previous))
        grid += 8.333
    scans.append((d["link"], retained))
    times = {"decoder_tick": stamp(origin, d["tick"], h, "hw-trace"), "ref_edge": stamp(origin, d["ref"], h, "hw-trace"),
             "trk2_exit": stamp(origin, d["wait"], h, "hw-trace")}
    if d["ready"] is not None:
        times["ready"] = stamp(origin, d["ready"], h, "app")
    return {"pair_id": pid, "arm": arm, "kind": kind, "epoch": 1, "mode": {"name": MODE, "lock": "locked"},
            "picture": {"previous": previous, "retained": retained, "selected": retained, "retired": retained, "retire_cookie": 7000 + n,
                        "admitted": [] if kind == "held" else [retained]},
            "times": times, "link": [dict(stamp(origin, t, link_h, "link-witness"), id=id_) for t, id_ in scans]}


def document(*sets, h=H, link_h=H):
    trials = []
    for p, d in enumerate(sets, 1):
        for k, (kind, arm) in enumerate((kind, arm) for kind in pe.KINDS for arm in pe.ARMS):
            trials.append(build(10 * p + k, f"fixture-{p}", kind, arm, d[(kind, arm)], h, link_h))
    return {"format": pe.FORMAT, "version": 1, "clock": {"calibrated": True}, "witness": {"resolves_repeats": True}, "trials": trials}


def two(**kw):
    """Two pairs at two phases of the fixed tick, both built from the same keywords."""
    return document(delays(**kw), delays(tick=TICK2, **kw))


def synthetic(doc):
    """The same trials with every time marked as a model's, which --allow-synthetic accepts."""
    for t in doc["trials"]:
        for time in [*t["times"].values(), *t["link"]]:
            time["src"] = "synthetic:fixture"
    return doc


def analyse(doc, **kw):
    """The arithmetic with the fixtures' source name qualified, which the real (empty) registry does not do."""
    return pe.analyse(doc, qualified=FIXTURE, **kw)


def find(doc, p, kind, arm):
    return next(t for t in doc["trials"] if (t["pair_id"], t["kind"], t["arm"]) == (f"fixture-{p}", kind, arm))


def centre(interval):
    return sum(interval) / 2


class Arithmetic(unittest.TestCase):
    def test_intervals_are_exact_and_widen_on_subtraction(self):
        d = pe.Interval(1, 2) - pe.Interval(Fraction("0.5"), Fraction("0.75"))
        self.assertEqual(tuple(d), (Fraction("0.25"), Fraction("1.5")))
        self.assertEqual(pe._num(0.1) + pe._num(0.2), pe._num(0.3))
        band = (Fraction("1.75"), Fraction("2.25"))
        self.assertEqual(pe.Interval(*band).relation(*band), "in")
        self.assertEqual(pe.Interval(Fraction("1.7"), 2).relation(*band), "straddle")
        self.assertEqual(pe.Interval(0, Fraction("0.1")).relation(*band), "apart")

    def test_a_true_two_ms_displacement_supports(self):
        report = analyse(two())
        self.assertEqual(report["verdict"], "supports", report["reasons"])
        deltas = report["pairs"][0]["deltas_ms"]
        for step in ("ready", "ref", "wait", "link"):
            self.assertAlmostEqual(centre(deltas["split"][step]), 2.0, places=6)
        self.assertAlmostEqual(centre(deltas["split"]["tick"]), 0.0, places=6)
        self.assertAlmostEqual(centre(deltas["unsplit"]["link"]), 0.0, places=6)
        self.assertAlmostEqual(centre(deltas["held"]["ref"]), 2.0, places=6)
        self.assertIn("discriminates the mechanism", report["notes"][-1])      # two pairs tell the mechanism apart; they do not qualify the product
        self.assertIn("does not qualify the product", report["notes"][-1])

    def test_one_pair_is_too_few_for_a_verdict(self):
        report = analyse(document(delays()))
        self.assertEqual((report["pairs"][0]["verdict"], report["verdict"], report["reasons"]), ("supports", "inconclusive", ["too_few_pairs"]))
        self.assertEqual(analyse(document(delays()), min_pairs=1)["verdict"], "supports")

    def test_a_fixed_tick_phase_alias_is_not_supports(self):
        # The +13 arm's previous link frame start sits 2 ms further from the fixed tick, so a handoff that follows the tick looks like +2 ms everywhere.
        alias = dict(drift=2.0, unsplit_link=2.0, held_link=2.0)
        report = analyse(document(delays(**alias), delays(tick=TICK2, **alias)))
        self.assertAlmostEqual(centre(report["pairs"][0]["deltas_ms"]["split"]["link"]), 2.0, places=6)
        self.assertEqual(report["verdict"], "inconclusive")
        self.assertIn("phase_mismatch", report["reasons"])

    def test_chain_outcomes(self):
        cases = {
            "supports": (dict(), "supports"),
            "reference moved, output fixed": (dict(wait=0.0, link=0.0), "rejects-in-configuration"),
            "output followed, wait did not": (dict(wait=0.0), "inconclusive"),
            "output followed, reference fixed": (dict(ref=0.0), "inconclusive"),
            "nothing moved": (dict(ref=0.0, wait=0.0, link=0.0), "inconclusive"),
            "output moved by neither 0 nor 2": (dict(link=5.0), "inconclusive"),
        }
        for label, (kw, expected) in cases.items():
            with self.subTest(label):
                self.assertEqual(analyse(two(**kw))["verdict"], expected)
        self.assertIn("chain_incoherent", analyse(two(wait=0.0))["reasons"])
        self.assertIn("reference_did_not_move", analyse(two(ref=0.0, wait=0.0, link=0.0))["reasons"])
        self.assertIn("link_displacement_unexplained", analyse(two(link=5.0))["reasons"])
        fixed_ref = analyse(two(ref=0.0))
        self.assertIn("reference_fixed_output_followed", fixed_ref["reasons"])
        shifts = fixed_ref["pairs"][0]["shifts"]
        self.assertEqual(shifts["split"], {"ready": "follows", "tick": "fixed", "ref": "fixed", "wait": "follows", "link": "follows"})
        self.assertEqual((shifts["unsplit"]["link"], shifts["held"]["ref"]), ("fixed", "follows"))

    def test_ambiguous_brackets_are_inconclusive(self):
        wide = analyse(document(delays(), delays(tick=TICK2), link_h=0.1))
        self.assertEqual((wide["verdict"], "displacement_unresolved:link" in wide["reasons"]), ("inconclusive", True))
        self.assertEqual(analyse(document(delays(wait=0.0, link=0.0), delays(tick=TICK2, wait=0.0, link=0.0), link_h=0.1))["verdict"], "inconclusive")
        doc = two()
        scans = find(doc, 1, "split", "+13")["link"]
        scans[-2]["ms"] = list(scans[-1]["ms"])
        self.assertIn("phase_ambiguous", analyse(doc)["reasons"])
        late = analyse(document(delays(tick=12.0), delays(tick=12.5)))        # +13 ms eligibility with the next fixed tick at +12 ms
        self.assertIn("eligibility_after_tick", late["reasons"])

    def test_alignment_uncertainty_counts_against_the_tolerance(self):
        doc = two()
        doc["clock"]["alignment_ms"] = {"link-witness": 0.2}
        report = analyse(doc)
        self.assertEqual((report["verdict"], "displacement_unresolved:link" in report["reasons"]), ("inconclusive", True))
        doc["clock"]["calibrated"] = False
        self.assertIn("clock_uncalibrated", analyse(doc)["reasons"])

    def test_ids_must_be_resolved_and_in_order(self):
        def edit(*path, value=None):
            def apply(doc):
                node = find(doc, 1, "split", "+13")
                for step in path[:-1]:
                    node = node[step]
                node[path[-1]] = value
            return apply

        def reorder(doc):
            a, b = find(doc, 1, "split", "+11")["picture"], find(doc, 1, "split", "+13")["picture"]
            a["selected"], b["selected"] = b["selected"], a["selected"]

        def share_cookie(doc):
            find(doc, 1, "split", "+13")["picture"]["retire_cookie"] = find(doc, 1, "split", "+11")["picture"]["retire_cookie"]

        def drop_witness(doc):
            del doc["witness"]

        cases = [("reordered selected ids", reorder, "id_mismatch"), ("link shows another picture", edit("link", -1, "id", value=9999), "id_mismatch"),
                 ("a repeat scan without an id", edit("link", 1, "id"), "repeat_unresolved"), ("the next scan without an id", edit("link", -1, "id"), "id_unresolved"),
                 ("no retire cookie", edit("picture", "retire_cookie"), "retire_cookie_missing"), ("no retained id", edit("picture", "retained"), "id_unresolved"),
                 ("a cookie reused", share_cookie, "id_not_unique"), ("repeats not resolvable", drop_witness, "repeat_unresolved")]
        for label, change, code in cases:
            with self.subTest(label):
                doc = two()
                change(doc)
                report = analyse(doc)
                self.assertEqual(report["verdict"], "inconclusive")
                self.assertIn(code, report["reasons"])

    def test_each_arm_admits_one_picture(self):
        cases = [("not recorded", "split", lambda p: p.pop("admitted"), "admitted_unresolved"),
                 ("two pictures", "split", lambda p: p.update(admitted=[p["retained"], p["retained"] + 1]), "selection_ambiguous"),
                 ("another picture", "split", lambda p: p.update(admitted=[p["retained"] + 1]), "selection_ambiguous"),
                 ("none", "unsplit", lambda p: p.update(admitted=[]), "selection_ambiguous"),
                 ("a new picture in a held arm", "held", lambda p: p.update(admitted=[p["retained"] + 1]), "selection_ambiguous")]
        for label, kind, change, code in cases:
            with self.subTest(label):
                doc = two()
                change(find(doc, 1, kind, "+13")["picture"])
                report = analyse(doc)
                self.assertEqual(report["verdict"], "inconclusive")
                self.assertIn(code, report["reasons"])

    def test_the_held_picture_ends_at_the_first_link_start_after_the_tick(self):
        def held(doc):
            return find(doc, 1, "held", "+13")

        def edge_past_the_end(doc):       # a start after the first one at or after the tick: the held picture ended earlier
            link = held(doc)["link"]
            lo, hi = link[-1]["ms"]
            link.append({"ms": [lo + 8.333, hi + 8.333], "id": 5, "src": "link-witness:fixture"})

        def stops_early(doc):             # the list ends before the tick
            del held(doc)["link"][2:]

        def tick_on_a_start(doc):         # the tick overlaps the repeat at 16.666: whether that start is before or after it is not certain
            origin = sum(held(doc)["link"][0]["ms"]) / 2
            held(doc)["times"]["decoder_tick"]["ms"] = [round(origin + 16.6, 4), round(origin + 16.75, 4)]

        cases = [("an edge past the first start after the tick", edge_past_the_end, "held_end_not_first_after_tick"),
                 ("the list stops before the tick", stops_early, "held_end_before_tick"),
                 ("the tick overlaps a start", tick_on_a_start, "tick_order_ambiguous")]
        self.assertEqual(analyse(two())["verdict"], "supports")
        for label, change, code in cases:
            with self.subTest(label):
                doc = two()
                change(doc)
                report = analyse(doc)
                self.assertEqual(report["verdict"], "inconclusive")
                self.assertIn(code, report["reasons"])

    def test_only_a_link_witness_counts_as_a_link_scan(self):
        def link_src(src):
            def apply(doc):
                for t in doc["trials"]:
                    for scan in t["link"]:
                        scan["src"] = src
            return apply

        def time_src(channel, src):
            return lambda doc: find(doc, 1, "split", "+11")["times"][channel].update(src=src)

        cases = [("irq", link_src("irq:irq104"), "provenance_not_link_evidence:irq"), ("ack", link_src("ack:feeder"), "provenance_not_link_evidence:ack"),
                 ("synthetic", link_src("synthetic:model"), "provenance_synthetic"), ("a trace as the link", link_src("hw-trace:x"), "provenance_wrong_kind:link:hw-trace"),
                 ("a witness as the reference", time_src("ref_edge", "link-witness:x"), "provenance_wrong_kind:ref_edge:link-witness"),
                 ("irq as the tick", time_src("decoder_tick", "irq:irq104"), "provenance_wrong_kind:decoder_tick:irq"),
                 ("unknown", link_src("flux:x"), "provenance_unknown:flux"), ("unnamed", link_src(""), "provenance_missing:link")]
        for label, change, code in cases:
            with self.subTest(label):
                doc = two()
                change(doc)
                report = analyse(doc)
                self.assertEqual(report["verdict"], "inconclusive")
                self.assertIn(code, report["reasons"])
        doc = two()
        link_src("synthetic:model")(doc)
        report = analyse(doc, allow_synthetic=True)
        self.assertEqual((report["verdict"], report["synthetic"]), ("supports", True))
        self.assertIn("SYNTHETIC", report["notes"][0])

    def test_no_link_witness_is_qualified_yet(self):
        self.assertEqual(pe.QUALIFIED_LINK_WITNESSES, frozenset())
        report = pe.analyse(two())                                  # the registry decides, and the fixture's name is not in it
        self.assertEqual(report["verdict"], "inconclusive")
        self.assertIn("link_witness_unqualified:fixture", report["reasons"])
        doc = two()                                                 # a record cannot qualify its own source
        doc["witness"]["qualified"] = doc["clock"]["qualified"] = ["fixture"]
        self.assertIn("link_witness_unqualified:fixture", pe.analyse(doc)["reasons"])
        unnamed = two()
        for t in unnamed["trials"]:
            for scan in t["link"]:
                scan["src"] = "link-witness"
        self.assertIn("link_witness_unqualified:unnamed", analyse(unnamed)["reasons"])
        self.assertEqual(analyse(two())["verdict"], "supports")     # only the caller's list changes that

    def test_irq_tout_crc_and_mcu_are_never_link_witnesses(self):
        for name in ("irq104", "osd_irq", "IRQ-104", "tout-crc", "factory_crc", "mcu-request"):
            with self.subTest(name):
                doc = two()
                for t in doc["trials"]:
                    for scan in t["link"]:
                        scan["src"] = "link-witness:" + name
                report = pe.analyse(doc, qualified={name})          # not even when the caller lists the name as qualified
                self.assertEqual(report["verdict"], "inconclusive")
                self.assertIn("link_witness_forbidden_source:" + name, report["reasons"])
        self.assertFalse(any(pe.NEVER_LINK_WITNESS.search(name) for name in {*pe.QUALIFIED_LINK_WITNESSES, "fixture"}))

    def test_lock_mode_and_epoch_never_reject(self):
        def lock(value):
            return lambda doc: find(doc, 1, "held", "+13")["mode"].update(lock=value)

        cases = [("lost", lock("lost"), "lock_lost"), ("relocked", lock("relocked"), "lock_lost"), ("unknown", lock(None), "lock_unknown"),
                 ("mode", lambda d: find(d, 1, "split", "+13")["mode"].update(name="3840x2160@60"), "mode_mismatch"),
                 ("epoch", lambda d: find(d, 1, "split", "+13").update(epoch=2), "epoch_mismatch")]
        self.assertEqual(analyse(two(wait=0.0, link=0.0))["verdict"], "rejects-in-configuration")
        for label, change, code in cases:
            with self.subTest(label):
                doc = two(wait=0.0, link=0.0)
                change(doc)
                report = analyse(doc)
                self.assertEqual(report["verdict"], "inconclusive")
                self.assertIn(code, report["reasons"])

    def test_a_bad_or_missing_control_is_inconclusive(self):
        def drop(kind):
            def apply(doc):
                doc["trials"] = [t for t in doc["trials"] if t["kind"] != kind]
            return apply

        def unsplit_eligibility(doc):
            ms = find(doc, 1, "unsplit", "+13")["times"]["ready"]["ms"]
            ms[:] = [ms[0] + 1.0, ms[1] + 1.0]

        cases = [("unsplit moves the link", two(unsplit_link=2.0), None, "control_explains_shift:unsplit"),
                 ("unsplit moves the reference", two(unsplit_ref=2.0), None, "control_reference_moved:unsplit"),
                 ("held moves the link", two(held_link=2.0), None, "control_explains_shift:held"),
                 ("held reference unchanged", two(held_ref=0.0), None, "control_reference_not_changed:held"),
                 ("held changes picture", two(), lambda d: find(d, 1, "held", "+13")["link"][-1].update(id=77), "id_mismatch"),
                 ("no held control", two(), drop("held"), "control_missing:held"), ("no unsplit control", two(), drop("unsplit"), "control_missing:unsplit"),
                 ("unsplit has other eligibility", two(), unsplit_eligibility, "control_eligibility_differs:unsplit")]
        for label, doc, change, code in cases:
            with self.subTest(label):
                if change:
                    change(doc)
                report = analyse(doc)
                self.assertEqual(report["verdict"], "inconclusive")
                self.assertIn(code, report["reasons"])

    def test_pairs_that_disagree_are_inconclusive(self):
        doc = document(delays(), delays(tick=TICK2, wait=0.0, link=0.0))
        report = analyse(doc)
        self.assertEqual([p["verdict"] for p in report["pairs"]], ["supports", "rejects-in-configuration"])
        self.assertEqual((report["verdict"], report["reasons"]), ("inconclusive", ["pairs_disagree"]))


IRQ_CSV = """# clock=CLOCK_MONOTONIC irq=104 initial=15568704 start_ns=130751542930820 last_read_ns=130753542891640
previous_read_start_ns,current_read_end_ns,increment
130751547201040,130751547358760,1
130751555522140,130751555681240,1
130751556935020,130751572385340,2
"""
UNAVAILABLE = ["decoder completion", "selected-frame id", "release", "optical times"]
FEEDER = {"format": pe.CAPTURE_FORMAT, "version": 1, "producer": "feeder", "clock": "CLOCK_MONOTONIC", "complete": True, "header": {"unavailable": UNAVAILABLE},
          "fields": ["epoch", "seq", "wire_key_ns", "bytes", "key", "recv_ns", "first_call_ns", "last_return_ns", "attempts", "player_ms", "play_rc", "queue",
                     "wait_us", "reason"],
          "records": [[1, 0, 130751515244360, 7661, 1, 130751515355180, 130751515494160, 130751515617660, 1, 1407, 0, 0, 261, 0]]}
STREAMER = {"format": pe.CAPTURE_FORMAT, "version": 1, "producer": "streamer", "clock": "CLOCK_MONOTONIC", "complete": True, "header": {"unavailable": UNAVAILABLE},
            "fields": ["epoch", "seq", "flags", "frame_index", "rtp_timestamp", "rtp_clock_hz", "bytes", "sink_entry_ns", "release_ns", "wire_key_ns",
                       "handover_done_ns", "held_at_entry", "in_flight_at_send"],
            "records": [[1, 0, 687, 1, 621620, 90000, 7661, 130751515242900, 130751515242900, 130751515244360, 130751515262500, 0, 1]]}
HISTORY_REASONS = {"no_trials", "link_witness_missing", "provenance_not_link_evidence:irq", "id_unresolved", "retire_cookie_missing", "clock_uncalibrated",
                   "repeat_unresolved", "witness_missing:ready", "witness_missing:decoder_tick", "witness_missing:ref_edge", "witness_missing:trk2_exit"}


class Historical(unittest.TestCase):
    def check(self, csv_path, *captures):
        report = pe.analyse(pe.historical(csv_path, captures))
        self.assertEqual((report["verdict"], report["pairs"]), ("inconclusive", []))
        self.assertTrue(HISTORY_REASONS <= set(report["reasons"]), HISTORY_REASONS - set(report["reasons"]))

    def test_trimmed_copies_of_the_irq_trace_and_capture(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = []
            for name, body in (("irq.csv", IRQ_CSV), ("feeder.json", json.dumps(FEEDER)), ("streamer.json", json.dumps(STREAMER))):
                paths.append(os.path.join(tmp, name))
                with open(paths[-1], "w", encoding="utf-8") as handle:
                    handle.write(body)
            self.check(*paths)
            doc = pe.historical(*paths[:1], paths[1:])
            self.assertEqual(doc["inventory"]["channels"]["link"], [{"src": "irq:irq104", "rows": 3, "increments": 4}])
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(pe.main(["--historical", paths[0], paths[1], paths[2]]), 0)
            self.assertIn("Verdict: inconclusive", out.getvalue())
            self.assertIn("does not close either hardware gate", out.getvalue())

    @unittest.skipUnless(os.path.isfile(f"{REAL}/irq-live.csv"), "the scratch captures are not on this machine")
    def test_the_recorded_irq_trace_and_captures(self):
        self.check(f"{REAL}/irq-live.csv", f"{REAL}/1795339332000-feeder.json", f"{REAL}/1795339332000-streamer.json")


class Command(unittest.TestCase):
    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = pe.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def test_json_report_and_bad_input(self):
        with tempfile.TemporaryDirectory() as tmp:
            model, named, bad = (os.path.join(tmp, name) for name in ("model.json", "trials.json", "other.json"))
            for path, body in ((model, synthetic(two())), (named, two()), (bad, {"format": "something else"})):
                with open(path, "w", encoding="utf-8") as handle:
                    json.dump(body, handle)
            code, out, _ = self.run_main(model, "--json", "--allow-synthetic")
            self.assertEqual((code, json.loads(out)["verdict"], json.loads(out)["synthetic"]), (0, "supports", True))
            code, out, _ = self.run_main(model, "--allow-synthetic")
            self.assertEqual((code, "Verdict: supports" in out, "does not close either hardware gate" in out), (0, True, True))
            code, out, _ = self.run_main(model)
            self.assertEqual((code, "Verdict: inconclusive" in out, "provenance_synthetic" in out), (0, True, True))
            code, out, _ = self.run_main(named, "--json")           # no source is qualified, and the command line cannot qualify one
            self.assertEqual((code, json.loads(out)["verdict"], "link_witness_unqualified:fixture" in json.loads(out)["reasons"]), (0, "inconclusive", True))
            code, _, err = self.run_main(bad)
            self.assertEqual((code, "paired_evidence:" in err), (2, True))
            self.assertEqual(self.run_main(os.path.join(tmp, "missing.json"))[0], 2)


if __name__ == "__main__":
    unittest.main()
