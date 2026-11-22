#!/usr/bin/env python3
"""Reads the two files of one VRR/PTS timing capture and says what survived from the GeForce NOW sender to the player's play() call.

    python3 analyse_capture.py <nonce>-streamer.json <nonce>-feeder.json [--json]

The streamer file (the engine, gfn/engine/crates/opennow-streamer-platform/src/vrr_capture.rs) holds each frame's source identity and RTP
timestamp at the sink and the wire key it was handed over with; the feeder file (gfn/feeder/src/gf_capture.h) holds the same wire key where
the player got it, with the exact argument of play(). The two are joined by epoch and the unrounded wire key. Records that find no partner
stay in the report; nothing is dropped from the counts. Both clocks are CLOCK_MONOTONIC of one machine.

What this can and cannot show is stated in the output: a constant frame spacing, repeated frame ids or missing downstream events do not
prove variable refresh. Decoder completion, the frame the decoder selects, release and optical times are not captured.

The streamer file may also carry two sections of the transport's own observations, `sender_reports` (RTCP sender reports per track) and
`frame_prefix` (the sender's prefix of each encoded picture). Each is `null` when not recorded and then reported as "not recorded", never as
zero. They are kept as three separate outcomes: report headers heard; usable audio and video timestamp pairs; sender capture times and
pacing flags per picture. A header, or a report body that could not be read, anchors no clock, and neither a capture time nor a pacing flag
shows that pixels changed, that Cloud G-SYNC was accepted or that a panel scanned.
"""
import collections
import json
import math
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
TRACKS = ("video", "audio")
PREFIX_STATUSES = ("read", "absent", "malformed")
PREFIX_FIELDS = ("frame_index", "rtp_timestamp", "arrival_ns", "status", "byte0", "header_size", "byte2", "byte3", "capture_time_us", "sender_paced",
                 "presentation_time_us", "access_unit_length", "prefix_bytes", "stripped_bytes", "other_tags")
BUCKET_MS = 0.25
SENDER_NOTE = ("A sender-report header, or a report whose body could not be read, anchors no clock: only a read report with a known NTP time on each track "
               "gives a usable pair, and that audio and video share one sender clock is assumed here, not verified.")
PREFIX_NOTE = ("A capture time or the 0x200 sender-paced record alone does not show that the game's pixels changed, that Cloud G-SYNC was accepted (the "
               "finalized answer is in the NVST cloud-gsync log line) or that the panel scanned a picture on its own clock. The field meanings are "
               "OpenNOW's names, not proven.")


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


def _signed(step):
    """A difference of two 32-bit counters as a signed step."""
    step %= WRAP
    return step - WRAP if step > WRAP // 2 else step


def _count(value):
    return "unknown" if value is None else str(value)


def _yes(value):
    return "unknown" if value is None else "yes" if value else "no"


def _pair(sample):
    """One (NTP, RTP) pair of a report: None when the report was not read or its NTP time is unknown."""
    if not isinstance(sample, dict) or sample.get("ntp_us") is None or sample.get("rtp") is None:
        return None
    return {"ntp_us": sample["ntp_us"], "rtp": sample["rtp"], "arrival_ns": sample.get("arrival_ns")}


def _rate_ratio(first, last, clock_hz):
    """RTP time over NTP time between a track's first and last pairs: about 1.0 if the declared clock rate is the sender's."""
    if first is None or last is None or not clock_hz:
        return None
    ntp_s = (last["ntp_us"] - first["ntp_us"]) / 1e6
    return _signed(last["rtp"] - first["rtp"]) / clock_hz / ntp_s if ntp_s > 0 else None


def report_headers(doc):
    """Outcome 1: the RTCP sender-report headers heard, per track. A header names a sender and shows that a report arrived; its times are
    encrypted, so on its own it is not a clock anchor."""
    section = doc.get("sender_reports")
    if not isinstance(section, dict):
        return {"recorded": False}
    tracks = {}
    for name in ("video", "audio", "unmapped"):
        track = section.get(name)
        if not isinstance(track, dict):
            continue
        wire, read = track.get("wire") or {}, track.get("read") or {}
        header_only, read_count = wire.get("count"), read.get("count")
        every = read.get("every_us") if read.get("every_us") is not None else wire.get("every_us")
        tracks[name] = {"heard": header_only + read_count if isinstance(header_only, int) and isinstance(read_count, int) else None,
                        "header_only": header_only, "read": read_count, "ssrc": track.get("ssrc"), "clock_hz": track.get("clock_hz"), "every_us": every}
    return {"recorded": True, "tracks": tracks, "both_heard": (section.get("coverage") or {}).get("both_heard")}


