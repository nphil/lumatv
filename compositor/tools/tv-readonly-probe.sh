#!/bin/sh
# Read-only probe, executed ON THE SET as /tmp/lumatv/probe.sh (tv-first-light.sh probe). Nothing is written outside
# /tmp/lumatv/out, no device is opened, nothing is restarted; safe beside another agent's session.
#   1. Every library or app binary that carries wl_webos_* protocol tables (strings), with the tables dumped by
#      wlproto_from_elf.py: the copies the clients really use, Netflix's media path included.
#   2. Every process that has libwayland-client mapped while LG's compositor runs: pid, uid, unit, command line.
#      Under LumaTV's compositor one of them came and went every 7 s; this is the list to match it against.
#   3. Units that restart: NRestarts and RestartUSec of the units those processes belong to.
OUT=/tmp/lumatv/out/probe; mkdir -p $OUT
sec() { echo; echo "=== $*"; }
{
sec "release"; head -3 /etc/starfish-release 2>/dev/null
sec "libraries and binaries naming wl_webos_imported or attach_punchthrough"
# Regular files only, bounded (a recursive grep over FIFOs hung the first inspection).
cands=$( (timeout 60 find /usr/lib /usr/bin /usr/sbin -maxdepth 3 -type f \( -name '*.so*' -o -perm -u+x \) -size -60M 2>/dev/null;
          timeout 30 find /media/cryptofs/apps/usr/palm/applications/netflix /media/cryptofs/apps/usr/palm/applications/amazon \
                           /media/cryptofs/apps/usr/palm/applications/youtube.leanback.v4 -maxdepth 3 -type f -size -60M 2>/dev/null) |
         xargs timeout 120 grep -l -a -E 'wl_webos_imported|attach_punchthrough' 2>/dev/null | sort -u)
echo "$cands"
if command -v python3 >/dev/null 2>&1; then
    for lib in $cands; do
        echo; echo "== $lib"
        timeout 30 python3 /tmp/lumatv/wlproto_from_elf.py "$lib" wl_webos_shell_interface wl_webos_shell_surface_interface \
            wl_webos_foreign_interface wl_webos_exported_interface wl_webos_imported_interface wl_webos_surface_group_compositor_interface \
            wl_webos_surface_group_interface wl_webos_surface_group_layer_interface wl_webos_input_manager_interface wl_webos_seat_interface \
            wl_webos_xinput_interface wl_webos_xinput_extension_interface 2>&1 | grep -v 'not exported and no struct found'
    done
fi
sec "processes with libwayland-client mapped (pid uid unit cmdline)"
for d in /proc/[0-9]*; do
    p=${d#/proc/}
    grep -q 'libwayland-client' $d/maps 2>/dev/null || continue
    uid=$(awk '/^Uid:/{print $2}' $d/status 2>/dev/null)
    unit=$(grep -m1 -oE '[^/]+\.(service|scope)' $d/cgroup 2>/dev/null)
    start=$(awk '{print $22}' $d/stat 2>/dev/null)
    echo "$p uid=$uid unit=${unit:-?} start_ticks=$start exe=$(readlink $d/exe 2>/dev/null) cmd=$(tr '\0' ' ' < $d/cmdline 2>/dev/null | cut -c1-200)"
done
sec "their units: restarts"
for d in /proc/[0-9]*; do grep -q 'libwayland-client' $d/maps 2>/dev/null && grep -m1 -oE '[^/]+\.service' $d/cgroup 2>/dev/null; done | sort -u |
    while read u; do echo "$u $(systemctl show $u -p Restart,RestartUSec,NRestarts 2>/dev/null | tr '\n' ' ')"; done
sec "uptime"; cat /proc/uptime
} > $OUT/probe.txt 2>&1
wc -l $OUT/probe.txt
