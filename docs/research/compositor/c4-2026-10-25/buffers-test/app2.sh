D=/tmp/lumatv; LIB=$D/lib; LD=$LIB/ld-linux-aarch64.so.1; BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
O=$D/out/app2; rm -rf $O; mkdir -p $O
for B in own driver; do
  XDG_RUNTIME_DIR=$D LUMATV_LOG=info $LD --library-path $LIB $D/bin/lumatv-compositor --runtime-dir $D --socket wayland-lumatv --renderer gl --driver $BLOB --buffers $B --seconds 14 > $O/server-$B.log 2>&1 &
  sleep 2
  (cd /usr/bin && XDG_RUNTIME_DIR=$D WAYLAND_DISPLAY=wayland-lumatv timeout 8 /usr/bin/com.webos.app.tvhotkey '{"appId":"com.webos.app.tvhotkey","launchedHidden":false}' > $O/client-$B.log 2>&1)
  echo "== $B: client exit $?"
  grep -vE 'ELFCLASS|^$' $O/client-$B.log | tail -12
  grep -E 'mapped|disconnected|error|mali' $O/server-$B.log | grep -v ld.so | head -8
  pkill -f 'lumatv-compositor --runtime-dir /tmp/lumatv ' ; sleep 2
done
