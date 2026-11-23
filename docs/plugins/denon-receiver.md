# Denon receiver: power and sound output

The Denon receiver plugin (`denon-receiver`, off until switched on in Settings) keeps the receiver with the TV while the TV's sound
output is the receiver (`external_arc` or `tv_speaker_external_arc`); a TV set to another output is left alone. Code:
`helper/app/src/receiver.rs` (`sync_power`, `Keeper`), `receiver_sync.rs` (`Intent`).

- **Sound output:** remembered in `receiver-output.json` in the helper's data directory, not read from LG's saved
  `sound.soundOutput` alone. LG saves `tv_speaker` itself about 1 s after the receiver leaves ARC (switched off, or off at boot after a
  power cut), so a `tv_speaker` saved while the receiver is not on keeps the receiver; one saved after it has read on for
  `receiver.hideAfterMs` (6 s) is the owner's choice and releases it. Any other output releases it.
- **Wake:** the screen coming on, or the helper starting with it on (a reboot, a power cut), sends `ZMON` to a receiver not read as on;
  standby sends `ZMOFF` as a backup to CEC when the receiver is still on. LG's own CEC wakes the receiver only when its last output was
  ARC (`Last ARC(1)` in the TV's log).
- **Screen on:** from `receiver.powerOnMs` (5 s) after a power command, a receiver read as off is sent `ZMON` again, backing off to
  `receiver.maxBackoffMs` (30 s) for as long as the screen is on, and set to the TV input (`SITV`) once it reads on after that
  command. A receiver switched off by hand while the screen is on gets one `ZMON`; switched off again, it is left off until the next
  wake.
- **Fallback:** a receiver that two reads in a row cannot reach, or that stays off after a power command, moves the TV's live route to its
  own speakers (`com.webos.service.audio/soundSettings/setSoundOut` `tv_speaker`), which leaves the saved output as it is. An
  unreachable receiver is read every 30 s at most; one that answers again off is turned on, and once it reads on the route goes back to
  the remembered output.
- **Standby:** nothing powers the receiver on while the screen is off.

The helper log says at each wake what the receiver read and what was sent (`receiver: the screen came on; the receiver reads standby:
ZMON`, `receiver: on`, `receiver: not on within 3000 ms: the TV's own speakers meanwhile`), or why nothing was (`... the TV's sound
output is not the receiver: left alone`). The keeper's commands: `receiver: the screen is on and the receiver is off: power on`, `...
the TV input`, `receiver: the receiver does not answer|stays off: the TV's own speakers`, `receiver: on again: back to external_arc`.

## Devices behind the receiver

Under the tile of the port the receiver is on, the input picker lists what can be played through it. With the plugin on, the rows are the
receiver's own video inputs, by the names the owner gave them on the receiver (the settings document's `input` options, minus the sound-only
ones: TV Audio, network, Bluetooth, tuner, phono, CD, USB and the like), so a source LG's HDMI-CEC list does not know (a Blu-ray player that
does not answer CEC) still has a row. The row the receiver's current input is carries the mark, only while that port is in front. OK on a row
launches the port's app when it is not in front, then posts `/v1/receiver/set` `input` with the input's code.

The HDMI-CEC list (`subList` entries at `depth` 2) adds to a row: a device whose name matches the input's (equal, else one name contains the
other, three letters at least, spaces and case aside; two inputs that fit equally match none) shows its power ("Off") after the name. A CEC
device no input is named like stays as a row that is only named. Settings > Inputs & devices has a switch per receiver input
(`inputs.receiver-<code>` in theme.json) to take one out of the picker; all are shown by default.

The mark and the switch need the plugin on, the receiver's reading in date and the input setting writable. With the plugin off the rows
are the CEC devices, only named: no mark, no switch. Code: `rustui/ui/src/host/inputs.rs` (`source_for`, `children`),
`rustui/ui/src/panels/picker.rs`, `picker_surface.rs`, `rustui/ui/src/settings/pages/inputs.rs`.

