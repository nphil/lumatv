# gfx64: 64-bit Mali path (heap shim, launcher, blob bundle)

## Resume here

State (2026-10-25, run 1): started. Nothing committed yet. TV not touched (it is busy; wait for "TV is yours").
Plan, in order: gfx64/heapcore.h (fallback logic, ops table) -> gfx64/heapshim.c (interposer) -> gfx64/test_heapcore.c
(mock ioctl) -> gfx64/fetch-blob.sh -> gfx64/launch.sh + test_launch.sh -> gfx64/build.sh -> gfx64/smoke-tv.sh ->
gfx64/README.md. Next command to run: `ls gfx64/` to see which of those exist, continue with the first missing one.
Pins: Rockchip blob JeffyCN/mirrors branch libmali commit 077bc8f582c5da0cb80136a711ce3be2fa1a04cd
(g24p0-10), sha256 15f94e34138ab315daab8d90b2bd17b792dc32d0b2206c6cc2efc670eff5f93d; a local copy of everything is
in /data/home/tmp/m64dl/ (blob, the Debian debs, deploy/lib = tested layout).
