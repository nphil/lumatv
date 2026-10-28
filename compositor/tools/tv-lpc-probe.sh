#!/bin/sh
# Read-only probes for the LPC 2026 round, run ON THE SET (`python3 docs/research/arch/tv.py "sh /tmp/lumatv/lpc-probe.sh irq 10 home"`).
# Nothing is written: no sysctl, no IRQ affinity, no cgroup, no priority. The interrupt probe is item 7 of docs/research/lpc-tv-result.md (where the
# interrupts are served, a proposal to be judged against these numbers and never changed by this script), the wake-up probe is item 5 (what wakes at
# idle, a second), the limits probe is item 3 (what the compositor may lock and what the set has left), the swap probe says what fills the swap.
#
#   lpc-probe.sh irq [SECS] [LABEL]      which CPU serves each interrupt over SECS (default 10), at Home and then in a stream (LABEL names the run): every
#                                        line that fired, busiest first, and the lines IRQ_NAMES names (OHCI, xHCI, the display's vsync and osd, the gpu, eth0)
#                                        starred even when quiet, each with the CPUs it may be served on; the softirqs; where the compositor's loop thread
#                                        ran, sampled ten times a second
#   lpc-probe.sh wakeups [SECS] [LABEL]  runs a second of every thread of the compositor, its shell client and the helper (the scheduler's own count of the
#                                        times a thread was put on a CPU, by thread id), with each busy thread's CPU and how much of its running was
#                                        being preempted, and the system's interrupts and context switches a second
#   lpc-probe.sh limits                  the compositor's limit on locked memory and its capabilities, what is resident, locked and swapped, the set's free
#                                        memory, CMA and swap, and the kernel settings the frame path leans on
#   lpc-probe.sh swap [SECS] [LABEL]     what fills the swap: the swap devices, how much of what is used the processes hold (the biggest holders by name,
#                                        LumaTV's three marked) and how much no process holds (tmpfs, shared memory), and whether pages move now (SECS
#                                        of the kernel's swap-in and swap-out counters)
PROC=${LUMATV_PROC:-/proc}
SYS=${LUMATV_SYS:-/sys}
TMPS=${LUMATV_TMP:-/tmp}
IRQ_NAMES=${IRQ_NAMES:-'ohci|xhci|ehci|usb|vsync|de2|osd|gfx|mali|gpu|eth0|enet|gmac'}      # the set's own: ohci_hcd:usb8, DE2VSYNC, osd_irq, c4250000.gpu, eth0 (round 7)
mode=${1:-help}
secs=${2:-10}
label=${3:-run}

comp_pid() { pgrep -f 'bin/lumatv-compositor' | head -n 1; }
# the compositor, the shell client it started (its child) and the helper, on one line
lumatv_pids() {
    c=$(comp_pid)
    echo $c $([ -n "$c" ] && pgrep -P $c) $(pgrep -f 'lumatv-helper start' | head -n 1)
}
# a line of /proc/PID/stat without its two first fields: the process's name may hold spaces, so the fields are counted from the closing bracket
# (state is then field 1, minflt 8, majflt 10, utime 12, stime 13, the CPU it last ran on 37)
stat_after_name() { awk '{ sub(/^.*\) /, ""); print }' "$1" 2>/dev/null; }

case "$mode" in
irq)
    a=$(mktemp); b=$(mktemp); sa=$(mktemp); sb=$(mktemp)
    cat $PROC/interrupts > $a; cat $PROC/softirqs > $sa 2>/dev/null
    pid=$(comp_pid); samples=""; n=0
    # the loop thread's CPU, ten times a second, while the interrupts count
    while [ $n -lt $((secs * 10)) ]; do
        [ -n "$pid" ] && samples="$samples $(stat_after_name $PROC/$pid/task/$pid/stat | awk '{ print $37 }')"
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
    # A thread's runs are field 3 of its schedstat: the times it was put on a CPU. Threads are told apart by their id and never by their name: two
    # QNetworkAccessManager threads of the shell client and the four mali-utility workers share one, and the first reading of this probe, which kept
    # one count a name, gave the first of them the last one's count as its start: 592 runs a second for a network thread whose servers (the compositor's
    # lumatv-shell-http 1.3 a second, the helper's http-accept 0.1) were asked once a second.
    snap() {
        for p in $1; do
            for t in $PROC/$p/task/*; do
                c=$(tr ' ' '_' < $t/comm 2>/dev/null)
                r=$(awk '{ print $3 }' $t/schedstat 2>/dev/null)
                u=$(stat_after_name $t/stat | awk '{ print $12 + $13 }')
                v=$(awk '/^nonvoluntary_ctxt_switches/ { print $2 }' $t/status 2>/dev/null)
                echo "$p ${t##*/} ${c:-?} ${r:-0} ${u:-0} ${v:-0}"
            done
        done
    }
    pids=$(lumatv_pids)
    s0=$(snap "$pids"); i0=$(awk '/^intr / { print $2 }' $PROC/stat); c0=$(awk '/^ctxt / { print $2 }' $PROC/stat)
    sleep "$secs"
    s1=$(snap "$pids"); i1=$(awk '/^intr / { print $2 }' $PROC/stat); c1=$(awk '/^ctxt / { print $2 }' $PROC/stat)
    echo "lpc-probe wakeups: $secs s, $label"
    # per thread: runs a second, CPU % (clock ticks of 10 ms a second), and the preempted runs (nonvoluntary switches) a second; a thread born during
    # the run has no first reading and is left out
    { echo "$s0" | sed 's/^/a /'; echo "$s1" | sed 's/^/b /'; } | awk -v secs="$secs" '
        $1 == "a" { r0[$2 " " $3] = $5; u0[$2 " " $3] = $6; v0[$2 " " $3] = $7; next }
        { k = $2 " " $3; if (!(k in r0)) next
          printf "%s %.3f %s %s %.3f %.3f\n", $2, ($5 - r0[k]) / secs, $3, $4, ($6 - u0[k]) / secs, ($7 - v0[k]) / secs }' | sort -k1,1n -k2,2nr | awk '
        function flush() { if (pid != "") printf "pid %s: %.1f runs/s, %.1f %% CPU:%s\n", pid, runs, cpu, line }
        $1 != pid { flush(); pid = $1; runs = 0; cpu = 0; line = "" }
        { runs += $2; cpu += $5
          pre = ($2 >= 5 && $6 * 2 >= $2) ? sprintf("%.0f %% preempted", $6 * 100 / $2) : ""
          note = ($5 >= 1 ? sprintf("%.1f %% CPU", $5) : "") (($5 >= 1 && pre != "") ? ", " : "") pre
          if ($2 >= 0.5 || $5 >= 1) line = line sprintf(" %s[%s] %.1f%s,", $4, $3, $2, note == "" ? "" : " (" note ")") }
        END { flush() }'
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
        # the page faults since the process started: major ones read a page back from flash or the compressed swap, under the one mmap lock
        stat_after_name $PROC/$pid/stat | awk '{ print "faults: minflt " $8 ", majflt " $10 }'
    fi
    grep -E '^(MemTotal|MemFree|MemAvailable|CmaTotal|CmaFree|SwapTotal|SwapFree|Mlocked|Unevictable)' $PROC/meminfo 2>/dev/null | tr -s ' '
    cat $PROC/swaps 2>/dev/null
    for k in vm/swappiness vm/overcommit_memory vm/max_map_count vm/min_free_kbytes vm/compact_unevictable_allowed kernel/sched_rt_runtime_us kernel/sched_schedstats; do
        echo "$k: $(cat $PROC/sys/$k 2>/dev/null || echo unreadable)"
    done
    ;;