Layout: a row is under its tile, set from the left like a tile's state line, its name wrapped to two lines at most; all rows
share one height. A column that would take the sheet off the top of the screen (large text) carries on in the free tile columns to its right.

## Receiver settings

Settings › Sound › Receiver shows the receiver's own settings while the plugin is on and the page is open. Code:
`helper/denon/src/settings.rs` (commands and replies), `helper/app/src/receiver_settings.rs` (what is offered, when it is writable),
`helper/app/src/receiver_telnet.rs` (write and read-back), `rustui/ui/src/settings/pages/receiver.rs`.

- **Model:** each connection reads the model name from the receiver's web interface. An AVR-S760H gets the full set: the FY21
  protocol's AVR-S960H commands, narrowed to the S760H owner's manual (Dialog Enhancer, no Pure Direct, no Center Level). Any other
  model gets power, input and the Stereo and Direct modes; an unidentified one gets nothing. Controls a model does not have are left
  off the page.
- **Sound mode:** the Movie, Music, Game and Pure buttons (`MSMOVIE`, `MSMUSIC`, `MSGAME`, `MSDIRECT`) and the modes Stereo, Direct,
  Auto, Dolby, DTS, Multi Ch Stereo, Virtual, Mono Movie, Rock Arena, Jazz Club, Matrix and Video Game. The receiver reports the
  decoder it ended on (`DOLBY AUDIO-DD+DSUR`, `M CH IN+NEURAL:X`), shown as is and grouped under the command that selects it. A
  button recalls its last mode or steps to the next, so its write is confirmed by any mode the manual lists under that button.
- **Quick Select 1–4** recalls a receiver memory; `MSQUICK0` (none recalled since a change) leaves all four choosable.
- **Levels:** Dialog Enhancer (not in Direct or Virtual), Dialog Control 0–6 (DTS:X), subwoofer level and each configured speaker's
  level (−12 to +12 dB in 0.5 dB steps; `CV?` lists only configured speakers), LFE 0 to −10 dB, dynamic compression (Auto not for
  DTS) and LFE for Dolby and DTS signals, loudness management for Dolby, subwoofer in Direct and Stereo, tone, Cinema EQ.
- **Audyssey:** MultEQ, Dynamic EQ, reference level offset, Dynamic Volume.
- **Zones and power:** Zone 2 power, input, volume (only with variable volume and a read limit) and mute; ECO.
- **Not carried:** Quick Select names (no protocol command), Zone 2 Quick Select, and the surround parameters the S760H manual marks
  unused (Center Spread, Speaker Virtualizer, Neural:X, effect, room size, delay, Restorer, manual EQ).

What can be changed right now comes from the receiver itself where it says: one `AppCommand0300.xml` request carries
`GetAudyssey`, `GetSurroundParameter` and `GetSoundMode`, and each control's `control` attribute is 2 for usable and 0 or 1 for
not (the S760H's own setup page agrees: `control="2"` items are shown, Dialog Control's `0` is hidden). The manual's mode and
signal rules apply only to what that reply leaves out. A Dolby Digital surround programme from Live TV reports `SYSDA Unknown`
and `SSINFAISSIG 12`, so the signal alone does not identify Dolby there; the receiver's reply does. A control that cannot be
changed stays on the page, disabled, with the reason under it.

Every change is one command, sent only for a value the helper advertises. After a mode, input or signal change, a control that
depends on it is held until the helper reads it again, which it does at once; values stay on screen meanwhile and are never
expired. A change counts as applied when the receiver's read-back matches. The page shows a change as soon as it is made, with
"Applying…" under it, and the read-back settles it; one that fails returns to the read-back with the error under the row. One
change is on its way at a time: changes made meanwhile wait, the latest per setting, and follow when it is settled. A sound mode
write waits for a matching mode until the 3 s deadline, because the receiver first repeats the mode being left. Nothing is
written in the background. While the receiver does not answer, the page keeps its last values, disabled, and the status chip
says "Not answering"; rows keep their size and place.
