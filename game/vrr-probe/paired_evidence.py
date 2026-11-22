#!/usr/bin/env python3
"""Paired-evidence analyzer for the split-reference VRR test (docs/research/decoded-vrr-4k.md, "Split-reference decision test").

    python3 paired_evidence.py trials.json [--json] [--min-pairs N] [--allow-synthetic]
    python3 paired_evidence.py --historical irq-live.csv <nonce>-streamer.json <nonce>-feeder.json [--json]

Host-only arithmetic over trial records somebody else recorded. It captures nothing, touches no TV and switches nothing, and it does not close either
hardware gate (a qualified output observer, a bounded rollback): it only says what a set of records would show once those exist. The verdict is
`supports`, `rejects-in-configuration` or `inconclusive`. Only complete, consistent evidence can leave `inconclusive`: a missing witness, an unresolved
picture id or repeat scan, a bad control, an ambiguous phase, a mismatched mode or a lost lock never rejects.

A trial is one tagged, retained picture made eligible 11 or 13 ms after a previous link frame start. A pair is six trials that share one previous
link phase and one next fixed tick: `split` (the candidate), `unsplit` (the same eligibility, reference unchanged) and `held` (the same picture kept
on screen, reference changed), each at `+11` and `+13`. Times are milliseconds on one calibrated clock, as `[lo, hi]` intervals that already contain
their own uncertainty; `clock.alignment_ms` widens every interval of a provenance kind further. Every delay is measured from the trial's previous
link frame start and subtracted with interval arithmetic (the widths add, as worst cases), then the `+13` delay minus the `+11` delay is the paired
displacement. A displacement is judged against a band of +/-0.25 ms around its target, and counts only when the whole interval is inside or outside.
Each displacement also reads `follows` (the +2 ms eligibility shift), `fixed`, `other` or `unresolved`. The verdict is for the split reference only: the
paired release discriminator (no actuator, so `split` and `unsplit` coincide) shows as `control_explains_shift:unsplit` with those readings and has no verdict here.

    {"format": "lumatv-vrr-paired-evidence", "version": 1,
     "clock": {"calibrated": true, "alignment_ms": {"app": 0.05}},
     "witness": {"resolves_repeats": true},
     "trials": [{"pair_id": "p1", "arm": "+11", "kind": "split", "epoch": 3,
                 "mode": {"name": "3840x2160@120 SDR", "lock": "locked"},
                 "picture": {"previous": 100, "retained": 101, "selected": 101, "retired": 101, "retire_cookie": 7},
                 "times": {"ready": T, "decoder_tick": T, "ref_edge": T, "trk2_exit": T},
                 "link": [{"ms": [lo, hi], "id": 100, "src": "link-witness"}, ...]}]}      T = {"ms": [lo, hi], "src": "app"}

`link` lists every link frame start from the previous one to the next one, repeats included. The first carries the previous picture, the last the
retained one (`split`, `unsplit`); for `held`, whose picture never changes, the last is the first frame start at or after the fixed tick. `ready` is
not needed for `held`. Each `src` is `<kind>` or `<kind>:<name>`; the kind says what produced the time:

    link-witness  an independent observer of the output link or panel scan: the only kind that counts as a link frame start
    hw-trace      a hardware-stamped SoC event: decoder tick, selected reference edge, TRK2 wait exit (and ready, if decode completion is traced)
    app           an application timestamp, such as the presenter's `ready_ns` (ready only)
    irq           IRQ counts and software vsync events (IRQ 104 is osd_irq): mixer corroboration, never evidence of a link scan
    ack           submit, ACK/DROP and play() times: the compressed-input handshake, never ownership, readiness or a link scan
    synthetic     a fixture or a model: refused unless --allow-synthetic, and then the report says it is arithmetic only
"""
import argparse
import csv
import json
import re
import sys
from fractions import Fraction

