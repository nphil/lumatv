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
sec "libraries mapped by Netflix (preloaded under LG's compositor) that carry wl_webos_imported"
# The media connection that died on opcode 7 is Netflix's own second connection, so its maps name the library.
nf=$(pgrep -f 'applications/netflix/bin/netflix.bin' | head -1)
cands=$( [ -n "$nf" ] && awk '{print $6}' /proc/$nf/maps 2>/dev/null | grep '\.so' | sort -u | sed 's#^/var/palm/jail/netflix##' |
         while read l; do [ -f "$l" ] && timeout 10 grep -l 'wl_webos_imported' "$l" 2>/dev/null; done)
echo "netflix pid ${nf:-none}"; echo "$cands"
if command -v python3 >/dev/null 2>&1; then
    for lib in $cands; do
        echo; echo "== $lib"
        timeout 30 python3 /tmp/lumatv/wlproto_from_elf.py "$lib" wl_webos_shell_interface wl_webos_shell_surface_interface \
            wl_webos_foreign_interface wl_webos_exported_interface wl_webos_imported_interface wl_webos_surface_group_compositor_interface \
            wl_webos_surface_group_interface wl_webos_surface_group_layer_interface wl_webos_input_manager_interface wl_webos_seat_interface \
            wl_webos_xinput_interface wl_webos_xinput_extension_interface 2>&1 | grep -v 'not exported and no struct found'
    done
fi
sec "LG's volume, mute and system UI calls (strings of the compositor's QML service plugins)"
# What LG's key filters call for Volume, Mute, the gear and Input keys; LumaTV's compositor makes the same calls.
for f in $(timeout 20 find /usr/lib/qt5/qml/WebOSServices /usr/lib/qt5/qml/WebOSCoreCompositor /usr/lib/qt5/qml/Starfish* -name '*.so' 2>/dev/null) /usr/lib/libWebOSCoreCompositor.so.0.0.1; do
    [ -f "$f" ] || continue
    hits=$(timeout 20 grep -o -E 'luna://[a-zA-Z0-9_.]+/[a-zA-Z0-9_/]*(olume|ute|udio)[a-zA-Z0-9_/]*|com\.webos\.service\.audio[a-zA-Z0-9_./]*|/master/[a-zA-Z]+' "$f" 2>/dev/null | sort -u | tr '\n' ' ')
    [ -n "$hits" ] && echo "$f: $hits"
done
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
