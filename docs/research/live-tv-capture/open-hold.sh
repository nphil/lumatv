#!/bin/sh
# usage: open-hold.sh SECONDS  - opens a SCAN pipeline from a luna-send that stays connected for SECONDS, prints its reply
rm -f /tmp/g1.open
payload='{"options":{"appId":"com.webos.app.channelsetting","windowId":"","purpose":"SCAN"}}'
nohup sh -c "sleep $1 | luna-send -i -f luna://com.webos.service.utp.broadcast/open '$payload'" > /tmp/g1.open 2>&1 < /dev/null &
sleep 3
cat /tmp/g1.open