FORMAT = "lumatv-vrr-paired-evidence"
CAPTURE_FORMAT = "lumagame-vrr-pts-capture"
ARMS = ("+11", "+13")
KINDS = ("split", "unsplit", "held")
TARGET_MS = Fraction(2)
TOLERANCE_MS = Fraction(1, 4)
MIN_PAIRS = 2
CHANNELS = ("ready", "decoder_tick", "ref_edge", "trk2_exit")
LINK, HW, APP, IRQ, ACK, SYNTHETIC = "link-witness", "hw-trace", "app", "irq", "ack", "synthetic"
ACCEPTED = {"link": (LINK,), "ready": (APP, HW), "decoder_tick": (HW,), "ref_edge": (HW,), "trk2_exit": (HW,)}
NOT_EVIDENCE = {
    IRQ: "IRQ counts and software vsync events corroborate the mixer only; they are not a panel-link witness",
    ACK: "submit, ACK/DROP and play() times show the compressed-input handshake; they are not picture ownership, readiness or a link scan",
}
UNAVAILABLE_CODES = {
    "optical times": "link_witness_missing",
    "selected-frame id": "id_unresolved",
    "release": "retire_cookie_missing",
    "decoder completion": "witness_missing:ready",
}
VERDICTS = ("supports", "rejects-in-configuration", "inconclusive")
CANNOT_CLOSE = ("This does not close either hardware gate. Whether the records come from a qualified, independent output observer, and whether a "
                "bounded rollback exists, stay open; this only applies interval arithmetic to records that exist. A `supports` verdict is one "
                "controllable displacement in one configuration, not the panel's VRR range or safe product operation.")
SYNTHETIC_NOTE = "SYNTHETIC input: the arithmetic was exercised on fixture times. This is not evidence about any panel."
STEPS = ("ready", "tick", "ref", "wait", "link")


class EvidenceError(Exception):
    """A document that is not paired evidence at all. Evidence that is merely incomplete becomes a reason in the report instead."""


class Interval(tuple):
    """`[lo, hi]` in ms, exact. Subtracting widens by both operands: the worst case, not a statistical combination."""
    __slots__ = ()

    def __new__(cls, lo, hi):
        return super().__new__(cls, (Fraction(lo), Fraction(hi)))

    lo = property(lambda self: self[0])
    hi = property(lambda self: self[1])

    def __sub__(self, other):
        return Interval(self.lo - other.hi, self.hi - other.lo)

    def widen(self, by):
        return Interval(self.lo - by, self.hi + by)

    def relation(self, lo, hi):
        """`in` when the whole interval is inside [lo, hi], `apart` when none of it is, otherwise `straddle`."""
        if lo <= self.lo and self.hi <= hi:
            return "in"
        return "apart" if self.hi < lo or self.lo > hi else "straddle"

    def json(self):
        return [round(float(self.lo), 6), round(float(self.hi), 6)]


class Problems(list):
    """What stops a verdict, as machine-readable codes with the place they were found."""
    synthetic = False

    def add(self, code, where, detail="", pair=None):
        self.append({"code": code, "where": where, "detail": detail, "pair": pair})


def _num(x):
    if isinstance(x, bool) or not isinstance(x, (int, float)):
        raise ValueError(f"{x!r} is not a number")
    return Fraction(repr(x))


def _key(value):
    """A picture id or retire cookie as a comparable string. None stays None: it was not recorded."""
    return None if value is None else json.dumps(value, sort_keys=True)


def _interval(raw):
    ms = raw.get("ms") if isinstance(raw, dict) else None
    if not (isinstance(ms, list) and len(ms) == 2):
        raise ValueError("a time is {\"ms\": [lo, hi], \"src\": ...}")
    lo, hi = _num(ms[0]), _num(ms[1])
    if lo > hi:
        raise ValueError("lo is above hi")
    return Interval(lo, hi)


def _kind(src, channel, add, problems, allow_synthetic):
    """The provenance kind of one time, or None (with the reason added) when it cannot stand for `channel`."""
    kind = src.partition(":")[0] if isinstance(src, str) else ""
    if not kind:
        add("provenance_missing:" + channel, "every time names where it came from")
    elif kind == SYNTHETIC:
        if allow_synthetic:
            problems.synthetic = True
            return kind
        add("provenance_synthetic", "fixture times are not evidence")
    elif kind in NOT_EVIDENCE:
        add("provenance_not_link_evidence:" + kind if channel == "link" else f"provenance_wrong_kind:{channel}:{kind}", NOT_EVIDENCE[kind])
    elif kind not in {LINK, HW, APP}:
        add("provenance_unknown:" + kind, f"{src!r} is not a known provenance kind")
    elif kind not in ACCEPTED[channel]:
        add(f"provenance_wrong_kind:{channel}:{kind}", f"{channel} comes from {' or '.join(ACCEPTED[channel])}")
    else:
        return kind
    return None


