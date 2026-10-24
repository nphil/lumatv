
cd /tmp/x2; chmod +x x2srv /tmp/m64/x2srv64
MALI=/usr/lib/libmali.so.0; export XDG_RUNTIME_DIR=/tmp/x2 WAYLAND_DISPLAY=wayland-x2
M64=/tmp/m64; BLOB=$M64/lib/libmali-bifrost-g52-g24p0-wayland-gbm.so
run() { name=$1; shift; srv=$1; shift; srvargs=$1; shift; cliargs=$*
  rm -f wayland-x2 wayland-x2.lock
  ( $srv --seconds 12 $srvargs > $name.srv.log 2>&1 & )
  sleep 1.0
  timeout 8 ./x2cli $MALI $cliargs > $name.cli.log 2>&1; echo "$name: client exit $?"
  n=0; while pgrep -f x2srv >/dev/null && [ $n -lt 14 ]; do sleep 0.5; n=$((n+1)); done
  pkill -x x2srv 2>/dev/null; pkill -x x2srv64 2>/dev/null
  echo "  $(grep -E 'summary|failed|missing' $name.srv.log | head -3)"
}
run r5-egl32 "./x2srv" "--egl $MALI --drm-device /dev/dri/renderD128" --frames 6 --size 256x256
run r8-egl64-render "$M64/lib/ld-linux-aarch64.so.1 --library-path $M64/lib $M64/x2srv64" "--egl $BLOB --drm-device /dev/dri/renderD128" --frames 6 --size 256x256
run r9-egl64-card0 "$M64/lib/ld-linux-aarch64.so.1 --library-path $M64/lib $M64/x2srv64" "--egl $BLOB --drm-device /dev/dri/card0" --frames 6 --size 256x256
echo "left running: $(pgrep -l 'x2srv|x2cli' || echo none)"
grep -E 'CmaFree|MemAvailable' /proc/meminfo
for r in r5-egl32 r8-egl64-render r9-egl64-card0; do echo "===== $r server"; cat $r.srv.log; echo "===== $r client"; grep -vE 'EGL display extensions' $r.cli.log; done
