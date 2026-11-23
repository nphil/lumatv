#!/usr/bin/env python3
"""Keeps the test tooling from cutting LG's panel refresh short.

When the C4 goes to standby with the panel's counter past `offrsLastTime + offrsInterval` (panel hours; 4 h), LG runs a
compensation of about 10 minutes with the screen dark; a wake in that time aborts it and LG retries at the next standby. Test rounds
that put the set to standby and wake it a minute later therefore never let one finish. `wait_out_refresh` is called by every tool
that wakes the set (tv_gate.wake, which tv_gate.py, tv_next_slot.py and tv_ui_measure.py use): when the set is in standby and a
run is due or likely running, it waits until 15 minutes of standby have passed, saying why; `--force-wake` skips it.

The rule is the helper's (helper/app/src/refreshguard.rs): a run may be in progress when the set has been in standby for under
15 minutes, a run was due when the standby began and the run count has not gone up since. Where the standby began comes from, in
order: the caller (the gate's own standby step knows), the helper's `panel` topic on the set (`standby.since`, the set's clock),
else "now" (the whole 15 minutes are waited). What LG says is read with luna-send over SSH; if that fails the wake goes ahead and
says so (the tool must not be blocked by a reader that is broken).

The direct signal is eplmanager's own: the first field of /mnt/lg/cmn_data/pnwash/state is 2 while a run is in progress. While it says 2
the wake waits, whatever the standby time says, and gives up after 15 minutes from when it was first seen; 8 after a 2 ends the wait.

    refresh_guard.py [--force-wake] status    what LG says, the standby, and whether a wake would wait
    refresh_guard.py [--force-wake] wait      wait out a run now, the way a wake would
    refresh_guard.py check                    exit 1 with the reason while a run is in progress (what installs ask first)

Pure parts (parse_info, verdict, read_state) are tested in tests/test_refresh_guard.py.
"""
import argparse
import json
import math
import os
import re
import sys
import time

SAFE_STANDBY_S = 15 * 60
TICKS_PER_HOUR = 6.0
REFRESHER = "luna://com.webos.service.oledepl/getPixelRefresherInfoList"
USAGE = "luna://com.webos.service.panelcontroller/getPanelUsageTime"
# getPanelUsageTime refuses '{}' (errorCode 3, "Failed to validate against schema", TV round 16); subscribe:false asks for the one answer.
USAGE_PAYLOAD = '{"subscribe":false}'
HELPER_PANEL = "http://127.0.0.1:18731/v1/state/panel"
# eplmanager's own word on a run: the first field of this file is 2 while it runs, 8 when it has finished.
PNWASH_STATE = "/mnt/lg/cmn_data/pnwash/state"
PNWASH_RUNNING, PNWASH_DONE = 2, 8
HELPER_REFRESHER = "http://127.0.0.1:18731/v1/panel/refresher"
# How often the count is read while waiting.
POLL_S = 60
# Standby states in which the screen is lit: nothing to protect.
LIT = ("active", "screen saver")


def decode(text):
    """The first JSON object in what luna-send printed (a pty adds carriage returns and may print noise first)."""
    text = text.replace("\r", "")
    i = text.find("{")
    if i < 0:
        raise ValueError("no JSON reply")
    return json.JSONDecoder().raw_decode(text[i:])[0]


def number(v):
    try:
        n = float(v)
    except (TypeError, ValueError):
        return None
    return n if n == n and n not in (float("inf"), float("-inf")) and n >= 0 else None


def find_offrs(v):
    """The first object, anywhere in a reply, with an `offrs` key: the values may be at the top or in a list of entries."""
    if isinstance(v, dict):
        if any(k.startswith("offrs") for k in v):
            return v
        v = list(v.values())
    if isinstance(v, list):
        for x in v:
            o = find_offrs(x)
            if o is not None:
                return o
    return None


