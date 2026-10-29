cd /tmp/lumatv
ctl() { ./lib/ld-linux-aarch64.so.1 --library-path lib bin/lumatv-compositor ctl --runtime-dir /tmp/xdg "$@" 2>/dev/null; }
fb() { sed -n "s/^ *$1 *[:=] *//p" /proc/lg/fbdev/fb0/status | head -1; }
echo "before: mem=$(fb hdr.w_mem)x$(fb hdr.h_mem) out=$(fb hdr.w_out)x$(fb hdr.h_out) flips=$(fb status.fb_flip_cnt)"
dmesg | wc -l > /tmp/hw-dm0
echo "start: $(ctl pageprobe $1 12 | cut -c1-200)"
sleep 3
echo "during: mem=$(fb hdr.w_mem)x$(fb hdr.h_mem) out=$(fb hdr.w_out)x$(fb hdr.h_out) flips=$(fb status.fb_flip_cnt) visible=$(fb visible)"
ctl pageprobe | cut -c1-300
dmesg | grep -E "fbdev|drm" | tail -3 | cut -c1-150
sleep 11
echo "after: $(ctl pageprobe) mem=$(fb hdr.w_mem)x$(fb hdr.h_mem) out=$(fb hdr.w_out)x$(fb hdr.h_out) flips=$(fb status.fb_flip_cnt)"
