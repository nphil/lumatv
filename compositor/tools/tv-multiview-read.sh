#!/bin/sh
# Read-only probes for "how does LG's Multi View give a second video a pipeline of its own" (docs/research/multiview-result.md), run ON THE SET
# (luna-send needs a PTY: the round's `T`, or ssh -t). Nothing is changed: no setting, no service, no file outside $OUT (default /tmp/lumatv/mvread).
# The binaries are only read (strings of them are kept, never the binaries), nothing under /usr or the root filesystem is written.
#
#   mvread.sh config            LG's media server files: /etc/umediaserver listed and every text file in it (the resource tables, the codec file,
#                               the pipeline list), and the Multi View settings the media stack is configured with
#   mvread.sh strings           the words of the media stack's programs and libraries that name a sub window, a multi view, a scaler, a policy
#                               level, a display path (one file per binary, sorted, at most 700 lines each)
#   mvread.sh luna LABEL        the read-only calls (the media server's pipelines and the resources each holds, videooutputd's sinks, SAM's foreground
#                               list, the Multi View controller's state), then the messages log's lines of the media stack (the log rotates about
#                               every minute: this copies it at once)
#   mvread.sh all LABEL         config, strings and luna
#
# Where it matters (docs/research/multiview-result.md, "TV probes"): `luna` is run with LG's compositor in front and
#   A. YouTube playing, nothing else                  (LABEL yt)
#   B. LG's own Multi View, YouTube and a second source (LABEL lgmv; Nitin starts it from the Multi View tile of the quick panel or the Multi View
#      app, with LG's compositor, not LumaTV's: `cutover disable` first)
#   C. YouTube playing and the camera feeder started on it   (LABEL yt-feeder; the feeder with `--ndl-app` the foreground app's id)
OUT=${LUMATV_MVREAD:-/tmp/lumatv/mvread}
LUNA_TIMEOUT=${LUMATV_MVREAD_LUNA_TIMEOUT:-10}
mode=${1:-help}
label=${2:-run}
mkdir -p "$OUT"

say() { printf '%s\n' "$*"; }