def _observed(raw, channel, add, problems, align, allow_synthetic):
    try:
        interval = _interval(raw)
    except ValueError as error:
        add("bad_interval:" + channel, str(error))
        return None
    kind = _kind(raw.get("src"), channel, add, problems, allow_synthetic)
    return None if kind is None else interval.widen(align.get(kind, 0))


def _read_trial(raw, problems, align, allow_synthetic):
    """One trial, checked as far as it can be alone. Every defect of the evidence becomes a problem tagged with its pair."""
    if not isinstance(raw, dict):
        raise EvidenceError("a trial must be an object")
    pid, arm, kind = raw.get("pair_id"), raw.get("arm"), raw.get("kind")
    if pid is None or isinstance(pid, (dict, list)):
        raise EvidenceError("a trial needs a scalar pair_id")
    where = f"{pid}/{kind}/{arm}"
    mine = Problems()

    def add(code, detail=""):
        mine.add(code, where, detail, pair=pid)

    mode = raw.get("mode") if isinstance(raw.get("mode"), dict) else {}
    pic = raw.get("picture") if isinstance(raw.get("picture"), dict) else {}
    times = raw.get("times") if isinstance(raw.get("times"), dict) else {}
    ids = {name: _key(pic.get(name)) for name in ("previous", "retained", "selected", "retired")}
    cookie = _key(pic.get("retire_cookie"))
    trial = {"pair": pid, "arm": arm, "kind": kind, "where": where, "epoch": raw.get("epoch"), "mode": mode.get("name"), "lock": mode.get("lock"),
             "ids": ids, "cookie": cookie, "link": None, "problems": mine}
    if arm not in ARMS or kind not in KINDS:
        add("trial_malformed", f"arm {arm!r} and kind {kind!r} must be one of {ARMS} and {KINDS}")

    for channel in CHANNELS:
        trial[channel] = None
        if times.get(channel) is not None:
            trial[channel] = _observed(times[channel], channel, add, mine, align, allow_synthetic)
        elif channel != "ready" or kind != "held":
            add("witness_missing:" + channel)

    scans = raw.get("link")
    if not isinstance(scans, list) or len(scans) < 2:
        add("link_witness_missing", "the previous and the next link frame start are both needed")
    else:
        read = [(_observed(s, "link", add, mine, align, allow_synthetic), _key(s.get("id") if isinstance(s, dict) else None)) for s in scans]
        if all(interval is not None for interval, _ in read):
            trial["link"] = [{"iv": interval, "id": id_} for interval, id_ in read]
            if any(not a["iv"].hi < b["iv"].lo for a, b in zip(trial["link"], trial["link"][1:])):
                add("phase_ambiguous", "two link frame start brackets overlap, so their order and count are not certain")
    problems.synthetic = problems.synthetic or mine.synthetic

    missing = [name for name, value in ids.items() if value is None]
    if missing:
        add("id_unresolved", "not recorded: " + ", ".join(missing))
    elif kind == "held":
        if len(set(ids.values())) != 1:
            add("id_mismatch", f"a held picture keeps one id, got {ids}")
    elif not ids["retained"] == ids["selected"] == ids["retired"] or ids["retained"] == ids["previous"]:
        add("id_mismatch", f"the retained picture is new, selected and retired as itself, got {ids}")
    if cookie is None:
        add("retire_cookie_missing")
    if trial["epoch"] is None:
        add("epoch_missing")

    if trial["link"]:
        first, last, between = trial["link"][0], trial["link"][-1], trial["link"][1:-1]
        if any(s["id"] is None for s in between):
            add("repeat_unresolved", "a link frame start between the previous and the next shows no resolved picture id")
        if first["id"] is None or last["id"] is None:
            add("id_unresolved", "the previous or the next link frame start shows no resolved picture id")
        elif not missing:
            end = ids["previous"] if kind == "held" else ids["selected"]
            if first["id"] != ids["previous"] or last["id"] != end or any(s["id"] not in (None, ids["previous"]) for s in between):
                add("id_mismatch", f"the link shows {first['id']} .. {last['id']}, expected {ids['previous']} .. {end}")
    return trial


