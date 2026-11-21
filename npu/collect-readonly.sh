#!/bin/sh
# Run only in an agreed idle TV slot. No device opens, Luna calls or TV writes.
set -eu
mode=${1:---inventory}
case "$mode" in --inventory|--archive) ;; *) echo 'usage: collect-readonly.sh [--inventory|--archive]' >&2; exit 2 ;; esac
set --
for path in \
    /usr/share/aibsp /usr/share/aif /usr/share/aiservice \
    /usr/lib/libaibsp.so.1.0.0 /usr/lib/libaibspLib.so.1.0.0 \
    /usr/lib/libnpu.so.1.0.0 /usr/lib/libnnrt.so.1.0.0 \
    /usr/lib/libwebos_offline_nnhal.so.1.0.0 /usr/lib/libwebos_nnhal.so \
    /usr/lib/libtflite-npu-delegate.so.1.0.0 \
    /usr/lib/libai-inference-interface.so.1.0 /usr/lib/libedgeai-vision.so.1.1.0 \
    /usr/lib/edgeai-extensions /usr/lib/libOpenCL.so.1.2 /usr/lib/libOpenCLDriverStub.so \
    /usr/bin/run_lgnpu /usr/bin/lgnpu_nn_hal_test /usr/bin/lgnpu_online_nnhal_test \
    /usr/bin/nn-proxy /usr/bin/ai-inference-manager /usr/bin/airunner \
    /usr/share/luna-service2/api-permissions.d/com.webos.service.aiinferencemanager.api.json \
    /usr/share/luna-service2/roles.d/com.webos.service.aiinferencemanager.role.json \
    /usr/share/luna-service2/roles.d/com.webos.service.nnproxy.role.json \
    /usr/share/luna-service2/client-permissions.d/com.webos.service.nnproxy.perm.json \
    /usr/lib/opkg/info/kernel-module-npu-drv-*.list \
    /media/system/apps/usr/palm/applications/com.webos.aimodel-*/model \
    /media/cryptofs/apps/usr/palm/applications/com.webos.aimodel-*/model
 do
    if [ -e "$path" ]; then set -- "$@" "$path"; fi
 done
if [ "$mode" = --archive ]; then
    [ "$#" -gt 0 ] || exit 1
    exec tar -cf - "$@"
fi
printf '%s\n' '=== Existing files: sizes and SHA256 ==='
for path do
    if [ -d "$path" ]; then
        find "$path" -type f -exec wc -c {} \; -exec sha256sum {} \;
    else
        wc -c "$path"
        sha256sum "$path"
    fi
done
printf '%s\n' '=== Device state (read-only) ==='
for path in /proc/npu/npu_device_capability /proc/npu/npu_utilization /proc/npu/npu_process_list; do
    if [ -r "$path" ]; then printf '\n%s\n' "$path"; cat "$path"; fi
done
printf '%s\n' '=== NPU device/module identity ==='
ls -l /dev/npu0
while IFS= read -r line; do
    case "$line" in npu_drv\ *) printf '%s\n' "$line" ;; esac
done < /proc/modules
printf '%s\n' '=== NPU descriptors currently held ==='
for path in /proc/[0-9]*/fd/*; do
    target=$(readlink "$path" 2>/dev/null) || continue
    case "$target" in */dev/npu0) printf '%s -> %s\n' "$path" "$target" ;; esac
done
printf '%s\n' '=== Additional model-cache names (not copied automatically) ==='
for path in /mnt/lg/flash/data/_liblgnpu /mnt/lg/cmn_data/edgeai /usr/lib/firmware /lib/firmware; do
    if [ -d "$path" ]; then
        find "$path" -type f \( -name '*.lgnpu' -o -name '*DNE*' -o -name '*dne*' -o -name '*GNMNET*' -o -name '*SGNET*' -o -name '*npu*' -o -name '*edgeai*' \) -print
    fi
done
