#!/bin/sh
# LG's Search app (com.webos.app.voice) in the Luma look: a patched copy of its QML, laid over LG's directory by an overlay mount.
# POSIX sh / busybox ash. The same machinery as tools/inputcommon.sh (stage, overlay), run for another tree; this file only says which one.
# The app is resident: SAM keeps it alive (keepAliveApps), a close only hides it and its QML is read once, when the process starts. So
# `on` mounts and then ends the process (SIGTERM; tools/inputcommon.sh, "Resident apps"), and the next open, or LG's preload manager, starts
# it from the patched files; `off` unmounts and ends it again, so that LG's come back. LG has no CRIU dump of it (round 11: none in
# /tmp/checkpoint, and the preload list has it as `partial`), so nothing is set aside unless one ever turns up.
#
#   voice.sh on|off|stage|status        as inputcommon.sh's, with "voice" in the JSON in place of "inputcommon"; on and off add "ended"
#                                       (the app's id when its process was ended), status adds "pids" (the app's processes now)
#
# [INFERENCE] where the QML is: appinfo.json has `"main": "file:///usr/bin/com.webos.app.voice"` and the app's own Main.qml builds its picture path
# from `applicationDirPath + "/qml/Views/images/"`, so /usr/palm/applications/com.webos.app.voice/qml is expected; VOICE_QML overrides.
# Verified on the set in round 11 (the directory, Style.qml and Main.qml with the census copy's md5).
# The dump's directory in CRIU's working directory is matched by name: voiceweb and voiceagent must not be taken for it. The same goes for
# the process: comm is the binary's name cut to 15 characters (com.webos.app.v), which a lookalike binary shares, so the whole of argv[0] decides.
# The binary is /usr/bin/com.webos.app.voice (appinfo.json's main); VOICE_BINARY overrides it (the host tests start real processes under a path of their own).
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
export APPDIR
export IC_NAME=voice
export IC_ID=com.webos.app.voice
export IC_ROOT_FILE=Main.qml
export IC_STYLE_FILE=Style.qml
export IC_PATCH_TOOL="$APPDIR/tools/voice_patch.py"
export IC_ASIDE=.lumatv-aside-voice.
export IC_DUMP_GLOBS="com.webos.app.voice com.webos.app.voice[!a-zA-Z]*"
export IC_BINARY=${VOICE_BINARY:-/usr/bin/com.webos.app.voice}
export IC_APPS=com.webos.app.voice
export IC_OVERRIDE_NAME=VOICE_QML
export IC_QML_OVERRIDE=${VOICE_QML:-}
exec sh "$APPDIR/tools/inputcommon.sh" "$@"
