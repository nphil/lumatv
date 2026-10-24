#!/bin/sh
# X4: read-only census of luna traffic around the compositor. One unfiltered ls-monitor (the hub mirrors every
# message to a monitor whatever the filter, so one unfiltered monitor costs the hub the same as a filtered one and
# captures every bus name at once). Output is JSON lines on tmpfs. Usage: x4-monitor.sh start|stop|status
D=/tmp/x4
case "$1" in
start)
  mkdir -p $D
  if [ -f $D/monitor.pid ] && kill -0 "$(cat $D/monitor.pid)" 2>/dev/null; then echo "already running"; exit 0; fi
  date +%s > $D/start.ts
  cat /proc/uptime > $D/start.uptime
  ( timeout 1800 ls-monitor -j -e > $D/all.jsonl 2> $D/monitor.err & echo $! > $D/monitor.pid )
  sleep 1
  echo "started pid $(cat $D/monitor.pid): $(pgrep -l ls-monitor)"
  ;;
stop)
  [ -f $D/monitor.pid ] && kill "$(cat $D/monitor.pid)" 2>/dev/null
  pkill -x ls-monitor 2>/dev/null
  sleep 1
  date +%s > $D/stop.ts
  echo "stopped; remaining: $(pgrep -l ls-monitor || echo none); lines $(wc -l < $D/all.jsonl) bytes $(stat -c %s $D/all.jsonl)"
  ;;
status)
  echo "running: $(pgrep -l ls-monitor || echo none); lines $(wc -l < $D/all.jsonl 2>/dev/null) bytes $(stat -c %s $D/all.jsonl 2>/dev/null)"
  ;;
esac