def _delays(trial):
    """Each time minus the trial's previous link frame start. None where a time is missing."""
    out = dict.fromkeys(STEPS)
    if trial["link"]:
        previous = trial["link"][0]["iv"]
        for step, time in (("ready", trial["ready"]), ("tick", trial["decoder_tick"]), ("ref", trial["ref_edge"]), ("wait", trial["trk2_exit"]),
                           ("link", trial["link"][-1]["iv"])):
            out[step] = None if time is None else time - previous
    return out


def _paired(low, high):
    """The `+13` delay minus the `+11` delay, step by step."""
    return {step: None if low is None or high is None or low[step] is None or high[step] is None else high[step] - low[step] for step in STEPS}


def _shift(interval, target, tol):
    """How a paired displacement reads: it `follows` the eligibility shift, stays `fixed`, is `unresolved` (straddles a band) or is `other`."""
    if interval is None:
        return None
    follows, fixed = interval.relation(target - tol, target + tol), interval.relation(-tol, tol)
    return "follows" if follows == "in" else "fixed" if fixed == "in" else "unresolved" if "straddle" in (follows, fixed) else "other"


def _judge(pid, trials, shared, target, tol):
    problems = list(shared) + [p for t in trials for p in t["problems"]]

    def add(code, where, detail=""):
        problems.append({"code": code, "where": where, "detail": detail, "pair": pid})

    cells = {}
    for t in trials:
        if t["kind"] in KINDS and t["arm"] in ARMS:
            if (t["kind"], t["arm"]) in cells:
                add("trial_duplicate", t["where"])
            cells[(t["kind"], t["arm"])] = t
    for kind in KINDS:
        for arm in ARMS:
            if (kind, arm) not in cells:
                add("trial_missing" if kind == "split" else "control_missing:" + kind, f"{pid}/{kind}/{arm}", "the pair needs all six trials")
    delays = {key: _delays(t) for key, t in cells.items()}
    deltas = {kind: _paired(delays.get((kind, ARMS[0])), delays.get((kind, ARMS[1]))) for kind in KINDS}

    modes = {t["mode"] for t in cells.values()}
    if None in modes:
        add("mode_unknown", pid, "every trial names its mode")
    if len(modes - {None}) > 1:
        add("mode_mismatch", pid, ", ".join(sorted(map(str, modes - {None}))))
    if len({t["epoch"] for t in cells.values()} - {None}) > 1:
        add("epoch_mismatch", pid, "the trials of a pair share one presenter epoch")
    for t in cells.values():
        if t["lock"] != "locked":
            add("lock_lost" if t["lock"] in ("lost", "relocked") else "lock_unknown", t["where"], f"lock {t['lock']!r}")
    own = [t for t in cells.values() if t["kind"] != "held"]
    for what, keys in (("picture", [(t["epoch"], t["ids"]["retained"]) for t in own if t["ids"]["retained"] is not None]),
                       ("retire cookie", [t["cookie"] for t in own if t["cookie"] is not None])):
        if len(keys) != len(set(keys)):
            add("id_not_unique", pid, f"two trials share a {what}; every trial needs its own tagged picture")

    for (kind, arm), d in delays.items():
        if d["ready"] is not None and d["tick"] is not None:
            if not d["ready"].lo > 0:
                add("phase_ambiguous", f"{pid}/{kind}/{arm}", "the ready time is not provably after the previous link frame start")
            elif not d["ready"].hi < d["tick"].lo:
                add("eligibility_after_tick", f"{pid}/{kind}/{arm}", "the picture must be eligible before the next fixed tick")

    for kind in KINDS:       # the same previous link phase and the same next fixed tick in both arms
        if deltas[kind]["tick"] is not None:
            rel = deltas[kind]["tick"].relation(-tol, tol)
            if rel != "in":
                add("phase_mismatch" if rel == "apart" else "phase_ambiguous", f"{pid}/{kind}",
                    "the next fixed tick sits at different delays from the previous link frame start in the two arms")
    for kind in ("split", "unsplit"):
        if deltas[kind]["ready"] is not None:
            rel = deltas[kind]["ready"].relation(target - tol, target + tol)
            if rel != "in":
                add("ready_delta_off_target" if rel == "apart" else "displacement_unresolved:ready", f"{pid}/{kind}", "eligibility must move by the target")
    for arm in ARMS:         # the unsplit control has the candidate's eligibility
        a, b = delays.get(("split", arm)), delays.get(("unsplit", arm))
        if a and b and a["ready"] is not None and b["ready"] is not None:
            rel = (b["ready"] - a["ready"]).relation(-tol, tol)
            if rel != "in":
                add("control_eligibility_differs:unsplit" if rel == "apart" else "control_unresolved:unsplit", f"{pid}/unsplit/{arm}")

    for kind, ref_band in (("unsplit", (-tol, tol)), ("held", (target - tol, target + tol))):
        d = deltas[kind]
        if d["ref"] is not None:      # unsplit: the reference does not move. held: it moves as in the candidate. Neither may move the link.
            rel = d["ref"].relation(*ref_band)
            if rel != "in":
                add(("control_reference_moved:" if kind == "unsplit" else "control_reference_not_changed:") + kind if rel == "apart"
                    else "control_unresolved:" + kind, f"{pid}/{kind}")
        if d["link"] is not None:
            rel = d["link"].relation(-tol, tol)
            if rel != "in":
                add("control_explains_shift:" + kind if rel == "apart" else "control_unresolved:" + kind, f"{pid}/{kind}",
                    "a control must leave the link frame start where it was")

    link = deltas["split"]["link"]
    if link is not None and "straddle" in (link.relation(target - tol, target + tol), link.relation(-tol, tol)):
        add("displacement_unresolved:link", f"{pid}/split", "the paired link displacement cannot be placed inside or outside its band")

    verdict = "inconclusive"
    if not problems:
        ref, wait = (deltas["split"][s] for s in ("ref", "wait"))
        follows, fixed = link.relation(target - tol, target + tol), link.relation(-tol, tol)
        ref_rel, wait_rel = ref.relation(target - tol, target + tol), wait.relation(target - tol, target + tol)
        if follows == "in":
            if ref_rel == wait_rel == "in":
                verdict = "supports"
            elif ref_rel == "apart" and wait_rel == "in":
                add("reference_fixed_output_followed", pid, "the TRK2 exit and the link frame start followed eligibility while the reference edge did not: "
                    "picture tracking that this test cannot credit to the split reference")
            else:
                add("chain_incoherent" if "apart" in (ref_rel, wait_rel) else "displacement_unresolved:chain", pid,
                    "the link frame start followed eligibility but the reference edge or the TRK2 exit did not move with it")
        elif fixed == "in":
            if ref_rel == "in":
                verdict = "rejects-in-configuration"
            else:
                add("reference_did_not_move" if ref_rel == "apart" else "displacement_unresolved:ref", pid,
                    "the link frame start stayed fixed but the reference edge was not shown to move, so the wiring is not established")
        else:
            add("link_displacement_unexplained", pid, "the link displacement is neither the eligibility shift nor zero")

    return {"pair_id": pid, "verdict": verdict, "reasons": sorted({p["code"] for p in problems}),
            "problems": [p for p in problems if p not in shared],
            "delays_ms": {f"{kind}{arm}": {s: None if v is None else v.json() for s, v in d.items()} for (kind, arm), d in sorted(delays.items())},
            "deltas_ms": {kind: {s: None if v is None else v.json() for s, v in d.items()} for kind, d in deltas.items()},
            "shifts": {kind: {s: _shift(v, target, tol) for s, v in d.items()} for kind, d in deltas.items()},
            "repeats": {f"{kind}{arm}": len(t["link"]) - 2 for (kind, arm), t in sorted(cells.items()) if t["link"]}}


