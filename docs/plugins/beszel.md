# Beszel: the TV in a Beszel hub

The Beszel plugin (`beszel`, off until switched on in Settings › System › Beszel) makes the TV a system of a Beszel hub. Code:
`helper/app/src/beszel.rs`; page `rustui/ui/src/settings/pages/beszel.rs`.

- **Agent:** Beszel's own `beszel-agent` (the linux arm64 release, pinned by version and hash in `beszel/fetch-agent.sh`, 0.21.1 to
  match the hub) is shipped in the payload and staged by `lumatv` into `bin/beszel` beside the helper. While the plugin is on the
  helper runs it as uid/gid 19999, confined as plugin programs are; off, it is stopped. It dials the hub over its WebSocket
  (`HUB_URL`, `TOKEN`, `KEY`) and listens on no port (`DISABLE_SSH`). An agent that ends is started again after 2 s, doubling to 60 s.
  Measured on the TV in Active Standby over 180 s: 0.03% of one core, 11-13 MB resident.
- **Page:** hub address, hub key and token, as the hub's Add System shows them. The token is a secret field, kept in `beszel/token`
  (0600) and passed only to the agent. Status reads the agent's log: connected, connecting, or the reason it is not.
- **Fingerprint:** the agent keeps its identity in `beszel/data/fingerprint`; the hub binds the system to it on first connection.
  Deleting it makes the hub refuse the TV until the system's fingerprint is reset in the hub.

## What the hub shows

| Beszel | Source on the TV |
|---|---|
| CPU, load, memory, swap | `/proc`, as the agent reads it on any Linux |
| Disk | `/media` as "Apps" (`FILESYSTEM`) and `/var` as "Data" (`EXTRA_FILESYSTEMS`); `/` is the read-only system image |
| Network | `eth0` and `wlan0` (`NICS`) |
| Temperature `soc` | `/proc/lg/pm/temperature` (whole degrees), laid out by the helper as a hwmon sensor under `/run/lumatv-beszel/sys` every 10 s (`SYS_SENSORS`); the TV's only temperature |
| GPU | Mali (`Mali-G52 MC3` style name from `/sys/class/misc/mali0/device/gpuinfo`): load is busy time over busy+idle between two 10 s looks of debugfs `mali0/dvfs_utilization`; memory is the pages held by GPU contexts (`mali0/gpu_memory`) out of the TV's RAM, which the GPU shares. Debugfs is root's, so the helper writes the sample and the payload's `nvtop` shim hands it to the agent (`GPU_COLLECTOR=nvtop`) |

Not shown, because the TV does not expose them: GPU clock and temperature, CPU frequency, fans, power draw, battery. Containers
and systemd services are off (`SKIP_SYSTEMD`); the TV has neither in a form the agent reads.
