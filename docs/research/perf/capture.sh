#!/bin/sh
# Compositor-side frame timing capture for LumaTV, run on the set as root.
#
#   capture.sh start NAME [full|basic]   arm ftrace and a 1 s system sampler
#   capture.sh mark TEXT                 write a marker into the trace
#   capture.sh stop NAME                 stop, dump /tmp/perf/NAME.trace and
#                                        NAME.sys (sampler), restore ftrace
#
# A presented frame is one drm_vblank_event_delivered (the page-flip
# completion the compositor waits on), so the trace counts what the panel
# received, not what QML thinks it drew. The Qt GPU profiler stays off.
# "full" adds sched/irq/mali/vmscan/cpuhp events for root-causing gaps;
# "basic" is the frame events only. Buffer: 8 MB per CPU, ring.
T=/sys/kernel/debug/tracing
D=/tmp/perf
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
    if [ "$mode" = full ]; then ev="$events_full"; else ev="$events_basic"; fi
    for e in $ev; do echo 1 > $T/events/$e/enable 2>/dev/null || echo "no event $e" >&2; done
    touch $D/.running
    sample > $D/$name.sys 2>/dev/null &
    echo $! > $D/.sampler
    echo 1 > $T/tracing_on
    echo "start $name $(cut -d' ' -f1 /proc/uptime)" > $T/trace_marker
    ;;
  mark)
    echo "$2" > $T/trace_marker
    ;;
  stop)
    name=$2
    echo "stop $name $(cut -d' ' -f1 /proc/uptime)" > $T/trace_marker
    echo 0 > $T/tracing_on
    rm -f $D/.running
    cat $T/trace > $D/$name.trace
    echo 0 > $T/events/enable
    echo > $T/trace
    echo 1408 > $T/buffer_size_kb
    wc -l $D/$name.trace
    ;;
  *)
    echo "usage: capture.sh start NAME [full|basic] | mark TEXT | stop NAME" >&2
    exit 1
    ;;
esac