def timestamp_pairs(doc):
    """Outcome 2: usable timestamp pairs, a read report with a known NTP time and its RTP time, on both tracks. Only these can place a track's RTP
    clock on a sender clock; that the two tracks share one is assumed, not checked."""
    section = doc.get("sender_reports")
    if not isinstance(section, dict):
        return {"recorded": False}
    tracks = {}
    for name in TRACKS:
        track = section.get(name) if isinstance(section.get(name), dict) else {}
        read = track.get("read") or {}
        first, last = _pair(read.get("first")), _pair(read.get("last"))
        tracks[name] = {"read": read.get("count"), "usable": read.get("usable"), "clock_hz": track.get("clock_hz"), "via": read.get("via"),
                        "first": first, "last": last, "rate_ratio": _rate_ratio(first, last, track.get("clock_hz"))}
    coverage = section.get("coverage") or {}
    return {"recorded": True, "both_read": coverage.get("both_read"), "both_usable": coverage.get("both_usable"), "tracks": tracks}


def _histogram(values):
    """The 12 fullest 0.25 ms buckets of a list of milliseconds, as [bucket start, count]."""
    buckets = collections.Counter(math.floor(v / BUCKET_MS) for v in values)
    return [[k * BUCKET_MS, n] for k, n in sorted(buckets.items(), key=lambda item: (-item[1], item[0]))[:12]]


