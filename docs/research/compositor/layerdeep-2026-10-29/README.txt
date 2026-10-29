Read-only observation of the C4's display state, 2026-10-29 07:45-07:46 UTC (03:46 EDT), LG's compositor live (surface-manager was
the DRM master; LumaTV's compositor was not running), the screen still. Collected by the LayerDeep worker over docs/research/arch/tv.py
with the commands of docs/research/display-chip-direct.md "TV checks"; read and analysed in docs/research/layer-deep-result.md § 1.
Nothing was written to any register or file on the set.

kernel.txt, vermagic.txt   /proc/version, modules_disabled, /proc/modules, /lib/modules, the vermagic and signature fields of the installed .ko
tracing.txt                tracefs: kprobe_events present and empty, the event groups, no function tracer
kallsyms.txt               the OSD, DRM and DE symbols of interest (osd_drm_flip_image, FBDEV_O22_CMN_WriteHdrRegs, ...)
params.txt                 /sys/module/lg1k and lg1kdrm parameters
cmdline.txt                the kernel command line (serial number and MAC redacted)
chip-diag.txt, osdinfo.txt, fb0..3-status.txt, fbdev-ls.txt, fbdev-knobs.txt, sysfs-fb.txt   /proc/lg/fbdev and /sys/class/graphics
regdump-1..3.txt           /proc/lg/fbdev/regdump three times a second apart (regdump-times.txt: the fb0 flip count at each)
dmesg-osd.txt              the orbit (vsc_set_orbit_move) and OSD lines of dmesg
drmplanes.txt, dri-*.txt   the DRM planes, formats and modifiers (docs/research/arch/drmplanes.c), debugfs state and clients
de.txt, logm.txt           /proc/lg/de status and /proc/logm
holders.txt, nodes.txt     who holds which device node, and the nodes' modes
dmabuf.txt, meminfo.txt, date.txt
