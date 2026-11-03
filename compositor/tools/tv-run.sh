#!/bin/sh
# The first-light runs, executed ON THE SET as /tmp/lumatv/run.sh (busybox sh). LG's compositor, socket and screen
# are never touched: everything lives under /tmp/lumatv, on a private Wayland socket, with no DRM master.
#
#   run.sh first-light   the X2 client (LG's 32-bit libmali) against the server with the blob serving
#                        mali_buffer_sharing (eglBindWaylandDisplayWL; X2 r8b path)       -> out/driver/
#   run.sh own           same client, the compositor's own mali_buffer_sharing + dma-buf import   -> out/own/
#   run.sh shm           the 64-bit shm test client (no GPU on the client side)            -> out/shm/
#   run.sh app [BIN...]  one unmodified LG app by hand against the socket, 12 s, captures   -> out/app/
#   run.sh protocols     interface tables and versions of LG's client libraries            -> out/protocols.txt
#   run.sh all           first-light, own, shm, protocols
#   run.sh clean         kill anything of ours
# Each case prints PASS/FAIL and the server's summary line; logs and PNGs stay under /tmp/lumatv/out/<case>/.
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
MALI32=/usr/lib/libmali.so.0
SRV="$LD --library-path $LIB $D/bin/lumatv-compositor --runtime-dir $D --log-protocol"
CLI64="$LD --library-path $LIB $D/bin/lumatv-testclient"
# Every request is logged (debug level): six frames are a few hundred lines, and the sequence is the evidence.
export XDG_RUNTIME_DIR=$D WAYLAND_DISPLAY=wayland-lumatv LUMATV_LOG=${LUMATV_LOG:-debug}
# The private loader sets up the blob's glibc; LG's /etc/ld.so.preload names a 32-bit libSegFault, which the
# 64-bit loader skips with a warning (seen in X2 r8b).

. "$D/compositor.sh"
kill_ours() {
    pkill -f 'bin/lumatv-compositor' 2>/dev/null
    pkill -f 'bin/x2cli' 2>/dev/null
    pkill -f 'bin/lumatv-testclient' 2>/dev/null
    rm -f $D/wayland-lumatv $D/wayland-lumatv.lock $D/lumatv-control
}

# start_server NAME [server options]
start_server() {
    name=$1; shift
    rm -rf $D/out/$name; mkdir -p $D/out/$name
    ( $SRV --capture-dir $D/out/$name "$@" > $D/out/$name/server.log 2>&1 & )
    n=0
    while [ ! -S $D/wayland-lumatv ] && [ $n -lt 40 ]; do sleep 0.1; n=$((n+1)); done
    [ -S $D/wayland-lumatv ] || { echo "$name: server did not open the socket"; comp_current_log | head -30; return 1; }
}

# wait_server NAME SECONDS: wait for the server to exit on its own, then make sure it is gone
wait_server() {
    n=0
    while pgrep -f 'bin/lumatv-compositor' >/dev/null && [ $n -lt $(( $2 * 2 )) ]; do sleep 0.5; n=$((n+1)); done
    pkill -f 'bin/lumatv-compositor' 2>/dev/null
    sleep 0.3
}

