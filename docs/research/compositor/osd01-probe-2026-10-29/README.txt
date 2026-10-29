The OSD0_1 probe of docs/research/scanout-research.md (§ The probe), run with tools/osd01-probe.sh on the C4, 2026-10-29
06:41-06:46 UTC, LG's compositor live, Home on the panel. Stage 2 started 06:45:47 UTC; the layer was on from 06:45:49 to 06:45:55.

probe.log           the script's log: stage 0 and stage 1 (clock screensaver on the panel), stage 0 again with Home showing, stage 2
baseline.txt        the registers stage 0 read (the second run); revert.sh is the file the dead-man ran, revert.log its two firings
                    (06:45:54 the script's own revert, 06:45:59 the dead-man, 12 s after it was armed)
regdump-*.txt       /proc/lg/fbdev/regdump before, with the layer on, after; regdump-*.diff the registers that differ (address, first, second)
before.png ...      LG's capture service (DISPLAY, 960x540): stage 0, just before the writes (s2-pre), layer on at once (s2-on) and 2.5 s
                    on (s2-on2), after the revert (s2-after), after the toast (s2-toast)
noise-dry-run/      two regdumps 0.4 s apart with nothing written: the registers that change by themselves
run1-saver/         stage 0 with the screensaver up (before.png is the saver)