def frame_prefix(doc, srows, header):
    """Outcome 3: what the sender wrote at the head of each encoded picture. The meanings are OpenNOW's names and not proven: capture_time_us is read
    as microseconds on a clock of the server's own (only differences mean anything), sender_paced (record 0x200) as a pacing flag."""
    section = doc.get("frame_prefix")
    if not isinstance(section, dict):
        return {"recorded": False}
    where = "streamer file"
    fields, raw = section.get("fields"), section.get("rows")
    _need(isinstance(fields, list) and isinstance(raw, list), where, "frame_prefix has no field list or rows")
    _need(set(PREFIX_FIELDS) <= set(fields), where, "frame_prefix lacks the columns " + ", ".join(sorted(set(PREFIX_FIELDS) - set(fields))))
    for row in raw:
        _need(isinstance(row, list) and len(row) == len(fields), where, "a frame_prefix row does not match its field list")
    table = [dict(zip(fields, row)) for row in raw]
    status = {name: sum(row["status"] == name for row in table) for name in PREFIX_STATUSES}
    _need(sum(status.values()) == len(table), where, "a frame_prefix row has a status other than read, absent or malformed")
    for row in table:
        _need(isinstance(row["frame_index"], int) and isinstance(row["rtp_timestamp"], int), where, "a frame_prefix row lacks its frame index or RTP timestamp")
        _need(row["status"] != "read" or isinstance(row["capture_time_us"], int), where, "a frame_prefix row that was read has no capture time")
    read = [row for row in table if row["status"] == "read"]
    hz, expected_ms = header["rtp_clock_hz"], 1000.0 / header["fps"]
    # Consecutive pictures only: a repeated or skipped frame index is not an interval, and neither is a row without a prefix.
    steps = [(a, b) for a, b in zip(table, table[1:]) if a["status"] == b["status"] == "read" and (b["frame_index"] - a["frame_index"]) % WRAP == 1]
    capture_ms = [_signed(b["capture_time_us"] - a["capture_time_us"]) / 1000.0 for a, b in steps]
    rtp_ms = [_signed(b["rtp_timestamp"] - a["rtp_timestamp"]) * 1000.0 / hz for a, b in steps]
    interval = stats(capture_ms)
    end_to_end = None
    if len(read) >= 2:
        capture_span = _signed(read[-1]["capture_time_us"] - read[0]["capture_time_us"]) / 1000.0
        rtp_span = _signed(read[-1]["rtp_timestamp"] - read[0]["rtp_timestamp"]) * 1000.0 / hz
        end_to_end = {"pictures": len(read), "capture_ms": capture_span, "rtp_ms": rtp_span, "drift_ms": capture_span - rtp_span,
                      "drift_ppm": (capture_span - rtp_span) / rtp_span * 1e6 if rtp_span > 0 else None}
    paced = sum(row["sender_paced"] is not None and row["sender_paced"] != 0 for row in table)
    unpaced = sum(row["sender_paced"] == 0 for row in table)
    known = paced + unpaced
    presentation_ms = [(b["presentation_time_us"] - a["presentation_time_us"]) / 1000.0 for a, b in zip(table, table[1:])
                       if a["presentation_time_us"] is not None and b["presentation_time_us"] is not None and (b["frame_index"] - a["frame_index"]) % WRAP == 1]
    by_tag, other_rows, other_total = collections.Counter(), 0, 0
    for row in table:
        if row["other_tags"]:
            other_rows += 1
            other_total += row["other_tags"].get("total", 0)
            by_tag.update(f"0x{record[0]:03x}" for record in row["other_tags"].get("records", []))
    both_lengths = [row for row in table if row["prefix_bytes"] is not None and row["stripped_bytes"] is not None]
    joinable = [r for r in srows if r["flags"] & F_INDEX_VALID]
    row_keys = {(row["frame_index"], row["rtp_timestamp"]) for row in table}
    record_keys = {(r["frame_index"], r["rtp_timestamp"]) for r in joinable}
    return {
        "recorded": True, "counts": section.get("counts"), "rows": len(table), "status": status,
        "present": {name: sum(row[name] is not None for row in table)
                    for name in ("capture_time_us", "sender_paced", "presentation_time_us", "access_unit_length")},
        "capture_interval": {
            "pairs": len(steps), "skipped": max(len(table) - 1, 0) - len(steps), "ms": interval, "constant": constant_rate(capture_ms),
            "not_forward": sum(step <= 0 for step in capture_ms), "buckets_ms": _histogram(capture_ms), "expected_ms": expected_ms,
            "median_vs_expected": interval["median"] / expected_ms if capture_ms else None,
        },
        "sender_paced": {
            "paced": paced, "unpaced": unpaced, "unknown": len(table) - known, "unknown_without_prefix": sum(row["status"] != "read" for row in table),
            "raw": dict(collections.Counter("unknown" if row["sender_paced"] is None else str(row["sender_paced"]) for row in table)),
            "paced_share_of_known": paced / known if known else None,
        },
        "capture_vs_rtp": {"pairs": len(steps), "error_ms": stats([c - r for c, r in zip(capture_ms, rtp_ms)]), "end_to_end": end_to_end},
        "presentation_interval": {"pairs": len(presentation_ms), "ms": stats(presentation_ms)},
        "other_tags": {"rows": other_rows, "total": other_total, "kept_by_tag": dict(sorted(by_tag.items()))},
        "shape": {
            "byte0": dict(sorted(collections.Counter(f"0x{row['byte0']:02x}" for row in table if row["byte0"] is not None).items())),
            "header_size": dict(sorted(collections.Counter(str(row["header_size"]) for row in table if row["header_size"] is not None).items())),
            "byte2": dict(sorted(collections.Counter(f"0x{row['byte2']:02x}" for row in table if row["byte2"] is not None).items())),
            "byte3": dict(sorted(collections.Counter(f"0x{row['byte3']:02x}" for row in table if row["byte3"] is not None).items())),
        },
        "lengths": {"compared": len(both_lengths), "differ": sum(row["prefix_bytes"] != row["stripped_bytes"] for row in both_lengths),
                    "stripped_unknown": sum(row["stripped_bytes"] is None for row in table)},
        "coverage": {
            "streamer_records": len(srows), "without_index": len(srows) - len(joinable),
            "with_prefix_row": sum((r["frame_index"], r["rtp_timestamp"]) in row_keys for r in joinable),
            "without_prefix_row": sum((r["frame_index"], r["rtp_timestamp"]) not in row_keys for r in joinable),
            "rows_without_record": sum((row["frame_index"], row["rtp_timestamp"]) not in record_keys for row in table),
        },
    }


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
    headers, anchors, prefix = report_headers(streamer), timestamp_pairs(streamer), frame_prefix(streamer, srows, sh)
    if headers["recorded"]:
        notes.append(SENDER_NOTE)
    if prefix["recorded"]:
        notes.append(PREFIX_NOTE)
        if not prefix["present"]["sender_paced"]:
            notes.append("No picture carried the 0x200 sender-paced record: the flag is unknown for every picture, not off.")
        if (prefix["counts"] or {}).get("omitted"):
            notes.append(f"The prefix log omitted {prefix['counts']['omitted']} pictures (ring full): its figures cover the stored rows only.")
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
        "report_headers": headers, "timestamp_pairs": anchors, "frame_prefix": prefix,
        "incomplete": incomplete, "unavailable": UNAVAILABLE, "notes": notes,
    }


def _fmt(s):
    if not s or not s.get("n"):
        return "none"
    return f"n={s['n']} min {s['min']:.3f} median {s['median']:.3f} p95 {s['p95']:.3f} max {s['max']:.3f} distinct {s['distinct']}"


