#!/bin/sh
# X4 part 2: scripted launches with the monitor on, standby and wake last. No playback is started: Netflix and
# PlxNative are only brought to their first screen and sent behind Home again. Each step is stamped with the
# monitor's clock (uptime seconds) so the capture can be cut by step.
L="luna-send -n 1 -f"
AM=luna://com.webos.applicationManager
step() { echo "[$(cut -d' ' -f1 /proc/uptime)] $1"; }
home() { $L $AM/launch '{"id":"com.webos.app.home"}' >/dev/null 2>&1; sleep 4; }
cd /tmp/x4 || exit 1
[ -f all.jsonl ] && [ ! -f pass1.jsonl ] && mv all.jsonl pass1.jsonl
sh /tmp/x4-monitor.sh start
sleep 3
step "netflix launch (first screen only)"; $L $AM/launch '{"id":"netflix"}' | grep -E 'returnValue|errorText'; sleep 10
step "home"; home
step "plxnative launch (first screen only)"; $L $AM/launch '{"id":"com.beb.plxnative"}' | grep -E 'returnValue|errorText'; sleep 10
step "home"; home
step "input picker"; $L luna://com.webos.surfacemanager/showInputPicker '{}' | grep -E 'returnValue|errorText'; sleep 5
step "input picker close"; $L luna://com.webos.service.systemUIManager/closeByAppId '{"appId":"com.webos.app.quickinputpicker"}' | grep -E 'returnValue|errorText'; sleep 3
step "toast"; $L luna://com.webos.notification/createToast '{"message":"LumaTV X4 capture"}' | grep -E 'returnValue|errorText'; sleep 5
step "settings launch"; $L $AM/launch '{"id":"com.palm.app.settings"}' | grep -E 'returnValue|errorText'; sleep 8
step "home"; home
step "standby"; $L luna://com.webos.service.tvpower/power/powerOff '{"reason":"remoteKey"}' | grep -E 'returnValue|errorText'; sleep 30
step "state in standby: $($L luna://com.webos.service.tvpower/power/getPowerState '{}' | grep -oE '"state": *"[^"]*"')"
step "wake"; $L luna://com.webos.service.tvpower/power/powerOn '{"reason":"remoteKey"}' | grep -E 'returnValue|errorText'; sleep 25
step "screen on"; $L luna://com.webos.service.tvpower/power/turnOnScreen '{}' | grep -E 'returnValue|errorText|errorCode'; sleep 8
step "state after wake: $($L luna://com.webos.service.tvpower/power/getPowerState '{}' | grep -oE '"state": *"[^"]*"')"
step "foreground: $($L luna://com.webos.surfacemanager/getForegroundAppInfo '{}' | grep -oE '"appId": *"[^"]*"' | head -2 | tr '\n' ' ')"
step "picture mode: $($L luna://com.webos.settingsservice/getSystemSettings '{"category":"picture","keys":["pictureMode"]}' | grep -oE '"pictureMode": *"[^"]*"')"
sleep 2
sh /tmp/x4-monitor.sh stop
mv all.jsonl pass2.jsonl
ls -la /tmp/x4
