#!/usr/bin/env python3
"""Reads the two files of one VRR/PTS timing capture and says what survived from the GeForce NOW sender to the player's play() call.

    python3 analyse_capture.py <nonce>-streamer.json <nonce>-feeder.json [--json]

The streamer file (the engine, gfn/engine/crates/opennow-streamer-platform/src/vrr_capture.rs) holds each frame's source identity and RTP
timestamp at the sink and the wire key it was handed over with; the feeder file (gfn/feeder/src/gf_capture.h) holds the same wire key where
the player got it, with the exact argument of play(). The two are joined by epoch and the unrounded wire key. Records that find no partner
stay in the report; nothing is dropped from the counts. Both clocks are CLOCK_MONOTONIC of one machine.

What this can and cannot show is stated in the output: a constant frame spacing, repeated frame ids or missing downstream events do not
prove variable refresh. Decoder completion, the frame the decoder selects, release and optical times are not captured.
"""
import json
import statistics
import sys

FORMAT = "lumagame-vrr-pts-capture"
COUNTS = ("omitted", "evicted", "unmatched", "resets")
PACINGS = ("Smooth", "Lowest latency")
UNAVAILABLE = ["decoder completion", "selected-frame id", "release", "optical times"]
CANNOT_PROVE = ("A constant frame spacing, repeated frame ids or missing downstream events cannot prove variable refresh (VRR); "
                "this capture only shows whether each frame's source identity and timestamp reach the player's play() call.")
F_CONTIGUOUS = 2
F_INDEX_VALID = 1
WRAP = 1 << 32


class CaptureError(Exception):
    """A capture file that lacks what the analysis needs; the message names it."""


