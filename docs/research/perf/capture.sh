#!/bin/sh
# Compositor-side frame timing capture for LumaTV, run on the set as root.
#
#   capture.sh start NAME [full|basic|kwait]   arm ftrace and a 1 s system sampler
#   capture.sh mark TEXT                 write a marker into the trace
#   capture.sh stop NAME                 stop, dump /tmp/perf/NAME.trace and
#                                        NAME.sys (sampler), restore ftrace
#   capture.sh clean [PREFIX]            stop whatever a dead run left (tracing, sampler, trigger), restore ftrace and delete
#                                        PREFIX*.trace / PREFIX*.sys (all of them without a PREFIX)
#
# /tmp on the set is a RAM-backed tmpfs, so nothing here may outlive its run: `stop` ends the sampler for good; `start` first ends any sampler and
# trigger an interrupted run left and deletes traces and samples older than LUMATV_PERF_STALE_MIN (default 30) minutes; a trace is cut at
# LUMATV_PERF_TRACE_MAX_MB (default 128; the full set of a 10 s window is about 70 MB on the set's 3 CPUs) with a note on stderr; the sampler ends
# itself after LUMATV_PERF_SAMPLER_MAX_S (default 1800) and turns tracing off then. The caller (run.py, gate.py) deletes what it pulled and calls `clean`.
#
# A presented frame is one drm_vblank_event_delivered (the page-flip
# completion the compositor waits on), so the trace counts what the panel
# received, not what QML thinks it drew. The Qt GPU profiler stays off.
# "full" adds sched/irq/mali/vmscan/cpuhp events for root-causing gaps;
# "basic" is the frame events only. Buffer: LUMATV_PERF_BUFFER_KB a CPU (default 16 MB), a ring: when it is full the oldest events go, and the
# dump's header says how many (`entries-in-buffer/entries-written`; analyse.py leaves a window the loss reaches unread). CPU 0 takes every device
# interrupt of the set, 11k to 33k events a second (2026-11-04, a USB game-controller dongle at 1 kHz, the network, video decode): 8 MB held
# 8 to 12 s of it, and 26 of 41 captures began without their first 0.1 to 4.8 s.
# "kwait" is for a long run (the 60 s x1 probe): the frame events, and the scheduler events of the compositor's own threads only,
# with the block, reclaim, CMA and GPU job events the waits are labelled from; a minute fits the ring where the full set holds 13 s.
# In "full" and "kwait" a `stacktrace` trigger leaves the kernel stack of each compositor thread that goes to sleep in D state
# (blocked in a lock, I/O or the driver); `kwait.py` labels each wait from it (kbase, mmap_sem, io, ...). A kernel without the
# trigger says so and the waits are labelled from the events around them instead.
# `start` also writes `loop TID` (the compositor's process id, its loop thread's) into the trace, which is how `kwait.py` finds the loop:
# run through its private loader the compositor's threads are named `ld-linux-aarch6`, like the X1 client's.
#
# The 1 s sampler is sample.py (one process, no forks) when it sits beside this script. The shell
# `sample` below forks about 40 processes a second; on the set that was 50-60 ms of CPU, and the
# scheduler's exec-time migrations, inside the first window of every full capture.
T=${LUMATV_TRACEFS:-/sys/kernel/debug/tracing}
D=${LUMATV_PERF_DIR:-/tmp/perf}
PROC=${LUMATV_PROC:-/proc}
MAX_MB=${LUMATV_PERF_TRACE_MAX_MB:-128}
STALE_MIN=${LUMATV_PERF_STALE_MIN:-30}
SAMPLER_MAX_S=${LUMATV_PERF_SAMPLER_MAX_S:-1800}
mkdir -p $D