cfg() {
    d=$OUT/config
    mkdir -p "$d"
    for dir in /etc/umediaserver /etc/umediaserver/external /var/lib/ums/conf /etc/multiviewcontroller /etc/mvc /usr/share/multiviewcontroller; do
        [ -d "$dir" ] || continue
        say "== $dir"
        ls -l "$dir"
    done > "$d/listing.txt" 2>&1
    n=0
    for dir in /etc/umediaserver /etc/umediaserver/external /var/lib/ums/conf /etc/multiviewcontroller /etc/mvc /usr/share/multiviewcontroller; do
        [ -d "$dir" ] || continue
        for f in "$dir"/*; do
            [ -f "$f" ] || continue
            # text only: a binary has a NUL in its first kilobyte
            if [ "$(head -c 1024 "$f" | tr -cd '\000' | wc -c)" = 0 ]; then
                out=$d/$(echo "$f" | tr '/' '_')
                head -c 60000 "$f" > "$out"
                n=$((n + 1))
            fi
        done
    done
    say "config: $n text files in $d, listing in $d/listing.txt"
    # the media stack's own idea of Multi View, from the settings service (read-only getters)
    timeout "$LUNA_TIMEOUT" luna-send -n 1 luna://com.webos.service.config/getConfigs '{"configNames":["multiview.supportMultiview","multiview.supportMultiScreen","tv.model.supportHDR","tv.hw.SoCOutputFrameRate"]}' > "$d/configs.json" 2>&1
    say "config: $d/configs.json"
}

# $1 name, finds a program or library of that name under the usual directories
locate_bin() {
    for dir in /usr/sbin /usr/bin /usr/lib /usr/lib/gstreamer-1.0 /usr/libexec /lib /sbin /usr/local/lib /usr/lib32; do
        for f in "$dir"/$1; do
            [ -f "$f" ] && { echo "$f"; return; }
        done
    done
}

# the printable strings of $1 (the programs may have no `strings`: busybox's grep -a does the same)
words() {
    if command -v strings > /dev/null 2>&1; then strings -a -n 5 "$1"; else grep -a -o -E '[[:print:]]{5,}' "$1"; fi
}

STR_RE='sub[_ -]?(window|scaler|sink|app|screen)|multi[_ -]?(view|screen)|mvc|scaler|primary|policy|level|display[_ -]?path|windowid|game|lowdelay|low_delay|vtg|gpsc|texture|occup|sinktype|sink_type|foreground|is_foreground|hfr|pip\b|SUB0|MAIN_|SUB_|multiview_sub'

strs() {
    d=$OUT/strings
    mkdir -p "$d"
    {
        for name in umediaserver starfish-media-pipeline multiviewcontroller videooutputd advr-play-pipeline starfish-camera-pipeline \
                    'libmedia-resource-calculator.so*' 'libplayerAPIs.so*' 'libNDL_directmedia.so*' 'libAcbAPI.so*' 'libumedia*' 'libums*' 'libpf.so*' \
                    'libstarfish*' 'libvideooutput*' 'libvtg*' 'libgstlxvideosink.so*' 'libgstdecproxy.so*' 'libgstlgaudiosink.so*'; do
            f=$(locate_bin "$name")
            [ -n "$f" ] || { say "strings: $name: not found"; continue; }
            out=$d/$(basename "$f").txt
            words "$f" | grep -i -E "$STR_RE" | sort -u | head -n 700 > "$out" 2>/dev/null
            say "strings: $f ($(wc -c < "$f") bytes): $(wc -l < "$out") lines in $out"
        done
    }
}

call() {
    # $1 uri, $2 payload, $3 file
    payload=$2
    [ -n "$payload" ] || payload='{}'
    timeout "$LUNA_TIMEOUT" luna-send -n 1 "$1" "$payload" > "$3" 2>&1
    say "luna: $1 -> $3 ($(wc -c < "$3") bytes)"
}

lun() {
    d=$OUT/luna-$label
    mkdir -p "$d"
    call luna://com.webos.media/getActivePipelines '{}' "$d/ums-pipelines.json"
    call luna://com.webos.service.videooutput/getStatus '{}' "$d/videooutput-status.json"
    call luna://com.webos.applicationManager/getForegroundAppInfo '{}' "$d/sam-foreground.json"
    call luna://com.webos.surfacemanager/getForegroundAppInfo '{}' "$d/lsm-foreground.json"
    call luna://com.webos.surfacemanager/getForegroundVideoWindow '{}' "$d/lsm-video-window.json"
    call luna://com.webos.service.multiviewcontroller/getStatus '{}' "$d/mvc-status.json"
    call luna://com.webos.service.multiviewcontroller/getMultiviewStatus '{}' "$d/mvc-multiview-status.json"
    call luna://com.webos.service.multiviewcontroller/getMediaControlStatus '{}' "$d/mvc-media-control.json"
    call luna://com.webos.applicationManager/running '{}' "$d/sam-running.json"
    # the log rotates about every minute: the file and what it rotated to, at once
    mkdir -p "$d/messages"
    cp /var/log/messages* "$d/messages/" 2>/dev/null
    cat "$d"/messages/messages.* "$d"/messages/messages 2>/dev/null |
        grep -E 'ums\.|umediaserver|multiviewcontroller|MVC |videooutputd|starfish|policy action|SUB0|SUB_SCALER|MAIN_SCALER|VDEC|ResourceManager|getForegroundApp' |
        tail -n 600 > "$d/journal-media.txt"
    say "luna: $(wc -l < "$d/journal-media.txt") journal lines in $d/journal-media.txt"
}

case "$mode" in
config) cfg ;;
strings) strs ;;
luna) lun ;;
all) cfg; strs; lun ;;
*) sed -n '2,23p' "$0" ;;
esac
