#!/usr/bin/env python3
"""Replays one VRR/PTS capture through the feeder's sender-time mapping and says how closely the player's stamps would follow the sender.

    python3 replay_sendertime.py <nonce>-streamer.json <nonce>-feeder.json [--mapper PATH] [--json]

Each frame that analyse_capture matches supplies its RTP timestamp and clock (streamer file), the load-relative millisecond the player was stamped
with (feeder file: the local time the mapping anchors to) and whether a gap was reported before it. The mapping is the feeder's own C code, run as
`vclock_test --replay` (sh gfn/feeder/build.sh builds it), so the figures are the feeder's and not a second implementation's. A reload (a new
feeder epoch) starts the mapping again, as a load does in the feeder.

The figures say what the mapping would have stamped. They do not show A/V alignment, what the player does with the stamps, or variable refresh.
"""
import argparse
import itertools
import json
import os
import subprocess
import sys

import analyse_capture as ac

MAPPER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "gfn", "feeder", "build", "host", "vclock_test")
WHY = {1: "first picture", 2: "timestamp went back", 3: "clock rate changed", 4: "offset bound"}


def run_mapper(path):
    """The feeder's mapping as a function: (rtp, clock Hz, local ms, gap) lines in, (stamp ms, epochs so far, why an epoch opened or 0) lines out."""
    def run(lines):
        text = "".join(f"{rtp} {hz} {local} {gap}\n" for rtp, hz, local, gap in lines)
        try:
            done = subprocess.run([path, "--replay"], input=text, capture_output=True, text=True, check=False)
        except FileNotFoundError:
            raise ac.CaptureError(f"{path} is missing: build it with sh gfn/feeder/build.sh") from None
        if done.returncode != 0:
            raise ac.CaptureError(f"{path} --replay exited {done.returncode}: {done.stderr.strip()[:200]}")
        answers = [tuple(int(word) for word in row.split()) for row in done.stdout.splitlines()]
        if len(answers) != len(lines):
            raise ac.CaptureError(f"{path} --replay answered {len(answers)} lines for {len(lines)}")
        return answers
    return run


def _tally(errors):
    return {"within_1ms": sum(1 for e in errors if e <= 1.0), "intervals": len(errors), "error_ms": ac.stats(errors)}


def replay(streamer, feeder, mapper):
    sender_header = ac.validate(streamer, "streamer")
    ac.validate(feeder, "feeder")
    pairs, _, _ = ac.join(ac.rows(streamer), ac.rows(feeder), ac.map_epochs(streamer, feeder))
    pairs.sort(key=lambda p: (p[0]["epoch"], p[0]["seq"]))
    kept = [p for p in pairs if p[1]["player_ms"] is not None]
    default_hz = sender_header["rtp_clock_hz"]
    answers = []
    for _, group in itertools.groupby(kept, key=lambda p: p[1]["epoch"]):
        answers += mapper([(s["rtp_timestamp"], s["rtp_clock_hz"] or default_hz, f["player_ms"], 0 if s["flags"] & ac.F_CONTIGUOUS else 1)
                           for s, f in group])
    today, mapped, offsets, opened = [], [], [], []
    went_back = equal = 0
    previous = None
    for index, ((s, f), (stamp, _, why)) in enumerate(zip(kept, answers)):
        offsets.append(stamp - f["player_ms"])
        if why:
            opened.append({"picture": index, "why": WHY[why]})
        if previous is not None and previous[1]["epoch"] == f["epoch"]:
            before_s, before_f, before_stamp = previous
            went_back += stamp < before_stamp
            equal += stamp == before_stamp
            if s["seq"] == before_s["seq"] + 1 and s["flags"] & ac.F_CONTIGUOUS and s["epoch"] == before_s["epoch"]:
                step = (s["rtp_timestamp"] - before_s["rtp_timestamp"]) % ac.WRAP
                step = step - ac.WRAP if step > ac.WRAP // 2 else step
                sender_ms = step * 1000.0 / (s["rtp_clock_hz"] or default_hz)
                today.append(abs(f["player_ms"] - before_f["player_ms"] - sender_ms))
                mapped.append(abs(stamp - before_stamp - sender_ms))
        previous = (s, f, stamp)
    return {
        "pictures": len(kept), "without_player_stamp": len(pairs) - len(kept), "intervals": len(mapped),
        "today": _tally(today), "mapped": dict(_tally(mapped), went_back=went_back, equal_to_the_one_before=equal),
        "epochs": {"count": len(opened), "opened": opened[:10]},
        "offset_to_today_ms": {"min": min(offsets), "max": max(offsets), "mean": sum(offsets) / len(offsets)} if offsets else None,
    }


def render(report):
    today, mapped, epochs, offset = report["today"], report["mapped"], report["epochs"], report["offset_to_today_ms"]

    def within(t):
        return f"{t['within_1ms']}/{t['intervals']}" + (f" ({100.0 * t['within_1ms'] / t['intervals']:.1f}%)" if t["intervals"] else "")

    lines = [
        f"Sender-time replay: {report['pictures']} pictures, {report['intervals']} intervals in continuous runs, "
        f"{report['without_player_stamp']} without a player stamp left out",
        f"Intervals within 1 ms of the sender's: today {within(today)}; sender-time mapping {within(mapped)}",
        f"Error against the sender's interval (ms), today:   {ac._fmt(today['error_ms'])}",
        f"Error against the sender's interval (ms), mapping: {ac._fmt(mapped['error_ms'])}",
        f"Mapped stamps: {mapped['went_back']} went back, {mapped['equal_to_the_one_before']} equal to the one before",
        f"Epochs opened: {epochs['count']}" + "".join(f"; picture {e['picture']}: {e['why']}" for e in epochs["opened"]),
    ]
    if offset:
        lines.append(f"Mapped stamp minus today's (ms): min {offset['min']} max {offset['max']} mean {offset['mean']:.2f}")
    lines.append("Not shown: A/V alignment, what the player does with the stamps, variable refresh.")
    return "\n".join(lines)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("streamer")
    parser.add_argument("feeder")
    parser.add_argument("--mapper", default=MAPPER, help="the feeder's vclock_test binary (default: its host build)")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = replay(ac.load(args.streamer), ac.load(args.feeder), run_mapper(args.mapper))
    except (ac.CaptureError, OSError, ValueError) as error:
        print(f"replay_sendertime: {error}", file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2) if args.json else render(report))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
