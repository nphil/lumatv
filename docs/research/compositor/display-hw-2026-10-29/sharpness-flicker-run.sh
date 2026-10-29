G=/proc/lg/fbdev/chip/osd_sr_gain; S=/proc/lg/fbdev/chip/osd_sr; DIAG=/proc/lg/fbdev/chip/diag
A=luna://com.webos.service.applicationmanager
ts() { date +%T; }
cur() { echo "gain=$(grep ti_gain $DIAG | head -1 | awk '{print $2}') sr=$(grep sw_status $DIAG | awk '{print $2}')"; }
set_state() {
  case $1 in
    NORMAL) echo 0x10 > $G; echo 1 > $S ;;
    STRONG) echo 1 > $S; echo 0x80 > $G ;;
    SOFT) echo 1 > $S; echo 0x00 > $G ;;
    OFF) echo 0x10 > $G; echo 0 > $S ;;
  esac
  echo "$(ts) $1: $(cur)"
}
wait_until() { while [ $(date +%s) -lt $1 ]; do sleep 0.2; done; }
echo "$(ts) before: $(cur)"
setsid nohup sh -c "sleep 150; echo 0x10 > $G; echo 1 > $S" > /dev/null 2>&1 &
echo "$(ts) deadman armed (150 s)"
T0=$(sh /tmp/opencard.sh 14)
echo "$(ts) card opened, T0=$T0"
wait_until $T0
for pair in "3 STRONG" "24 SOFT" "45 OFF"; do
  set -- $pair; base=$1; other=$2
  k=0
  while [ $k -lt 6 ]; do
    wait_until $((T0 + base + 3 * k))
    if [ $((k % 2)) -eq 0 ]; then set_state NORMAL; else set_state $other; fi
    k=$((k + 1))
  done
done
wait_until $((T0 + 63))
set_state NORMAL
sleep 2
luna-send -n 1 -f $A/closeByAppId '{"id":"com.webos.app.browser"}' > /dev/null 2>&1
sleep 1
luna-send -n 1 -f $A/launch '{"id":"com.webos.app.home"}' > /dev/null 2>&1
rm -f /tmp/card.b64
echo "$(ts) restored: $(cur)"; luna-send -n 1 -f $A/getForegroundAppInfo '{}' | grep appId
