#!/bin/sh
# Read-only probes for the LPC 2026 round, run ON THE SET (`python3 docs/research/arch/tv.py "sh /tmp/lumatv/lpc-probe.sh irq 10 home"`).
# Nothing is written: no sysctl, no IRQ affinity, no cgroup, no priority. The interrupt probe is item 7 of docs/research/lpc-tv-result.md (where the
# interrupts are served, a proposal to be judged against these numbers and never changed by this script), the wake-up probe is item 5 (what wakes at
# idle, a second), the limits probe is item 3 (what the compositor may lock and what the set has left).
#
#   lpc-probe.sh irq [SECS] [LABEL]      which CPU serves each interrupt over SECS (default 10), at Home and then in a stream (LABEL names the run): every
#                                        line that fired, busiest first, and the lines IRQ_NAMES names (OHCI, xHCI, the display's vsync and osd, the gpu, eth0)
#                                        starred even when quiet, each with the CPUs it may be served on; the softirqs; where the compositor's loop thread
#                                        ran, sampled ten times a second
#   lpc-probe.sh wakeups [SECS] [LABEL]  runs a second of every thread of the compositor, its shell client and the helper (the scheduler's own count of the
#                                        times a thread was put on a CPU), and the system's interrupts and context switches a second
#   lpc-probe.sh limits                  the compositor's limit on locked memory and its capabilities, what is resident, locked and swapped, the set's free
#                                        memory, CMA and swap, and the kernel settings the frame path leans on
PROC=${LUMATV_PROC:-/proc}
SYS=${LUMATV_SYS:-/sys}
IRQ_NAMES=${IRQ_NAMES:-'ohci|xhci|ehci|usb|vsync|de2|osd|gfx|mali|gpu|eth0|enet|gmac'}      # the set's own: ohci_hcd:usb8, DE2VSYNC, osd_irq, c4250000.gpu, eth0 (round 7)
mode=${1:-help}
secs=${2:-10}
label=${3:-run}

comp_pid() { pgrep -f 'bin/lumatv-compositor' | head -n 1; }

case "$mode" in
irq)
    a=$(mktemp); b=$(mktemp); sa=$(mktemp); sb=$(mktemp)
    cat $PROC/interrupts > $a; cat $PROC/softirqs > $sa 2>/dev/null
    pid=$(comp_pid); samples=""; n=0
    # the loop thread's CPU, ten times a second, while the interrupts count (field 39 of the thread's stat: the CPU it last ran on)
    while [ $n -lt $((secs * 10)) ]; do
        [ -n "$pid" ] && samples="$samples $(awk '{ print $39 }' $PROC/$pid/task/$pid/stat 2>/dev/null)"
        sleep 0.1
        n=$((n+1))
    done
    cat $PROC/interrupts > $b; cat $PROC/softirqs > $sb 2>/dev/null
    echo "lpc-probe irq: $secs s, $label; CPUs online $(cat $SYS/devices/system/cpu/online 2>/dev/null); default affinity $(cat $PROC/irq/default_smp_affinity 2>/dev/null)"
    echo "(* named in IRQ_NAMES)  IRQ  rate/s  per-CPU counts over the run  affinity  name"
    awk -v secs="$secs" -v names="$IRQ_NAMES" -v proc="$PROC" '
        FNR == 1 { file++; if (file == 1) ncpu = NF }
        FNR > 1 {
            id = $1; sub(/:$/, "", id)
            if (file == 1) { for (i = 1; i <= ncpu; i++) first[id, i] = $(i + 1); next }
            total = 0; counts = ""
            for (i = 1; i <= ncpu; i++) { d = $(i + 1) - first[id, i]; total += d; counts = counts sprintf(" %6d", d) }
            desc = ""; for (i = ncpu + 2; i <= NF; i++) desc = desc (desc == "" ? "" : " ") $i
            star = (tolower(desc) ~ names) ? "*" : " "
            if (total > 0 || star == "*") {
                aff = ""; f = proc "/irq/" id "/smp_affinity_list"; if ((getline aff < f) <= 0) aff = "?"; close(f)
                printf "%s%-4s %7.1f %s  %-8s  %s\n", star, id, total / secs, counts, aff, desc
            }
        }' $a $b | sort -k2,2 -n -r
    # the softirqs a frame leans on: the network stack, the timers, tasklets, the scheduler's balancing
    awk '
        FNR == 1 { file++; ncpu = NF; next }
        { name = $1; sub(/:$/, "", name) }
        file == 1 { for (i = 1; i <= ncpu; i++) first[name, i] = $(i + 1); next }
        name == "NET_RX" || name == "TIMER" || name == "TASKLET" || name == "SCHED" { line = ""; for (i = 1; i <= ncpu; i++) line = line sprintf(" %6d", $(i + 1) - first[name, i]); printf "softirq %-8s%s\n", name, line }' $sa $sb 2>/dev/null
    if [ -n "$pid" ]; then
        echo "$samples" | tr ' ' '\n' | awk -v pid="$pid" 'NF { c[$1]++; n++ } END { if (!n) { print "loop thread: not sampled"; exit } out = "loop thread (pid " pid ") ran on:"; for (k in c) out = out sprintf(" CPU%s %d %%,", k, c[k] * 100 / n); print out " over " n " samples" }'
    else
        echo "loop thread: no compositor running"
    fi
    rm -f $a $b $sa $sb
    ;;