def _inventory(inventory, problems):
    """What an old capture does and does not hold, as the same reasons a trial would give."""
    channels = inventory.get("channels", {})
    link = channels.get("link", [])
    if not any(c["src"].partition(":")[0] == LINK for c in link):
        names = ", ".join(c["src"] for c in link)
        problems.add("link_witness_missing", "inventory", "no link-witness times" + (f"; {names} is not a link witness" if names else ""))
        for kind in sorted({c["src"].partition(":")[0] for c in link} & set(NOT_EVIDENCE)):
            problems.add("provenance_not_link_evidence:" + kind, "inventory", NOT_EVIDENCE[kind])
    for channel in CHANNELS:
        if not channels.get(channel):
            problems.add("witness_missing:" + channel, "inventory", "no times recorded")
    for what in inventory.get("unavailable", []):
        problems.add(UNAVAILABLE_CODES.get(what, "witness_missing:" + what), "inventory", f"the capture says it did not record: {what}")


def analyse(doc, *, allow_synthetic=False, min_pairs=MIN_PAIRS, target_ms=TARGET_MS, tolerance_ms=TOLERANCE_MS):
    """A paired-evidence document in, a report out. Raises EvidenceError only for a document that is not one."""
    if not isinstance(doc, dict) or doc.get("format") != FORMAT or doc.get("version") != 1 or not isinstance(doc.get("trials"), list):
        raise EvidenceError(f"not a {FORMAT} version 1 document with a trials list")
    shared = Problems()
    clock = doc.get("clock") if isinstance(doc.get("clock"), dict) else {}
    if clock.get("calibrated") is not True:
        shared.add("clock_uncalibrated", "file", "the times must be on one calibrated clock")
    try:
        align = {kind: _num(ms) for kind, ms in (clock.get("alignment_ms") or {}).items()}
    except (AttributeError, ValueError) as error:
        raise EvidenceError(f"clock.alignment_ms: {error}") from error
    if any(ms < 0 for ms in align.values()):
        raise EvidenceError("clock.alignment_ms must not be negative")
    if not (isinstance(doc.get("witness"), dict) and doc["witness"].get("resolves_repeats") is True):
        shared.add("repeat_unresolved", "file", "the link witness is not declared to resolve repeat scans")
    if not doc["trials"]:
        shared.add("no_trials", "file", "no paired trials were recorded")
    if isinstance(doc.get("inventory"), dict):
        _inventory(doc["inventory"], shared)

    trials = [_read_trial(raw, shared, align, allow_synthetic) for raw in doc["trials"]]
    if len({t["mode"] for t in trials} - {None}) > 1:
        shared.add("mode_mismatch", "file", "every trial runs in one established mode")
    by_pair = {}
    for t in trials:
        by_pair.setdefault(t["pair"], []).append(t)
    pairs = [_judge(pid, group, shared, target_ms, tolerance_ms) for pid, group in by_pair.items()]

    found = {p["verdict"] for p in pairs}
    extra = []
    if not pairs or "inconclusive" in found:
        verdict = "inconclusive"
    elif len(found) > 1:
        verdict, extra = "inconclusive", ["pairs_disagree"]
    elif len(pairs) < min_pairs:
        verdict, extra = "inconclusive", ["too_few_pairs"]
    else:
        verdict = found.pop()
    problems = list(shared) + [p for pair in pairs for p in pair["problems"]]
    return {"verdict": verdict, "reasons": sorted({p["code"] for p in problems} | set(extra)),
            "problems": [dict(p) for i, p in enumerate(problems) if p not in problems[:i]],
            "pairs": pairs, "min_pairs": min_pairs, "target_ms": float(target_ms), "tolerance_ms": float(tolerance_ms),
            "synthetic": shared.synthetic, "notes": ([SYNTHETIC_NOTE] if shared.synthetic else []) + [CANNOT_CLOSE]}


