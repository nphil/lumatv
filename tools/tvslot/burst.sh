# burst.sh DIR NAME APPID MS: launch APPID, then capture NAME-NN.png (960x540) back to back for MS ms from the launch call's return.
# DIR/times has "launch-call-ms N" and then "NN ms-since-the-return" per frame; DIR/md5.txt the frames' md5; DIR/done when finished.
# The loop runs on the set so the capture cadence is not the SSH round trip's (about 0.2 s a frame).
D=$1; N=$2; A=$3; MS=${4:-12000}
rm -rf "$D"; mkdir -p "$D"
S=$(cut -d' ' -f1 /proc/uptime | tr -d .)
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"'$A'"}' >/dev/null
L=$(cut -d' ' -f1 /proc/uptime | tr -d .)
echo launch-call-ms $(( (L-S)*10 )) > "$D/times"
i=1
while [ $(( $(cut -d' ' -f1 /proc/uptime | tr -d .) - L )) -lt $(( MS / 10 )) ]; do
 n=$(printf %02d $i)
 luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{"path":"'$D'/'$N'-'$n'.png","method":"DISPLAY","format":"PNG","width":960,"height":540}' >/dev/null
 echo $n $(( ($(cut -d' ' -f1 /proc/uptime | tr -d .) - L) * 10 )) >> "$D/times"
 i=$((i+1))
done
cd "$D" && md5sum ./*.png > md5.txt
echo done > "$D/done"