wakeups)
    snap() { for p in $1; do for t in $PROC/$p/task/*; do echo "$(tr ' ' '_' < $t/comm 2>/dev/null) $p $(awk '{ print $3 }' $t/schedstat 2>/dev/null)"; done; done; }
    comp=$(comp_pid)
    # the compositor, the shell client it started (its child) and the helper
    pids="$comp $([ -n "$comp" ] && pgrep -P $comp) $(pgrep -f 'lumatv-helper start' | head -n 1)"
    s0=$(snap "$pids"); i0=$(awk '/^intr / { print $2 }' $PROC/stat); c0=$(awk '/^ctxt / { print $2 }' $PROC/stat)
    sleep "$secs"
    s1=$(snap "$pids"); i1=$(awk '/^intr / { print $2 }' $PROC/stat); c1=$(awk '/^ctxt / { print $2 }' $PROC/stat)
    echo "lpc-probe wakeups: $secs s, $label"
    { echo "$s0" | sed 's/^/a /'; echo "$s1" | sed 's/^/b /'; } | awk -v secs="$secs" '
        $1 == "a" { first[$2 " " $3] = $4; next }
        { d = $4 - first[$2 " " $3]; total[$3] += d; thread[$3] = thread[$3] sprintf(" %s %.1f,", $2, d / secs) }
        END { for (p in total) printf "pid %s: %.1f runs/s:%s\n", p, total[p] / secs, thread[p] }' | sort
    echo "system: $(( (i1 - i0) / secs )) interrupts/s, $(( (c1 - c0) / secs )) context switches/s"
    ;;
limits)
    pid=$(comp_pid)
    echo "lpc-probe limits: compositor pid ${pid:-none}"
    if [ -n "$pid" ]; then
        grep -E 'Max locked memory|Max address space|Max realtime priority' $PROC/$pid/limits 2>/dev/null | tr -s ' '
        grep -E '^(VmRSS|VmLck|VmSwap|VmHWM|Threads|CapEff)' $PROC/$pid/status 2>/dev/null | tr -s '\t ' ' '
        # one line a mapping: a count that grows through a run is a mapping made per frame (the driver's buffers, a thread's stack), one that holds is not
        echo "maps: $(wc -l < $PROC/$pid/maps 2>/dev/null)"
        # the page faults since the process started (fields 10 and 12 of its stat): major ones read a page back from flash or the compressed swap, under the one mmap lock
        awk '{ print "faults: minflt " $10 ", majflt " $12 }' $PROC/$pid/stat 2>/dev/null
    fi
    grep -E '^(MemTotal|MemFree|MemAvailable|CmaTotal|CmaFree|SwapTotal|SwapFree|Mlocked|Unevictable)' $PROC/meminfo 2>/dev/null | tr -s ' '
    cat $PROC/swaps 2>/dev/null
    for k in vm/swappiness vm/overcommit_memory vm/max_map_count vm/min_free_kbytes vm/compact_unevictable_allowed kernel/sched_rt_runtime_us kernel/sched_schedstats; do
        echo "$k: $(cat $PROC/sys/$k 2>/dev/null || echo unreadable)"
    done
    ;;
*) sed -n '2,/^PROC=/p' "$0" | sed '$d' ;;
esac
