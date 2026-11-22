#!/bin/sh
# Transport and lease are external. No temporary files or display control calls.
set -eu
kernel=5.4.268-329.ptl4tv.2
if [ "$#" -ne 0 ] || [ "$(uname -r)" != "$kernel" ]; then
    echo 'STOP: not the qualified kernel or unexpected arguments' >&2
    exit 1
fi
section() { printf '\n@@VRR %s\n' "$1"; }
section boot
cat /proc/sys/kernel/random/boot_id
section image
sha256sum "/boot/Image.lz4-$kernel"
section cmdline
cat /proc/cmdline
section overrides
cd -P /var
if [ -L firmware ] || [ -e firmware ]; then
    cd -P firmware
    for i in 0 1 2; do
        p=pqefw$i.bin
        if [ -e "$p" ] || [ -L "$p" ]; then echo "STOP: override $p" >&2; exit 1; fi
    done
fi
printf 'none\n'
section status
cat /proc/lg/de/fw-status
section foreground
timeout 5 luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}'
section loader
# Preserve order and explicit read failure, without exporting unrelated kernel messages.
{ dmesg && printf '\nVRR_DMESG_READ_OK\n'; } | awk '/load\(from:|ioload fw|de firmware is not working|backup open\(|fw_opt skip|rpm \(from:/{print} /^VRR_DMESG_READ_OK$/{ok=1} END{if (!ok) exit 1}'
section end
cat /proc/sys/kernel/random/boot_id