def _pairs_line(name, track):
    def show(pair):
        return "none" if pair is None else f"ntp_us={pair['ntp_us']} rtp={pair['rtp']}"
    ratio = track["rate_ratio"]
    return (f"  {name}: {_count(track['usable'])} usable of {_count(track['read'])} read, clock {_count(track['clock_hz'])} Hz; first {show(track['first'])}; "
            f"last {show(track['last'])}; RTP over NTP {'unknown' if ratio is None else f'{ratio:.4f}'}")


def _render_sender(report):
    """The three sender outcomes, one block each: what is absent reads 'not recorded' or 'unknown', never zero."""
    headers, anchors, prefix = report["report_headers"], report["timestamp_pairs"], report["frame_prefix"]
    lines = []
    if not headers["recorded"]:
        lines.append("Sender-report headers heard: not recorded in this capture")
    else:
        lines.append(f"Sender-report headers heard (a header alone anchors nothing): audio and video both heard: {_yes(headers['both_heard'])}")
        for name, t in headers["tracks"].items():
            lines.append(f"  {name}: {_count(t['heard'])} heard ({_count(t['header_only'])} header only, {_count(t['read'])} read), ssrc {_count(t['ssrc'])}, "
                         f"every {_count(t['every_us'])} us")
    if not anchors["recorded"]:
        lines.append("Usable audio and video timestamp pairs: not recorded in this capture")
    else:
        lines.append(f"Usable audio and video timestamp pairs: both tracks read: {_yes(anchors['both_read'])}, both usable: {_yes(anchors['both_usable'])}")
        lines += [_pairs_line(name, t) for name, t in anchors["tracks"].items()]
    if not prefix["recorded"]:
        lines.append("Per-picture sender prefix: not recorded in this capture")
        return lines
    st, ci, pc, cv = prefix["status"], prefix["capture_interval"], prefix["sender_paced"], prefix["capture_vs_rtp"]
    ratio = "unknown" if ci["median_vs_expected"] is None else f"{ci['median_vs_expected']:.3f}x"
    share = "unknown" if pc["paced_share_of_known"] is None else f"{pc['paced_share_of_known'] * 100:.1f}%"
    e2e = cv["end_to_end"]
    drift = "not enough pictures" if e2e is None else (f"{e2e['drift_ms']:.3f} ms over {e2e['rtp_ms']:.1f} ms of RTP time ("
                                                       + ("unknown" if e2e["drift_ppm"] is None else f"{e2e['drift_ppm']:.1f}") + " ppm)")
    lines += [
        f"Per-picture sender prefix: {prefix['rows']} rows ({st['read']} read, {st['absent']} absent, {st['malformed']} malformed); present: "
        + ", ".join(f"{name} {n}" for name, n in prefix["present"].items()),
        f"  Capture interval (ms), {ci['pairs']} consecutive pairs ({ci['skipped']} skipped): {_fmt(ci['ms'])}  constant={ci['constant']}, "
        f"{ci['not_forward']} not forward, median {ratio} the {ci['expected_ms']:.3f} ms of the header fps",
        "  Capture interval buckets (ms: count): " + ("none" if not ci["buckets_ms"] else ", ".join(f"{start:.2f}: {n}" for start, n in ci["buckets_ms"])),
        f"  Sender-paced flag (0x200): {pc['paced']} paced, {pc['unpaced']} unpaced, {pc['unknown']} unknown ({pc['unknown_without_prefix']} without a prefix), "
        f"paced share of the known {share}, raw {pc['raw']}",
        f"  Capture time vs RTP time, per interval (ms, {cv['pairs']} pairs): {_fmt(cv['error_ms'])}",
        f"  Capture time vs RTP time, end to end: {drift}",
    ]
    pi = prefix["presentation_interval"]
    lines.append(f"  Presentation time (0x201) interval (ms): {_fmt(pi['ms'])}")
    ot, ln, co = prefix["other_tags"], prefix["lengths"], prefix["coverage"]
    lines += [
        f"  Other extension tags: {ot['rows']} rows, {ot['total']} records, kept by tag {ot['kept_by_tag']}",
        f"  Prefix shape: {prefix['shape']}",
        f"  Declared prefix length vs what the receiver strips: {ln['differ']} of {ln['compared']} differ, {ln['stripped_unknown']} found no codec data",
        f"  Rows against the capture's records: {co['with_prefix_row']} of {co['streamer_records'] - co['without_index']} records have a row, "
        f"{co['without_prefix_row']} do not, {co['rows_without_record']} rows have no record",
    ]
    return lines


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
    lines += _render_sender(report)
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