report() {
    name=$1; want=$2
    log=$D/out/$name/server.log
    comp_current_log >> "$log"
    grep -E ' (warn|error) |egl:|check x2|window [0-9]+ mapped|client bound|create_buffer' $log | head -30
    summary=$(grep '^summary:' $log | tail -1)
    echo "$summary"
    ok=$(echo "$summary" | sed -n 's/.*check ok \([0-9]*\) bad \([0-9]*\).*/\1/p')
    bad=$(echo "$summary" | sed -n 's/.*check ok \([0-9]*\) bad \([0-9]*\).*/\2/p')
    if [ "${ok:-0}" -ge "$want" ] && [ "${bad:-1}" -eq 0 ]; then echo "$name: PASS ($ok frames pixel-exact)"; else echo "$name: FAIL (ok ${ok:-?} bad ${bad:-?}, want $want)"; fi
    ls $D/out/$name/*.png 2>/dev/null | wc -l | sed "s/^/$name: PNG frames: /"
}

case_driver() {
    kill_ours
    start_server driver --driver $BLOB --buffers driver --check x2 --frames 6 --seconds 25 --exit-on-disconnect || return 1
    timeout 12 $D/bin/x2cli $MALI32 --frames 6 --size 1920x1080 > $D/out/driver/client.log 2>&1; echo "driver: client exit $?"
    wait_server driver 20
    grep -E 'global|EGL [0-9]|GL "|frame [0-9]' $D/out/driver/client.log | head -20
    report driver 6
}

case_own() {
    kill_ours
    start_server own --driver $BLOB --buffers own --check x2 --frames 6 --seconds 25 --exit-on-disconnect || return 1
    timeout 12 $D/bin/x2cli $MALI32 --frames 6 --size 1920x1080 > $D/out/own/client.log 2>&1; echo "own: client exit $?"
    wait_server own 20
    grep -E 'global|EGL [0-9]|GL "|frame [0-9]' $D/out/own/client.log | head -20
    report own 6
}

case_shm() {
    kill_ours
    start_server shm --driver $BLOB --buffers driver --check x2 --frames 6 --seconds 25 --exit-on-disconnect || return 1
    timeout 15 $CLI64 --frames 6 --size 1920x1080 > $D/out/shm/client.log 2>&1; echo "shm: client exit $?"
    wait_server shm 20
    tail -4 $D/out/shm/client.log
    report shm 6
}

# One LG app launched by hand: its luna traffic still goes to LG's services, only its Wayland comes to us.
case_app() {
    kill_ours
    if [ $# -gt 0 ]; then apps="$*"; else apps="/usr/bin/com.webos.exampleapp.systemui /usr/bin/com.webos.app.tvhotkey"; fi
    start_server app --driver $BLOB --buffers driver --seconds 20 --capture-every 30 || return 1
    for bin in $apps; do
        [ -x "$bin" ] || { echo "app: $bin not executable, skipped"; continue; }
        echo "app: launching $bin"
        ( cd "$(dirname $bin)" && timeout 12 "$bin" '{"appId":"com.webos.exampleapp.systemui","launchedHidden":false}' > $D/out/app/client-$(basename $bin).log 2>&1 & )
        sleep 8
        $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D windows > $D/out/app/windows-$(basename $bin).json 2>&1
        $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D capture $D/out/app/app-$(basename $bin).png > /dev/null 2>&1
        head -c 600 $D/out/app/windows-$(basename $bin).json; echo
        pkill -f "$bin" 2>/dev/null
        sleep 1
    done
    $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D journal > $D/out/app/journal.json 2>&1
    $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir $D quit > /dev/null 2>&1
    wait_server app 5
    grep -E 'window [0-9]+ (created|mapped)|set_property|client (connected|disconnected)|warn|error' $D/out/app/server.log | head -40
    ls -la $D/out/app/*.png 2>/dev/null
}

# wlproto_from_elf.py dumps only the *_interface symbols not starting with wl_ by default, so the LG ones are named
# explicitly; a library without the symbol (libgm-wayland links them statically) falls back to the tool's name scan.
WEBOS_IFACES="wl_webos_shell_interface wl_webos_shell_surface_interface wl_webos_foreign_interface wl_webos_exported_interface wl_webos_imported_interface \
 wl_webos_surface_group_compositor_interface wl_webos_surface_group_interface wl_webos_surface_group_layer_interface wl_webos_input_manager_interface \
 wl_webos_seat_interface wl_webos_gyroscope_interface wl_webos_accelerometer_interface wl_webos_xinput_interface wl_webos_xinput_extension_interface \
 wl_webos_system_ux_interface wl_webos_tablet_interface text_model_interface text_model_factory_interface input_method_interface input_method_context_interface \
 input_panel_interface input_panel_surface_interface"
case_protocols() {
    out=$D/out/protocols.txt
    : > $out
    if ! command -v python3 >/dev/null 2>&1; then
        echo "(no python3 on the set: fetch the libraries and run docs/research/arch/wlproto_from_elf.py on the workstation)" >> $out
        wc -l $out; return
    fi
    dump() { lib=$1; shift; [ -f "$lib" ] || return; echo "== $lib" >> $out; python3 $D/wlproto_from_elf.py "$lib" "$@" >> $out 2>&1 || echo "(wlproto_from_elf failed)" >> $out; }
    dump /usr/lib/libwayland-webos-client.so.1.0.0 $WEBOS_IFACES
    dump /usr/lib/libwayland-webos-server.so.1.0.0 $WEBOS_IFACES
    dump /usr/lib/libwebos-platform-interface.so.1.0.0 $WEBOS_IFACES
    dump /usr/lib/libgm-wayland.so.1.0.0 wl_webos_shell_interface wl_webos_shell_surface_interface wl_webos_foreign_interface wl_webos_exported_interface wl_webos_imported_interface wl_webos_surface_group_compositor_interface wl_webos_surface_group_interface
    dump /usr/lib/libwayland-starfish-client.so.1.0.0 wl_starfish_pointer_interface wl_starfish_output_interface
    dump /usr/lib/libwayland-input-helper.so.1.0.0 wl_webos_shell_interface wl_webos_shell_surface_interface
    dump /usr/lib/flutter/3.22.2/release/libflutter_elinux_wayland.so wl_webos_shell_interface wl_webos_shell_surface_interface wl_webos_surface_group_interface wl_webos_surface_group_layer_interface wl_webos_foreign_interface wl_webos_xinput_interface wp_presentation_interface
    dump /usr/lib/libQt5WaylandClient.so.5.12.4
    dump /usr/lib/libWebOSCoreCompositor.so.0.0.1
    dump /usr/lib/plugins/compositor/libstarfishcompositorplugin.so wl_starfish_pointer_interface wl_starfish_output_interface
    wc -l $out
}

mkdir -p $D/out
case "${1:-help}" in
    first-light) case_driver ;;
    own) case_own ;;
    shm) case_shm ;;
    app) shift; case_app "$@" ;;
    protocols) case_protocols ;;
    all) case_driver; case_own; case_shm; case_protocols ;;
    clean) kill_ours; echo "left running: $(pgrep -fl 'lumatv|x2cli' || echo none)" ;;
    *) sed -n 2,13p $0 ;;
esac