def parse_info(refresher_text, usage_text):
    """{count, last, interval, usage} (hours; usage from the ten-minute ticks) from the two replies. ValueError says which is wrong."""
    r = decode(refresher_text)
    if r.get("returnValue") is False:
        raise ValueError("getPixelRefresherInfoList: %s" % r.get("errorText", "returnValue false"))
    o = find_offrs(r)
    if o is None:
        raise ValueError("getPixelRefresherInfoList: no offrs values in the reply")
    vals = {}
    for key, name in (("offrsCount", "count"), ("offrsLastTime", "last"), ("offrsInterval", "interval")):
        n = number(o.get(key))
        if n is None:
            raise ValueError("getPixelRefresherInfoList: no %s" % key)
        vals[name] = n
    u = decode(usage_text)
    if u.get("returnValue") is False:
        raise ValueError("getPanelUsageTime: %s" % u.get("errorText", "returnValue false"))
    ticks = number(u.get("panelUsageTime"))
    if ticks is None:
        raise ValueError("getPanelUsageTime: no panelUsageTime")
    vals["count"] = int(vals["count"])
    vals["usage"] = ticks / TICKS_PER_HOUR
    return vals


def due(info):
    return info["usage"] >= info["last"] + info["interval"]


def verdict(info, elapsed_s, due_at_start=None, count_at_start=None):
    """Seconds left of the window in which a run may be in progress, 0 when none may be: standby for under 15 minutes, a run due when it
    began (`info` stands in where that is not known) and no run finished since (the count)."""
    if elapsed_s >= SAFE_STANDBY_S:
        return 0
    d = due(info) if due_at_start is None else due_at_start
    if not d:
        return 0
    if count_at_start is not None and info["count"] > count_at_start:
        return 0
    return math.ceil(SAFE_STANDBY_S - elapsed_s)


def read_info(run):
    """Reads LG's numbers over `run(command, timeout)` (the set's shell). ValueError when they cannot be had."""
    try:
        a = run("luna-send -n 1 -f %s '{}'" % REFRESHER, 20)
        b = run("luna-send -n 1 -f %s '%s'" % (USAGE, USAGE_PAYLOAD), 20)
    except Exception as e:  # a dropped connection is a reading that failed, not a reason to stop the tool
        raise ValueError("no answer from the set: %s" % e)
    return parse_info(a, b)


def helper_standby(run):
    """The helper's `panel` topic: {elapsed, due_at_start, count_at_start} of the standby it is tracking, on the set's clock; None when the
    helper is not running, has no such topic, or says the screen is on."""
    try:
        out = run("curl -fsS -m 5 %s; echo; date +%%s" % HELPER_PANEL, 20)
        body, _, tail = out.replace("\r", "").strip().rpartition("\n")
        s = decode(body)["value"]["standby"]
        tv_now = int(tail.strip())
    except Exception:
        return None
    if not s or number(s.get("since")) is None:
        return None
    return {"elapsed": tv_now - number(s["since"]) / 1000.0, "due_at_start": s.get("dueAtStart"), "count_at_start": s.get("countAtStart")}


def read_state(run):
    """eplmanager's pnwash state, the first field of the state file: 2 while a run is in progress, 8 once one has finished, 1 idle in standby,
    0 with the screen on (C4, 2026-11-23; docs/research/oled-cleaning-signal.md). None when it cannot be read."""
    try:
        out = run("cat %s 2>/dev/null; true" % PNWASH_STATE, 20)
    except Exception:
        return None
    m = re.search(r"^\s*(\d+)(?:\s+\d+)*\s*$", out.replace("\r", ""), re.M)
    return int(m.group(1)) if m else None


def refusal(run):
    """Why the set must not be woken or restarted now, in plain words; None when LG's pixel refresher is not running. The state file is the
    direct signal (2 is running, 8 done, 0 screen on); only when it says neither does the helper's reading (`/v1/panel/refresher`, which
    infers from standby time and the run count) decide."""
    state = read_state(run)
    if state in (0, PNWASH_DONE):
        return None
    how = "its state file says so"
    if state != PNWASH_RUNNING:
        try:
            h = decode(run("curl -fsS -m 5 %s" % HELPER_REFRESHER, 20))
        except Exception:
            return None
        if h.get("running") is not True:
            return None
        how = "the helper's reading says so"
    return ("LG's pixel refresher is running (%s). It takes about 7 minutes and a wake or a restart of the UI cuts it short; try again after it finishes" % how)


