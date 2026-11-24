#!/bin/sh
# Rate-follow test (docs/research/decoded-vrr-4k.md, "Output rate following the source"). Runs on the set as root: plays a black clip through the
# feeder's ES pipe at each rate in turn, under the UI, and samples the panel's rate and mute state every 100 ms. Writes no driver control or setting.
#   rate_follow.sh OUT CLIP WxH RATE...
OUT=$1; CLIP=$2; SIZE=$3; shift 3
FEEDER=/var/lib/lumatv/bin/lumatv-gfn-feeder
WIN0=/proc/lg/de/win0/status
OVI=/proc/lg/ovi/status
SCALER=/proc/lgtv-driver/scaler/main.p0
mkdir -p "$OUT"
S="$OUT/samples.txt"

irq() { awk '$NF=="osd_irq"{o=$2} $NF=="DE2VSYNC"{d=$2} END{printf "%s %s", o, d}' /proc/interrupts; }
sample() { # one line: uptime osd de2 localmute vFreq black_out src label
  read -r up _ < /proc/uptime
  lm=$(sed -n 's/^localmute=//p' $WIN0); vf=$(sed -n 's/^vFreq *: *//p' $WIN0)
  bo=$(sed -n 's/^lvds.black_out: *//p' $OVI | tr ' ' '_')
  src=$(sed -n 's/^connect_src=//p' $SCALER 2>/dev/null)
  echo "$up $(irq) $lm $vf $bo $src $1" >> "$S"
}
# The rate over the last second, from the last 11 samples: "osdHz deHz".
rate() { tail -11 "$S" | awk 'NR==1{t=$1;o=$2;d=$3} END{dt=$1-t; if(dt>0) printf "%.1f %.1f", ($2-o)/dt, ($3-d)/dt; else printf "0 0"}'; }
watch() { # watch SECONDS LABEL: sample every 100 ms; returns 1 when the picture stays muted or the UI rate collapses for 2 s
  n=$(( $1 * 10 )); bad=0
  while [ $n -gt 0 ]; do
    sample "$2"; usleep 100000; n=$((n - 1))
    set -- "$1" "$2" $(rate)
    low=$(awk -v r="$3" 'BEGIN{print (r>0 && r<40) ? 1 : 0}')
    case "$(tail -1 "$S" | cut -d' ' -f6)" in orignal*) blk=0 ;; *) blk=1 ;; esac
    if [ "$low" = 1 ] || [ "$blk" = 1 ]; then bad=$((bad + 1)); else bad=0; fi
    [ $bad -ge 20 ] && return 1
  done
  return 0
}
COMP=/var/lib/lumatv/compositor
ctl() { "$COMP/lib/ld-linux-aarch64.so.1" --library-path "$COMP/lib" "$COMP/bin/lumatv-compositor" ctl --runtime-dir /tmp/xdg "$@"; }
# The compositor closes a video window nobody claimed (3 s), so each clip claims a small one, as the video check does. Only PIPE=smp shows the
# display engine the clip's rate: the ES and NDL pipes present 60 fps whatever --fps and LOWDELAY say (2026-11-24). CODEC is h264 unless set.
play() { # play FPS SECONDS
  ctl vwin claim ratefollow pip 1280 720 480 270 org.nphil.lumatv.videocheck "${CODEC:-h264}" "$SIZE@$1" > "$OUT/claim-$1.json" 2>&1
  XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0 "$FEEDER" --play "$CLIP" --size "$SIZE" --codec "${CODEC:-h264}" --fps "$1" --seconds "$2" --loop \
    --pipe "${PIPE:-smp}" --low-delay "${LOWDELAY:-0}" -w video -a org.nphil.lumatv.videocheck --ndl-app com.webos.app.home > "$OUT/feeder-$1.log" 2>&1 &
  PID=$!
}
stop() { kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; ctl vwin release ratefollow > /dev/null 2>&1; }

read -r T0 _ < /proc/uptime
watch 2 idle
for fps in "$@"; do
  play "$fps" 9
  if ! watch 8 "play$fps"; then
    stop; echo "STOP at $fps: muted or under 40 Hz for 2 s" >> "$OUT/result.txt"
    play 30 6; watch 5 recover30; stop; watch 3 after-recover
    break
  fi
  echo "$fps $(rate)" >> "$OUT/result.txt"
  stop
  watch 3 "after$fps"
  echo "after$fps $(rate)" >> "$OUT/result.txt"
done
dmesg | awk -v t="$T0" -F'[][]' '$2+0 >= t' | grep -i -E 'Hz model|fr_rate|pulldown|timing_changed|DCO' > "$OUT/dmesg.txt" 2>&1
cat "$OUT/result.txt"