events_full="drm/drm_vblank_event_delivered drm/drm_vblank_event
sched/sched_switch sched/sched_wakeup sched/sched_migrate_task
irq/irq_handler_entry irq/irq_handler_exit irq/softirq_entry irq/softirq_exit
mali/mali_pm_status mali/mali_job_slots_event
cpuhp/cpuhp_enter cpuhp/cpuhp_exit
vmscan/mm_vmscan_direct_reclaim_begin vmscan/mm_vmscan_direct_reclaim_end
vmscan/mm_vmscan_kswapd_wake vmscan/mm_vmscan_kswapd_sleep
compaction/mm_compaction_begin compaction/mm_compaction_end
cma/cma_alloc cma/cma_release
workqueue/workqueue_execute_start workqueue/workqueue_execute_end
block/block_rq_issue"
events_basic="drm/drm_vblank_event_delivered drm/drm_vblank_event"
events_kwait="drm/drm_vblank_event_delivered drm/drm_vblank_event sched/sched_switch sched/sched_wakeup
block/block_rq_issue
vmscan/mm_vmscan_direct_reclaim_begin vmscan/mm_vmscan_direct_reclaim_end
compaction/mm_compaction_begin compaction/mm_compaction_end cma/cma_alloc
mali/mali_job_slots_event"

# The compositor's process id, which is its loop thread's id (LumaTV's, else LG's surface-manager), its threads' ids, and a filter that names
# them: `FIELD == A || FIELD == B`.
comp_pid() {
  pid=$(pgrep -f 'bin/lumatv-compositor' | head -n 1)
  [ -n "$pid" ] || pid=$(pidof surface-manager | cut -d' ' -f1)
  echo "$pid"
}
comp_tids() {
  pid=$(comp_pid)
  [ -n "$pid" ] && ls $PROC/$pid/task 2>/dev/null
}
tid_filter() {
  f=""
  for t in $(comp_tids); do f="${f:+$f || }$1 == $t"; done
  echo "$f"
}

# The kernel stack of a compositor thread at the moment it blocks in D state (prev_state bit 2).
arm_kstack() {
  f=$(tid_filter prev_pid)
  [ -n "$f" ] || { echo "no compositor thread to take kernel stacks of" >&2; return; }
  st="stacktrace if prev_state & 2 && ($f)"
  if echo "$st" > $T/events/sched/sched_switch/trigger 2>/dev/null; then
    echo "$st" > $D/.kstack
  else
    echo "no stacktrace trigger on this kernel: kernel waits are labelled from context" >&2
  fi
}

# End the sampler of this directory and forget it. The pid is only signalled if it still is one of ours (sample.py, or the shell sampler, a
# subshell of this script).
stop_sampler() {
  rm -f $D/.running
  p=$(cat $D/.sampler 2>/dev/null)
  rm -f $D/.sampler
  [ -n "$p" ] || return 0
  case "$(tr '\0' ' ' < $PROC/$p/cmdline 2>/dev/null)" in
    *sample.py*|*capture.sh*) kill $p 2>/dev/null ;;
  esac
  return 0
}

# Take the stack trigger and the thread filters off the sched events (start sets them for kwait/full, stop and a dead run's cleanup put them back).
disarm_kstack() {
  if [ -f $D/.kstack ]; then
    echo "!$(cat $D/.kstack)" > $T/events/sched/sched_switch/trigger 2>/dev/null
    rm -f $D/.kstack
  fi
  echo 0 > $T/events/sched/sched_switch/filter 2>/dev/null
  echo 0 > $T/events/sched/sched_wakeup/filter 2>/dev/null
}

sweep_stale() {
  find $D -maxdepth 1 -type f \( -name '*.trace' -o -name '*.sys' \) -mmin +$STALE_MIN -exec rm -f {} \; 2>/dev/null
  return 0
}

sample() {
  # one line per second: wall, uptime, per-cpu /proc/stat, pressure, memory,
  # vmstat counters, gpu busy/idle, compositor faults, cpu online, temperature
  pid=$(pidof surface-manager)
  i=0
  while [ -f $D/.running ]; do
    if [ $i -ge $SAMPLER_MAX_S ]; then rm -f $D/.running; echo 0 > $T/tracing_on; break; fi
    i=$((i + 1))
    up=$(cut -d' ' -f1 /proc/uptime)
    cpu=$(grep -E '^cpu[0-9]' /proc/stat | tr '\n' ';')
    psi="$(sed -n 1p /proc/pressure/cpu)|$(sed -n 1p /proc/pressure/memory)|$(sed -n 2p /proc/pressure/memory)|$(sed -n 1p /proc/pressure/io)"
    mem=$(grep -E '^(MemFree|MemAvailable|CmaFree|SwapFree|Dirty)' /proc/meminfo | tr -s ' ' | tr '\n' ';')
    vm=$(grep -E '^(pgscan_kswapd|pgsteal_kswapd|pgscan_direct|pgsteal_direct|allocstall_normal|allocstall_movable|pswpin|pswpout|pgmajfault|compact_stall|nr_free_cma) ' /proc/vmstat | tr '\n' ';')
    gpu=$(cat /sys/kernel/debug/mali0/dvfs_utilization 2>/dev/null | tr '\n' ' ')
    gpust=$(cat /sys/class/misc/mali0/device/power/runtime_status 2>/dev/null)
    sm=$(awk '{print "minflt="$10" majflt="$12" utime="$14" stime="$15" rss="$24}' /proc/$pid/stat 2>/dev/null)
    online=$(cat /sys/devices/system/cpu/online)
    temp=$(cat /proc/lg/pm/temperature 2>/dev/null)
    lgload=$(grep '^load:' /proc/lg/pm/status 2>/dev/null | tr -s ' ')
    echo "$up|$cpu|$psi|$mem|$vm|gpu=$gpu|gpust=$gpust|$sm|online=$online|temp=$temp|$lgload"
    sleep 1
  done
}

