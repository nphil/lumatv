#!/bin/sh
cd /tmp/r20-build
K=home,osd,toast,alert,quick,picker,drawer,settings,entry,keyboard,menu,confirm,switcher,saver,displaylab
sh run20.sh apps-on --only apps --apps-repeat 15
python3 /tmp/lt/tv.py "printf '\n' > /var/lib/lumatv/compositor/shell-rust.conf" 20
sh run20.sh apps-off --only apps --apps-repeat 15
python3 /tmp/lt/tv.py "printf '%s\n' $K > /var/lib/lumatv/compositor/shell-rust.conf; cat /var/lib/lumatv/compositor/shell-rust.conf" 20 >> /tmp/r20/gate/chain.log
sh run20.sh frames60q --only frames --frames-quick
sh run20.sh frames120q --only frames --frames-quick --refresh 120
echo "$(date +%T) gates done" >> /tmp/r20/gate/chain.log