def historical(irq_csv, capture_paths):
    """An evidence document for old captures: no trials (none were run) and an inventory of what each file holds."""
    channels = {name: [] for name in ("link",) + CHANNELS}
    unavailable = []
    with open(irq_csv, encoding="utf-8") as handle:
        lines = handle.read().splitlines()
    number = re.search(r"\birq=(\d+)", lines[0]) if lines and lines[0].startswith("#") else None
    rows = list(csv.DictReader(line for line in lines if not line.startswith("#")))
    if rows:
        channels["link"].append({"src": "irq:irq" + (number.group(1) if number else "?"), "rows": len(rows),
                                 "increments": sum(int(r["increment"]) for r in rows)})
    for path in capture_paths:
        with open(path, encoding="utf-8") as handle:
            capture = json.load(handle)
        if capture.get("format") != CAPTURE_FORMAT:
            raise EvidenceError(f"{path}: not a {CAPTURE_FORMAT} file")
        channels.setdefault("ack", []).append({"src": "ack:" + str(capture.get("producer")), "records": len(capture.get("records", [])),
                                               "fields": [f for f in capture.get("fields", []) if f.endswith("_ns")]})
        unavailable += [u for u in capture.get("header", {}).get("unavailable", []) if u not in unavailable]
    return {"format": FORMAT, "version": 1, "clock": {"calibrated": False}, "trials": [],
            "inventory": {"channels": channels, "unavailable": unavailable}}