swap)
    kb() { awk -v k="$1:" '$1 == k { print $2 }' $PROC/meminfo; }
    vm() { awk -v k="$1" '$1 == k { print $2 }' $PROC/vmstat 2>/dev/null; }
    mine=$(lumatv_pids)
    in0=$(vm pswpin); out0=$(vm pswpout); maj0=$(vm pgmajfault)
    sleep "$secs"
    in1=$(vm pswpin); out1=$(vm pswpout); maj1=$(vm pgmajfault)
    total=$(kb SwapTotal); free=$(kb SwapFree); used=$((${total:-0} - ${free:-0}))
    # every process's own swapped pages (VmSwap of its status; kernel threads have none), LumaTV's marked
    rows=$(awk -v mine=" $mine " '
        function flush() { if (pid != "") { sum += swap; if (swap > 0) printf "R %d %d %s %s%s\n", swap, rss, pid, name, index(mine, " " pid " ") ? " [LumaTV]" : "" } }
        FNR == 1 { flush(); n = split(FILENAME, f, "/"); pid = f[n - 1]; name = "?"; swap = 0; rss = 0 }
        $1 == "Name:" { name = $2 }
        $1 == "VmSwap:" { swap = $2 }
        $1 == "VmRSS:" { rss = $2 }
        END { flush(); printf "T %d\n", sum }' $PROC/[0-9]*/status 2>/dev/null)
    held=$(echo "$rows" | awk '$1 == "T" { print $2 }')
    echo "lpc-probe swap: $secs s, $label; swap used $used kB of ${total:-0} kB ($((used * 100 / (${total:-0} + (${total:-0} == 0))))%), held by processes ${held:-0} kB, by no process (tmpfs and shared memory, the kernel) $((used - ${held:-0})) kB"
    echo "swap devices:"; cat $PROC/swaps 2>/dev/null
    for z in $SYS/block/zram*; do
        [ -d "$z" ] && echo "${z##*/}: disksize $(cat $z/disksize 2>/dev/null), algorithm $(cat $z/comp_algorithm 2>/dev/null), mm_stat (original, compressed, memory used) $(cat $z/mm_stat 2>/dev/null)"
    done
    echo "pages moved in $secs s: swapped in $(( (${in1:-0} - ${in0:-0}) / secs )) a second, swapped out $(( (${out1:-0} - ${out0:-0}) / secs )) a second, major faults $(( (${maj1:-0} - ${maj0:-0}) / secs )) a second"
    for k in SwapCached Shmem Mlocked Unevictable AnonPages Mapped MemAvailable; do echo "$k: $(kb $k) kB"; done
    echo "LumaTV (compositor, shell client, helper) holds $(echo "$rows" | awk '/LumaTV\]/ { s += $2 } END { print s + 0 }') kB of the swap"
    echo "the biggest holders (kB swapped, kB resident):"
    echo "$rows" | grep '^R ' | sort -k2,2 -n -r | head -n 10 | awk '{ printf "  %8d %8d  pid %-6s %s %s\n", $2, $3, $4, $5, $6 }'
    echo "the biggest of $TMPS (tmpfs pages are swapped out with nobody's name on them):"
    du -sk $TMPS/* 2>/dev/null | sort -k1,1 -n -r | head -n 5 | awk '{ printf "  %8d kB  %s\n", $1, $2 }'
    ;;
*) sed -n '2,/^PROC=/p' "$0" | sed '$d' ;;
esac
