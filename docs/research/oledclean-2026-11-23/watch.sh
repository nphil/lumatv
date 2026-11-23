#!/bin/sh
# Read-only OLED cleaning watch (docs/research/oled-cleaning-signal.md, TV test). Waits for the owner to put the set in standby,
# then samples the refresher counter and pnwash state every 5 s for up to 15 min or until offrsCount rises. Never wakes the set.
R=$(cd "$(dirname "$0")" && pwd)
TV=192.168.1.242
export TV PLX_TV_LOCK_AGENT=Main
L=/data/home/Marquee/tools/tv-lock.sh
tv() { ssh -tt -o BatchMode=yes -o ConnectTimeout=8 root@$TV "$@" 2>&1 | tr -d '\r' | grep -v '^Connection to'; }
deadline=$(date -d '06:00' +%s)
state() { tv "luna-send -n 1 luna://com.webos.service.tvpower/power/getPowerState '{}'" | grep -o '"state": *"[^"]*"' | cut -d'"' -f4; }

echo "$(date +%T) waiting for standby" >> "$R/log.txt"
while :; do
    s=$(state)
    echo "$(date +%T) power=$s" >> "$R/log.txt"
    case "$s" in Active|"") ;; *) break ;; esac
    if [ "$(date +%s)" -ge "$deadline" ]; then echo "$(date +%T) gave up: still on at 06:00" >> "$R/log.txt"; exit 0; fi
    $L renew >/dev/null 2>&1
    sleep 10
done
t0=$(date +%s)
echo "$(date +%T) standby seen ($s) t0=$t0" >> "$R/log.txt"
$L renew >/dev/null 2>&1

# passive captures, all ended below
tv "nohup sh -c 'ls-monitor -j -e > /tmp/oc-standby.jsonl 2>&1' >/dev/null 2>&1 &
nohup sh -c 'luna-send -n 100 -f luna://com.webos.service.oledepl/getPixelRefresherInfoList \"{\\\"subscribe\\\":true}\" | while IFS= read -r l; do echo \"\$(date +%s) \$l\"; done > /tmp/oc-sub.txt 2>&1' >/dev/null 2>&1 &
nohup sh -c 'tail -F /var/log/messages | grep --line-buffered -i -E \"eplmanager|pnwash|offrs|compens|oslb|tvpowerd\" > /tmp/oc-log.txt 2>&1' >/dev/null 2>&1 &
echo started"
base=""
end=$((t0 + 900))
while [ "$(date +%s)" -lt "$end" ]; do
    line=$(tv 'echo "$(date +%s) $(luna-send -n 1 luna://com.webos.service.tvpower/power/getPowerState "{}" | grep -o "\"state\": *\"[^\"]*\"" | cut -d\" -f4) $(luna-send -n 1 luna://com.webos.service.oledepl/getPixelRefresherInfoList "{}" | tr -d " \n" | grep -o "\"offrsCount\":[0-9]*\|\"offrsLastTime\":[0-9]*" | tr "\n" " ") $(luna-send -n 1 luna://com.webos.service.panelcontroller/getPanelUsageTime "{\"subscribe\":false}" | tr -d " \n" | grep -o "\"panelUsageTime\":[0-9]*") state=$(cat /mnt/lg/cmn_data/pnwash/state 2>/dev/null | tr "\n" " ")"')
    echo "$line" >> "$R/samples.txt"
    c=$(echo "$line" | grep -o '"offrsCount":[0-9]*' | cut -d: -f2)
    [ -z "$base" ] && base=$c
    if [ -n "$c" ] && [ -n "$base" ] && [ "$c" -gt "$base" ]; then echo "$(date +%T) offrsCount rose $base -> $c" >> "$R/log.txt"; sleep 30; break; fi
    # the power state is the second word; "Active Standby" is standby, only a bare "Active" is the set on
    if echo "$line" | awk '{exit !($2 == "Active" && $3 != "Standby")}'; then echo "$(date +%T) set woke (owner): stopping" >> "$R/log.txt"; break; fi
    sleep 5
done
echo "$(date +%T) sampling ended" >> "$R/log.txt"
tv 'pkill -f "ls-monitor -j -e"; pkill -f "getPixelRefresherInfoList"; pkill -f "tail -F /var/log/messages"; sleep 1; true' >/dev/null
for f in oc-standby.jsonl oc-sub.txt oc-log.txt; do scp -q -o BatchMode=yes root@$TV:/tmp/$f "$R/$f" 2>/dev/null; done
tv 'rm -f /tmp/oc-standby.jsonl /tmp/oc-sub.txt /tmp/oc-log.txt' >/dev/null
tv 'date +%s; luna-send -n 1 -f luna://com.webos.service.oledepl/getPixelRefresherInfoList "{}"; luna-send -n 1 -f luna://com.webos.service.panelcontroller/getPanelUsageTime "{\"subscribe\":false}"; ls -l --full-time /mnt/lg/cmn_data/pnwash/; for f in state completedOffRsCount autoOffRsLastTime failAlertCount completedJbCount autoJbLastTime; do echo "$f=$(cat /mnt/lg/cmn_data/pnwash/$f 2>&1)"; done; ls -l --full-time /mnt/lg/cmn_data/var/spool/rdxd/ 2>&1 | grep -i pnwash; luna-send -n 1 -f luna://com.webos.service.micomservice/listPowerOnOffHistory "{\"maxCount\":10,\"startIndex\":0}"' > "$R/after.txt"
$L release >/dev/null 2>&1
echo "$(date +%T) done, lock released" >> "$R/log.txt"
