# Home Assistant device: Bluetooth proxy and TV sensors

The TV is one ESPHome device to Home Assistant ("LumaTV Bluetooth", `lumatv-ble`, port 6053, Noise-encrypted with the key from
`lumatv-helper ble-link`). Two of the helper's own plugins serve it, each off until the owner switches it on in Settings › Inputs &
devices › Home Assistant device, which asks first in the plugin's own words. The device exists only while the link file is there and one
of the two is on.

- **Bluetooth proxy** (`ble-proxy`, `helper/app/src/ble.rs`): a passive scanner for Home Assistant's Bluetooth integration, in windows
  of the link's length (10 s a minute by default); `ble-link --standby` lets it scan in Always Ready. A TV linked before the proxy was a
  plugin comes up with it on, once (`<data dir>/ble/plugin-migrated` marks that).
- **TV sensors** (`tv-sensors`, `helper/app/src/tvsensors.rs`): read-only entities of the same device.

| Entity | Type | Source | Sent |
|---|---|---|---|
| SoC temperature (°C) | sensor | `/proc/lg/pm/temperature` | every 30 s (screen on) or 60 s (off), when it changed |
| Power state | text | `screen` topic (`tvpower`) | at its change |
| Screen | binary | `screen` topic | at its change |
| Foreground app | text | `getForegroundAppInfo` subscription | at its change |
| Input | text | the app in front; an input's name from `eim/getAllInputStatus` | at its change |
| Picture mode | text | `settingsservice` subscription (`picture.pictureMode`) | at its change |
| CPU load | sensor (diagnostic) | `/proc/loadavg`, one minute | every 30/60 s, when it changed |
| Free memory (MB) | sensor (diagnostic) | `/proc/meminfo` `MemAvailable` | every 30/60 s, when it changed |
| Live TV Tuner | text | the tuner's status (Off, Idle, Streaming 2.4 WEST, Busy) | at its change |
| Tuner signal (%) | sensor | the demodulator's report while a channel streams, unknown otherwise | every 30/60 s, when it changed |

Nothing is read while Home Assistant has no connection subscribed to the device's states. Switching a plugin on or off ends Home
Assistant's connection, which reconnects and reads the device anew. There is no ambient light sensor: the TV has no readable lux value.