case "$1" in
  start)
    name=$2; mode=${3:-full}
    echo 0 > $T/tracing_on
    stop_sampler
    disarm_kstack
    sweep_stale
    echo 0 > $T/events/enable
    echo > $T/trace
    echo ${LUMATV_PERF_BUFFER_KB:-16384} > $T/buffer_size_kb
    echo local > $T/trace_clock
    case "$mode" in full) ev="$events_full" ;; kwait) ev="$events_kwait" ;; *) ev="$events_basic" ;; esac
    if [ "$mode" = kwait ]; then
      f=$(tid_filter prev_pid); g=$(tid_filter next_pid); w=$(tid_filter pid)
      [ -n "$f" ] && echo "($f) || ($g)" > $T/events/sched/sched_switch/filter 2>/dev/null
      [ -n "$w" ] && echo "$w" > $T/events/sched/sched_wakeup/filter 2>/dev/null
    fi
    for e in $ev; do echo 1 > $T/events/$e/enable 2>/dev/null || echo "no event $e" >&2; done
    if [ "$mode" = full ] || [ "$mode" = kwait ]; then arm_kstack; fi
    touch $D/.running
    if [ -f $D/sample.py ]; then
      set -- $(pidof surface-manager)
      python3 $D/sample.py $D/$name.sys ${1:-0} $D > /dev/null 2>&1 &
    else
      sample > $D/$name.sys 2>/dev/null &
    fi
    echo $! > $D/.sampler
    echo 1 > $T/tracing_on
    echo "start $name $(cut -d' ' -f1 /proc/uptime)" > $T/trace_marker
    # The loop thread's id, for kwait.py: the compositor runs under its private loader, so its threads are named `ld-linux-aarch6`, which names nothing.
    loop=$(comp_pid)
    if [ -n "$loop" ]; then echo "loop $loop" >> $T/trace_marker; fi
    ;;
  mark)
    echo "$2" > $T/trace_marker
    ;;
  stop)
    name=$2
    echo "stop $name $(cut -d' ' -f1 /proc/uptime)" > $T/trace_marker
    echo 0 > $T/tracing_on
    stop_sampler
    disarm_kstack
    # `head -c` ends the read at the cap; the dump is cut, so its header's loss count and the stop marker may be missing.
    cat $T/trace | head -c $((MAX_MB * 1048576)) > $D/$name.trace
    if [ "$(wc -c < $D/$name.trace)" -ge $((MAX_MB * 1048576)) ]; then
      echo "trace $name cut at $MAX_MB MB (LUMATV_PERF_TRACE_MAX_MB)" >&2
    fi
    echo 0 > $T/events/enable
    echo > $T/trace
    echo 1408 > $T/buffer_size_kb
    wc -l $D/$name.trace
    ;;
  clean)
    echo 0 > $T/tracing_on 2>/dev/null
    stop_sampler
    disarm_kstack
    echo 0 > $T/events/enable 2>/dev/null
    echo > $T/trace 2>/dev/null
    echo 1408 > $T/buffer_size_kb 2>/dev/null
    rm -f $D/"$2"*.trace $D/"$2"*.sys
    ;;
  *)
    echo "usage: capture.sh start NAME [full|basic|kwait] | mark TEXT | stop NAME | clean [PREFIX]" >&2
    exit 1
    ;;
esac