def load(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def _need(cond, where, what):
    if not cond:
        raise CaptureError(f"{where}: {what}")


def validate(doc, producer):
    """The header is part of the file, not of a log: every field the analysis reports must be in it."""
    where = f"{producer} file"
    _need(isinstance(doc, dict) and doc.get("format") == FORMAT, where, f"not a {FORMAT} file")
    _need(doc.get("producer") == producer, where, f"producer is {doc.get('producer')!r}, expected {producer!r}")
    header = doc.get("header")
    _need(isinstance(header, dict), where, "no header")
    _need(isinstance(header.get("build_id"), str) and header["build_id"], where, "header lacks the LumaGame build id")
    _need(isinstance(header.get("codec"), str) and header["codec"], where, "header lacks the negotiated codec")
    for key in ("width", "height", "fps"):
        _need(isinstance(header.get(key), int) and header[key] > 0, where, f"header lacks the negotiated {key}")
    counts = header.get("counts")
    _need(isinstance(counts, dict), where, "header lacks the counts")
    for key in COUNTS:
        _need(isinstance(counts.get(key), int) and counts[key] >= 0, where, f"header counts lack {key}")
    if producer == "streamer":
        _need(isinstance(header.get("rtp_clock_hz"), int) and header["rtp_clock_hz"] > 0, where, "header lacks the RTP clock rate (Hz)")
        _need(header.get("frame_pacing") in PACINGS, where, "header lacks the frame pacing setting (Smooth or Lowest latency)")
    else:
        _need(isinstance(header.get("epochs"), list) and header["epochs"], where, "header lacks the load epochs")
    fields = doc.get("fields")
    _need(isinstance(fields, list) and isinstance(doc.get("records"), list), where, "no field list or records")
    for row in doc["records"]:
        _need(isinstance(row, list) and len(row) == len(fields), where, "a record does not match the field list")
    return header


def rows(doc):
    names = doc["fields"]
    return [dict(zip(names, row)) for row in doc["records"]]


def stats(values):
    """min, median, p95, max and the number of distinct values (to 1 microsecond) of a list of milliseconds."""
    if not values:
        return {"n": 0}
    ordered = sorted(values)
    return {
        "n": len(ordered), "min": ordered[0], "median": statistics.median(ordered),
        "p95": ordered[min(len(ordered) - 1, int(0.95 * len(ordered)))], "max": ordered[-1],
        "distinct": len({round(v, 3) for v in ordered}),
    }


def constant_rate(values):
    """True if every spacing is within 2% of the median: a steady cadence says nothing about variable refresh."""
    if len(values) < 2:
        return None
    median = statistics.median(values)
    return median > 0 and max(values) - min(values) <= 0.02 * median


def map_epochs(streamer, feeder):
    """The streamer's epoch is its session ordinal and the feeder's its load ordinal; they are matched by the monotonic clock: the feeder
    epoch loaded last before the streamer's first frame, and every later one of the capture (a reload inside one session)."""
    start = streamer["header"]["start_ns"]
    loads = sorted((e["load_ns"], e["epoch"]) for e in feeder["header"]["epochs"])
    before = [load for load in loads if load[0] <= start]
    first = before[-1][0] if before else (loads[0][0] if loads else 0)
    return {epoch for load_ns, epoch in loads if load_ns >= first}


def join(streamer_rows, feeder_rows, feeder_epochs):
    by_key = {}
    for row in feeder_rows:
        by_key.setdefault((row["epoch"], row["wire_key_ns"]), []).append(row)
    pairs, streamer_only, used = [], [], set()
    for row in streamer_rows:
        key = row["wire_key_ns"]
        hit = None
        if key is not None:
            for epoch in feeder_epochs:
                candidates = [r for r in by_key.get((epoch, key), []) if id(r) not in used]
                if candidates:
                    hit = candidates[0]
                    break
        if hit is None:
            streamer_only.append(row)
        else:
            used.add(id(hit))
            pairs.append((row, hit))
    feeder_only = [r for r in feeder_rows if id(r) not in used]
    return pairs, streamer_only, feeder_only


def spacing(pairs, clock_hz):
    """Sender (RTP), handover (wire key) and player (play() argument) spacing, in ms, between neighbouring frames of one continuous run."""
    sender, handover, player, delta_ok = [], [], [], []
    repeated = missing = gaps = no_index = 0
    previous = None
    for streamer, feeder in pairs:
        run = previous is not None and streamer["seq"] == previous[0]["seq"] + 1 and streamer["flags"] & F_CONTIGUOUS and streamer["epoch"] == previous[0]["epoch"]
        if streamer["flags"] & F_INDEX_VALID == 0:
            no_index += 1
        if run:
            before = previous[0]
            step = (streamer["rtp_timestamp"] - before["rtp_timestamp"]) % WRAP
            step = step - WRAP if step > WRAP // 2 else step
            hz = streamer["rtp_clock_hz"] or clock_hz
            sender_ms = step * 1000.0 / hz
            handover_ms = (streamer["wire_key_ns"] - before["wire_key_ns"]) / 1e6
            sender.append(sender_ms)
            handover.append(handover_ms)
            if feeder["player_ms"] is not None and previous[1]["player_ms"] is not None:
                player_ms = feeder["player_ms"] - previous[1]["player_ms"]
                player.append(player_ms)
                delta_ok.append(abs(player_ms - sender_ms) <= 1.0)
            if streamer["flags"] & F_INDEX_VALID and before["flags"] & F_INDEX_VALID:
                gap = (streamer["frame_index"] - before["frame_index"]) % WRAP
                if gap == 0:
                    repeated += 1
                elif gap > 1:
                    gaps += 1
                    missing += gap - 1
        previous = (streamer, feeder)
    return {
        "sender_ms": stats(sender), "handover_ms": stats(handover), "player_ms": stats(player),
        "sender_constant": constant_rate(sender), "handover_constant": constant_rate(handover), "player_constant": constant_rate(player),
        "player_follows_sender_within_1ms": (sum(delta_ok) / len(delta_ok)) if delta_ok else None, "compared": len(delta_ok),
        "frame_index": {"repeated": repeated, "gaps": gaps, "missing": missing, "without_index": no_index},
    }


def latencies(pairs):
    """Where the time between the sink and the player's call goes, from the shared monotonic clock (ms)."""
    sink_to_handover, transport, to_play, in_play = [], [], [], []
    for streamer, feeder in pairs:
        sink_to_handover.append((streamer["handover_done_ns"] - streamer["sink_entry_ns"]) / 1e6)
        transport.append((feeder["recv_ns"] - streamer["handover_done_ns"]) / 1e6)
        if feeder["first_call_ns"] is not None:
            to_play.append((feeder["first_call_ns"] - feeder["recv_ns"]) / 1e6)
            in_play.append((feeder["last_return_ns"] - feeder["first_call_ns"]) / 1e6)
    return {"sink_to_handover_ms": stats(sink_to_handover), "handover_to_feeder_ms": stats(transport),
            "feeder_to_play_ms": stats(to_play), "play_duration_ms": stats(in_play)}


def player_outcomes(feeder_rows):
    out = {"records": len(feeder_rows), "accepted": 0, "dropped": 0, "never_reached_play": 0, "retried": 0, "play_rc_nonzero": 0, "player_ms_went_back": 0}
    last = None
    for row in feeder_rows:
        out["accepted" if row["reason"] == 0 else "dropped"] += 1
        out["never_reached_play"] += row["attempts"] == 0
        out["retried"] += row["attempts"] > 1
        out["play_rc_nonzero"] += bool(row["play_rc"])
        if row["player_ms"] is not None:
            out["player_ms_went_back"] += last is not None and row["player_ms"] < last
            last = row["player_ms"]
    return out


def analyse(streamer, feeder):
    sh, fh = validate(streamer, "streamer"), validate(feeder, "feeder")
    for key in ("build_id", "codec", "width", "height", "fps"):
        _need(sh[key] == fh[key], "the two files", f"disagree on {key}: streamer {sh[key]!r}, feeder {fh[key]!r}")
    srows, frows = rows(streamer), rows(feeder)
    epochs = map_epochs(streamer, feeder)
    pairs, streamer_only, feeder_only = join(srows, frows, epochs)
    pairs.sort(key=lambda p: (p[0]["epoch"], p[0]["seq"]))
    handed = [r for r in srows if r["wire_key_ns"] is not None]
    spacings = spacing(pairs, sh["rtp_clock_hz"])
    incomplete = [f"streamer: {why}" for why in streamer.get("incomplete", [])] + [f"feeder: {why}" for why in feeder.get("incomplete", [])]
    notes = [CANNOT_PROVE]
    if spacings["sender_constant"] and spacings["player_constant"]:
        notes.append("The sender spacing and the player spacing are both constant: the stream shows no variable cadence to carry through.")
    if spacings["frame_index"]["repeated"]:
        notes.append("Frame ids repeat: the id is not a per-frame identity on this stream.")
    if incomplete:
        notes.append("The capture is incomplete (" + "; ".join(incomplete) + "): the figures cover the stored records only.")
    return {
        "header": {key: sh[key] for key in ("build_id", "codec", "width", "height", "fps", "rtp_clock_hz", "frame_pacing")},
        "producers": {
            "streamer": {"counts": sh["counts"], "definitions": sh.get("definitions", {}), "complete": streamer.get("complete")},
            "feeder": {"counts": fh["counts"], "definitions": fh.get("definitions", {}), "complete": feeder.get("complete")},
        },
        "epochs": {"streamer": sh.get("epoch"), "feeder": sorted(epochs)},
        "join": {
            "key": "epoch + unrounded wire key (ns)", "streamer_records": len(srows), "feeder_records": len(frows), "matched": len(pairs),
            "streamer_unmatched": len(streamer_only), "streamer_not_handed": len(srows) - len(handed), "feeder_unmatched": len(feeder_only),
            "streamer_unmatched_seq": [r["seq"] for r in streamer_only[:20]], "feeder_unmatched_seq": [r["seq"] for r in feeder_only[:20]],
            "handed_that_reached_the_feeder": (len(pairs) / len(handed)) if handed else None,
        },
        "spacing": spacings, "latency": latencies(pairs), "player": player_outcomes(frows),
        "incomplete": incomplete, "unavailable": UNAVAILABLE, "notes": notes,
    }


def _fmt(s):
    if not s or not s.get("n"):
        return "none"
    return f"n={s['n']} min {s['min']:.3f} median {s['median']:.3f} p95 {s['p95']:.3f} max {s['max']:.3f} distinct {s['distinct']}"


def render(report):
    h, j, sp = report["header"], report["join"], report["spacing"]
    lines = [
        f"LumaGame build {h['build_id']}: {h['codec']} {h['width']}x{h['height']} at {h['fps']} fps, RTP clock {h['rtp_clock_hz']} Hz, frame pacing {h['frame_pacing']}",
        "Per producer (omitted / evicted / unmatched / resets):",
    ]
    for name, info in report["producers"].items():
        c = info["counts"]
        lines.append(f"  {name}: {c['omitted']} / {c['evicted']} / {c['unmatched']} / {c['resets']}  ({'complete' if info['complete'] else 'incomplete'})")
    lines += [
        f"Join by {j['key']}: {j['matched']} matched, {j['streamer_unmatched']} streamer-only ({j['streamer_not_handed']} never handed over), "
        f"{j['feeder_unmatched']} feeder-only",
        f"Spacing (ms), sender RTP:    {_fmt(sp['sender_ms'])}  constant={sp['sender_constant']}",
        f"Spacing (ms), handover key:  {_fmt(sp['handover_ms'])}  constant={sp['handover_constant']}",
        f"Spacing (ms), player play(): {_fmt(sp['player_ms'])}  constant={sp['player_constant']}",
    ]
    follow = sp["player_follows_sender_within_1ms"]
    lines.append("Player spacing within 1 ms of the sender's: " + ("no pairs to compare" if follow is None else f"{follow * 100:.1f}% of {sp['compared']}"))
    fi = sp["frame_index"]
    lines.append(f"Frame ids: {fi['repeated']} repeated, {fi['gaps']} gaps ({fi['missing']} missing), {fi['without_index']} frames without one")
    for name, s in report["latency"].items():
        lines.append(f"{name}: {_fmt(s)}")
    p = report["player"]
    lines.append(f"Player: {p['accepted']} accepted, {p['dropped']} dropped, {p['never_reached_play']} never reached play(), {p['retried']} retried, "
                 f"{p['play_rc_nonzero']} play() returns not 0, {p['player_ms_went_back']} timestamps went back")
    lines.append("Not captured: " + ", ".join(report["unavailable"]))
    lines += report["notes"]
    return "\n".join(lines)


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    if len(args) != 2 or any(a.startswith("--") and a != "--json" for a in argv):
        print(__doc__, file=sys.stderr)
        return 2
    try:
        report = analyse(load(args[0]), load(args[1]))
    except (CaptureError, OSError, ValueError) as error:
        print(f"analyse_capture: {error}", file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2) if "--json" in argv else render(report))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
