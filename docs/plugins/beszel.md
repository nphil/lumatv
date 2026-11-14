# Beszel: the TV in a Beszel hub

The Beszel plugin (`beszel`, off until switched on in Settings › System › Beszel) makes the TV a system of a Beszel hub. Code:
`helper/app/src/beszel.rs`; page `rustui/ui/src/settings/pages/beszel.rs`.

- **Agent:** `beszel-native` (`helper/beszel-native`), a Rust agent that speaks the hub protocol of Beszel 0.21.1 (`UPSTREAM` in
  `src/system.rs`), built static for aarch64 musl by `helper/build.sh`. It ships in the payload beside Beszel's own `beszel-agent`
  (the linux arm64 release, pinned by version and hash in `beszel/fetch-agent.sh`, 0.21.1), and `lumatv` stages both into
  `bin/beszel` beside the helper. While the plugin is on the helper runs `beszel-native` as uid/gid 19999, confined as plugin
  programs are; off, it is stopped. It dials the hub over its WebSocket (`HUB_URL`, `TOKEN`, `KEY`) and listens on no port. An agent
  that ends is started again after 2 s, doubling to 60 s. Measured on the TV in Active Standby over 180 s (2026-11-13, hub 0.21.2):
  `beszel-native` 0.01 s of CPU (0.006% of one core, one clock tick) and 1.1 MB resident; Beszel's agent 0.05 s (0.03%) and 11.9 MB.
- **Fallback:** `beszel-native` exits with 76 when the hub's protocol has moved past it: a request, or the payload of a data or
  fingerprint request, that does not decode, or a hub that refuses its version when it connects. A wrong key or token and network
  failures never do. On 76 the helper runs `beszel-agent` with the same settings until the plugin is switched off and on again or
  the helper restarts, and the status ends "(official agent: the hub is newer than the built-in agent)".
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
| GPU | Mali (`Mali-G52 MC3` style name from `/sys/class/misc/mali0/device/gpuinfo`): load is busy time over busy+idle between two 10 s looks of debugfs `mali0/dvfs_utilization`; memory is the pages held by GPU contexts (`mali0/gpu_memory`) out of the TV's RAM, which the GPU shares. Debugfs is root's, so the helper writes the sample to `/run/lumatv-beszel/gpu` as one `nvtop -lP` line: `beszel-native` reads it (`GPU_SAMPLE_FILE`), and the payload's `nvtop` shim hands it to Beszel's agent (`GPU_COLLECTOR=nvtop`) |

Not shown, because the TV does not expose them: GPU clock and temperature, CPU frequency, fans, power draw, battery. Containers
and systemd services are off (`SKIP_SYSTEMD`); the TV has neither in a form the agent reads.

## The built-in agent

`beszel-native` sends what Beszel's agent sends for the TV, with the same field keys, so the hub cannot tell them apart, and logs
the same lines, which the helper's status reads. It differs where the TV has no use for a feature:

- Wi-Fi is the signal level only (`/proc/net/wireless`), read with each 60 s request.
- Not ported: SMART and eMMC health, containers, systemd services, ZFS, fans, batteries, cgroup limits (`USE_CGROUP`),
  `DISK_USAGE_CACHE`, `SENSORS_TIMEOUT`, `SMART_INTERVAL`, `[...]` classes in name patterns, the SSH server and `LISTEN`, proxies
  (`BESZEL_AGENT_ALL_PROXY`) and the system's CA store (TLS trusts the webpki roots and `CA_CERT_FILE`).
- Requests for containers, SMART data and network monitors get the answer Beszel's agent gives without the feature (empty, and
  only an empty set of monitors is accepted); systemd requests and unknown actions get an error reply.

Against a 0.21.2 hub on a PC (`hub-test.sh` with `HUB_TEST_MEASURE=180`, 181 live requests) it used 0.14 s of CPU and 2.2 MB
resident, against 0.41 s and 13.2 MB for Beszel's agent. Beyond what Beszel's agent sends from the TV, the hub gets the eMMC's read
and write rates (`dio`, `dios`).

## Following the hub

- `helper/beszel-native/upstream.sh <tag>` prints the diff, between `UPSTREAM` and the tag, of the files that define the protocol
  (`internal/common/common-ws.go`, `internal/entities/system/system.go`, `internal/hub/agent_connect.go`, `agent/client.go`,
  `agent/handlers.go`); it exits 1 when they changed. 0.21.2 changes none of the wire format.
- `helper/beszel-native/hub-test.sh [version]` runs the host build (`helper/build.sh host`) against a real hub of that version
  (default `UPSTREAM`; hub cached in `~/.cache/beszel-hub`) with a fake SoC sensor and GPU sample, and passes when the system is up
  and a stats record carries CPU, memory, disk, temperature and GPU. `HUB_TEST_AGENT=official` runs Beszel's agent instead;
  `HUB_TEST_MEASURE=<seconds>` adds the agent's CPU time and memory over that long of the hub's live view (a request a second).
- A new hub version: run both scripts; port what changed, then move `UPSTREAM` and `beszel/fetch-agent.sh` together.
