# C4 hardware census for LumaTV

## Resume here

Live-set collection pending the shared TV slot. Static source analysis is in progress. This file is deliberately updated as evidence arrives.

## Scope and evidence rules

Target: OLED65C4PUA, webOS 10.2.1, kernel 5.4.268, LG O22/O22N2 platform (O22A3), four Cortex-A76 cores at 1.4 GHz, 2 GB RAM, Mali-G52 MC3, 64-bit kernel and 32-bit webOS userspace. `TV:` means measured on this set; `SRC:` is LG's published O22 source (webOS 25 source is a close but newer reference); `PLAN:` is a prior measured LumaTV finding; `[INFERENCE]` needs the listed TV probe.

## Evidence still to integrate

- NPU/DNE source and live library/model census.
- Video, image, 2D, PQ, audio, security and board slices.
- Two 10-second live counter windows: idle and Home.

