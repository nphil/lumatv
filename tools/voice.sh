#!/bin/sh
# LG's Search app (com.webos.app.voice) in the Luma look: a patched copy of its QML, laid over LG's directory by an overlay mount.
# POSIX sh / busybox ash. The same machinery as tools/inputcommon.sh (stage, overlay, LG's CRIU dump set aside, the idle instance ended
# through SAM so that it starts again from the patched files), run for another tree; this file only says which one.
#
#   voice.sh on|off|stage|status        as inputcommon.sh's, with "voice" in the JSON in place of "inputcommon"
#
# [INFERENCE] where the QML is: appinfo.json has `"main": "file:///usr/bin/com.webos.app.voice"` and the app's own Main.qml builds its picture path
# from `applicationDirPath + "/qml/Views/images/"`, so /usr/palm/applications/com.webos.app.voice/qml is expected; VOICE_QML overrides.
# The dump's directory in CRIU's working directory is matched by name: voiceweb and voiceagent must not be taken for it.
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
export IC_APPS=com.webos.app.voice
export IC_OVERRIDE_NAME=VOICE_QML
export IC_QML_OVERRIDE=${VOICE_QML:-}
exec sh "$APPDIR/tools/inputcommon.sh" "$@"
