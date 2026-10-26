.pragma library

// Which sound outs LG lets the TV's own sound settings act on (research/lgsettings/main.js: `QH` and `i6`). The sound out is the key
// `sound.soundOutput`; the ones that include the TV's speakers are the TV alone and each "+ TV Speaker" pairing.
var withTv = ["tv_speaker", "tv_external_speaker", "tv_speaker_external_arc", "tv_speaker_headphone", "tv_speaker_bluetooth", "tv_speaker_wow_cast", "tv_speaker_optical_arc"];

// The TV's speakers play (balance, installation type, AI acoustic tuning).
function tvSpeakers(out) { return withTv.indexOf(String(out)) >= 0; }
// The TV's speakers or the eARC receiver (Dolby Atmos, automatic volume).
function tvOrArc(out) { return tvSpeakers(out) || String(out) === "external_arc"; }
// The Equalizer: the TV's speakers, but not with a Bluetooth device.
function equalizer(out) { return tvSpeakers(out) && String(out) !== "tv_speaker_bluetooth"; }
// The sound goes to the receiver over HDMI(ARC), alone or with the TV's speakers.
function toReceiver(out) { return String(out) === "external_arc" || String(out) === "tv_speaker_external_arc"; }
