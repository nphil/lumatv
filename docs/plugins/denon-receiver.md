# Denon receiver: power and sound output

The Denon receiver plugin (`denon-receiver`, off until switched on in Settings) keeps the receiver with the TV. It acts only while the
TV's saved sound output is the receiver (`external_arc` or `tv_speaker_external_arc` in `settingsservice` `sound.soundOutput`); a TV set to
its own speakers is left alone. Code: `helper/app/src/receiver.rs` (`sync_power`, `Keeper`), `receiver_sync.rs`.

- **Wake:** the screen coming on sends `ZMON`; standby sends `ZMOFF` as a backup to CEC when the receiver is still on.
- **Screen on:** from 5 s after the wake, a receiver read as off is sent `ZMON` again, backing off 5 s to 30 s, and set to the TV input
  (`SITV`) once it reads on after that command. A receiver switched off by hand while the screen is on gets one `ZMON`; switched off
  again, it is left off until the next wake.
- **Fallback:** a receiver that two reads in a row cannot reach, or that stays off after a power command, moves the TV's live route to its
  own speakers (`com.webos.service.audio/soundSettings/setSoundOut` `tv_speaker`), which leaves the saved preference as it is. One that
  answers again off is turned on; once it reads on, the route goes back to the saved output.
- **Standby:** nothing powers the receiver on while the screen is off.

The helper log has one line for each of these commands: `receiver: the screen is on and the receiver is off: power on`, `... the TV
input`, `receiver: the receiver does not answer|stays off: the TV's own speakers`.