def _fmt(interval):
    return "-" if interval is None else f"[{interval[0]:.3f}, {interval[1]:.3f}]"


def render(report):
    lines = [f"Verdict: {report['verdict']}  ({len(report['pairs'])} pair(s), at least {report['min_pairs']} needed)"]
    for note in report["notes"][:-1]:
        lines.append(note)
    for pair in report["pairs"]:
        lines.append(f"Pair {pair['pair_id']}: {pair['verdict']}")
        lines.append(f"  {'step (ms from the previous link start)':40}{'split +11':>20}{'split +13':>20}{'+13 minus +11':>20}{'reading':>12}")
        for step in STEPS:
            row = [(pair["delays_ms"].get(f"split{arm}") or {}).get(step) for arm in ARMS] + [pair["deltas_ms"]["split"][step]]
            lines.append(f"  {step:40}" + "".join(f"{_fmt(v):>20}" for v in row) + f"{pair['shifts']['split'][step] or '-':>12}")
        for kind in ("unsplit", "held"):
            ref, link = pair["deltas_ms"][kind]["ref"], pair["deltas_ms"][kind]["link"]
            lines.append(f"  control {kind}: reference {_fmt(ref)} {pair['shifts'][kind]['ref'] or '-'}, link {_fmt(link)} {pair['shifts'][kind]['link'] or '-'}")
        lines.append("  repeat scans between: " + (", ".join(f"{k} {v}" for k, v in pair["repeats"].items()) or "-"))
    if report["reasons"]:
        lines.append("Reasons:")
        seen = set()
        for p in report["problems"]:
            if p["code"] not in seen:
                seen.add(p["code"])
                lines.append(f"  {p['code']}" + (f": {p['detail']}" if p["detail"] else "") + f" ({p['where']})")
        lines += [f"  {code}" for code in report["reasons"] if code not in seen]
    lines.append(report["notes"][-1])
    return "\n".join(lines)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("files", nargs="+", help="trials.json; with --historical: irq-live.csv followed by capture JSON files")
    parser.add_argument("--historical", action="store_true", help="inventory an old IRQ trace and capture instead of reading trials")
    parser.add_argument("--allow-synthetic", action="store_true", help="accept `synthetic` provenance (fixtures); the report then says it is arithmetic only")
    parser.add_argument("--min-pairs", type=int, default=MIN_PAIRS, help="pairs needed for a verdict; each pair lists its tick delay, so phase coverage can be read off (default %(default)s)")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        if args.historical:
            doc = historical(args.files[0], args.files[1:])
        elif len(args.files) == 1:
            with open(args.files[0], encoding="utf-8") as handle:
                doc = json.load(handle)
        else:
            parser.error("one trials file, or --historical with an IRQ trace")
        report = analyse(doc, allow_synthetic=args.allow_synthetic, min_pairs=args.min_pairs)
    except (EvidenceError, OSError, ValueError, KeyError) as error:
        print(f"paired_evidence: {error}", file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2) if args.json else render(report))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
