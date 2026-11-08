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
