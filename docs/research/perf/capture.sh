#!/bin/sh
# Compositor-side frame timing capture for LumaTV, run on the set as root.
#
#   capture.sh start NAME [full|basic|kwait]   arm ftrace and a 1 s system sampler
#   capture.sh mark TEXT                 write a marker into the trace
#   capture.sh stop NAME                 stop, dump /tmp/perf/NAME.trace and
#                                        NAME.sys (sampler), restore ftrace
#
# A presented frame is one drm_vblank_event_delivered (the page-flip
# completion the compositor waits on), so the trace counts what the panel
# received, not what QML thinks it drew. The Qt GPU profiler stays off.
# "full" adds sched/irq/mali/vmscan/cpuhp events for root-causing gaps;
# "basic" is the frame events only. Buffer: 8 MB per CPU, ring.
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
block/block_rq_issue
power/cpu_idle"
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

sample() {
  # one line per second: wall, uptime, per-cpu /proc/stat, pressure, memory,
  # vmstat counters, gpu busy/idle, compositor faults, cpu online, temperature
  pid=$(pidof surface-manager)
  while [ -f $D/.running ]; do
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
    echo 0 > $T/events/enable
    echo > $T/trace
    echo 8192 > $T/buffer_size_kb
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
    rm -f $D/.running
    if [ -f $D/.kstack ]; then
      echo "!$(cat $D/.kstack)" > $T/events/sched/sched_switch/trigger 2>/dev/null
      rm -f $D/.kstack
    fi
    echo 0 > $T/events/sched/sched_switch/filter 2>/dev/null
    echo 0 > $T/events/sched/sched_wakeup/filter 2>/dev/null
    cat $T/trace > $D/$name.trace
    echo 0 > $T/events/enable
    echo > $T/trace
    echo 1408 > $T/buffer_size_kb
    wc -l $D/$name.trace
    ;;
  *)
    echo "usage: capture.sh start NAME [full|basic|kwait] | mark TEXT | stop NAME" >&2
    exit 1
    ;;
esac