def wait_out_refresh(run, standby_since=None, force=False, now=time.time, sleep=time.sleep, say=print):
    """Called with the set in standby, before it is woken. Waits while a run is in progress: while the state file says 2 (at most 15 minutes
    from when it was first seen), else until 15 minutes of standby have passed when a run was due. Returns whether it waited.
    `standby_since` is when the standby began, in `now()`'s clock, where the caller knows."""
    if force:
        say("refresh guard: --force-wake, LG's panel refresh is not checked")
        return False
    state = read_state(run)
    try:
        info = read_info(run)
    except ValueError as e:
        if state != PNWASH_RUNNING:
            say("refresh guard: LG's panel refresh state could not be read (%s); waking anyway" % e)
            return False
        info = None
    t0 = now()
    due_at_start = count_at_start = None
    elapsed0, how = 0.0, "taken as beginning now: where it began is not known"
    if standby_since is not None:
        elapsed0, how = max(0.0, t0 - standby_since), "from the caller"
    else:
        h = helper_standby(run)
        if h:
            elapsed0, due_at_start, count_at_start, how = max(0.0, h["elapsed"]), h["due_at_start"], h["count_at_start"], "from the helper"
    run_seen = t0 if state == PNWASH_RUNNING else None
    waited = False
    while True:
        elapsed = elapsed0 + (now() - t0)
        if state == PNWASH_RUNNING:
            left = max(0, math.ceil(SAFE_STANDBY_S - (now() - run_seen)))
        elif state == PNWASH_DONE and run_seen is not None:
            left = 0
        elif info is None:
            left = 0
        else:
            left = verdict(info, elapsed, due_at_start, count_at_start)
        if left == 0:
            if waited:
                if state == PNWASH_RUNNING:
                    why = "LG's refresher has been running 15 minutes, longer than a run takes"
                elif run_seen is not None and state == PNWASH_DONE:
                    why = "the panel refresh finished (state file says 8)"
                elif elapsed >= SAFE_STANDBY_S:
                    why = "15 minutes of standby have passed"
                else:
                    why = "the panel refresh finished (run %d)" % info["count"]
                say("refresh guard: %s; waking" % why)
            return waited
        if count_at_start is None and info is not None:
            count_at_start = info["count"]
        if not waited:
            if state == PNWASH_RUNNING:
                say("refresh guard: LG's pixel refresher is running (its state file says 2); waiting for it to finish, %d min at most "
                    "(--force-wake skips this)" % ((left + 59) // 60))
            else:
                say("refresh guard: the set has been in standby %d min (%s) and LG's panel refresh is due (run %d at %g h; panel at %.1f h, every %g h); "
                    "waiting until 15 minutes of standby have passed, %d min to go (--force-wake skips this)"
                    % (elapsed // 60, how, info["count"], info["last"], info["usage"], info["interval"], (left + 59) // 60))
        waited = True
        sleep(min(POLL_S, left))
        seen = read_state(run)
        if seen is not None:
            state = seen
            if state == PNWASH_RUNNING and run_seen is None:
                run_seen = now()
        try:
            info = read_info(run)
        except ValueError:
            pass  # the clock still ends the wait


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("verb", choices=["status", "wait", "check"])
    ap.add_argument("--force-wake", action="store_true", help="do not wait (wait: report only)")
    a = ap.parse_args()
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "docs", "research", "arch"))
    import tv
    run = lambda cmd, timeout=20: tv.tv(cmd, timeout=timeout)
    if a.verb == "wait":
        wait_out_refresh(run, force=a.force_wake)
        return
    if a.verb == "check":
        why = refusal(run)
        if why:
            sys.exit("refresh guard: " + why)
        print("LG's pixel refresher is not running")
        return
    print("pnwash state: %s (2 running, 8 done, 1 idle in standby, 0 screen on)" % read_state(run))
    try:
        info = read_info(run)
    except ValueError as e:
        sys.exit("refresh guard: %s" % e)
    print("LG says: run %d at panel hour %g, every %g h, panel at %.1f h, %s" % (info["count"], info["last"], info["interval"], info["usage"], "due" if due(info) else "not due"))
    h = helper_standby(run)
    if h is None:
        print("the helper reports no standby (running with the screen on, not running, or no panel topic)")
        return
    left = verdict(info, h["elapsed"], h["due_at_start"], h["count_at_start"])
    print("standby %d min; a wake would %s" % (h["elapsed"] // 60, "wait %d more min" % ((left + 59) // 60) if left else "go ahead"))


if __name__ == "__main__":
    main()
