"""The TV probe of the native Home stub (tools/homestub-native-tv.sh) against a stand-in set: SAM and its log, the compositor's ctl, the tvpower service,
`lumatv` and /proc are small shell scripts over a few state files, and busybox sh runs the probe as it runs on the set. What is shown is the
probe's own logic, which is what protects the set: that it refuses before it changes anything, that each check says what it found, that a
failing check or a signal or a killed probe ends with the web stub back, and that what was not run is said so. What SAM really does with a
native Home is the TV's to say; the stand-in models one plausible answer and flags for the others.
"""
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
PROBE = ROOT / "tools" / "homestub-native-tv.sh"
BUSYBOX = "/usr/bin/busybox"
# SAM's log as the set keeps it, with an old launch event for Home from before the probe: no step of the probe may take it for its own
OLD_SAM = ('2026-11-09T23:20:00.000000Z [900.000000000] user.info sam [] SAM SUBSCRIPTION_REPLY '
           '{"skey":"getAppLifeEvents","payload":{"event":"launch","appId":"com.webos.app.home","returnValue":true,"reason":"undefined"}}\n')

# ---- the stand-in set: one file per fact in $TV, a flag is a file named flag-NAME
COMMON = r'''
TV="$FAKE_TV"
get() { cat "$TV/$1" 2>/dev/null; }
put() { printf '%s' "$2" > "$TV/$1"; }
flag() { [ -e "$TV/flag-$1" ]; }
log() { printf '%s\n' "$*" >> "$TV/$LOGNAME"; }
new_pid() {
  n=$(get nextpid); n=${n:-4242}; put nextpid $((n + 1))
  mkdir -p "$PROBE_PROC/$n"
  printf '/usr/palm/applications/com.webos.app.home/lumatv-homestub\0%s\0' "$1" > "$PROBE_PROC/$n/cmdline"
  printf 'Name:\tlumatv-homestub\nUid:\t0\t0\t0\t0\nVmRSS:\t     %s kB\n' "$(rss)" > "$PROBE_PROC/$n/status"
  printf 'Rss:                 %s kB\nPss:                  88 kB\n' "$(rss)" > "$PROBE_PROC/$n/smaps_rollup"
  echo $n
}
rss() { if flag big_rss; then echo 5120; else echo 124; fi; }
jlog() { printf '%s\n' "$*" >> "$TV/journal"; }
# The compositor with the native stub's window in it: it raises the card for the Home key and for SAM's launch event (home_stub.rs). An older
# compositor, or a program that does not announce itself, leaves both to SAM.
raises() { [ "$(get kind)" = native ] && ! flag no_announce && ! flag old_compositor; }
raise_home() {
  put home_state Fullscreen; put front com.webos.app.home
  [ -s "$TV/app" ] && put app_state Minimized
  return 0
}
# SAM ends the stub and starts another (the Home key's answer on the set, 2026-11-05)
restart_home() {
  put pid "$(new_pid x)"
  raises && raise_home
  return 0
}
# SAM's own lines are in dbg-log (not in messages): the launch it takes, and the life event it sends for a Home that is running
samlog() { printf '2026-11-09T23:30:34.093933Z [1577.679738160] user.info sam [] SAM %s\n' "$*" >> "$PROBE_DBGLOG"; }
sam_takes_launch() {
  samlog 'API_REQUEST {"category":"/","method":"launch","caller":"com.webos.lunasend-6737","status":"received_request"}'
  samlog 'APP_LAUNCH {"app_id":"com.webos.app.home","caller_id":"com.webos.lunasend-6737","status":"launch"}'
}
launch_event() {
  flag sam_silent && return 0
  # the log rotates between SAM's request and its event
  if flag log_rotates && [ -s "$TV/app" ]; then
    mv "$PROBE_DBGLOG" "$PROBE_DBGLOG.0"; echo '2026-11-09T23:31:00.000000Z [1600.000000000] user.info sam [] SAM NEW_LOG_FILE' > "$PROBE_DBGLOG"
  fi
  samlog 'SUBSCRIPTION_REPLY {"skey":"getAppLifeEvents","payload":{"event":"launch","appId":"com.webos.app.home","returnValue":true,"reason":"undefined"}}'
  raises || return 0
  flag launch_event_missed && return 0
  if [ "$(get front)" = com.webos.app.home ]; then
    jlog "home: SAM said launch for Home: its card is in front"
  else
    jlog "home: SAM said launch for Home: the stub's card raised"
    jlog "front: com.webos.app.home (window 7) raised to the front, not launched again"
    raise_home
  fi
}
# Home as SAM starts it: a launch argument that says preload maps nothing and takes no front
start_home() {
  [ -e "$TV/home_running" ] || : > "$TV/home_running"
  case "$(get kind)" in
    native)
      put pid "$(new_pid "$1")"
      case "$1" in *preload*) rm -f "$TV/home_mapped"; put home_state Default; return ;; esac
      put home_size "1,1" ;;
    *) put pid ""; put home_size "1920,1080" ;;
  esac
  : > "$TV/home_mapped"; put home_state Fullscreen; put front com.webos.app.home
  [ -s "$TV/app" ] && put app_state Minimized
  return 0
}
home_json() {
  state=$(get home_state); [ -e "$TV/home_mapped" ] && m=true || m=false
  props='{}'; [ "$(get kind)" = native ] && ! flag no_announce && props='{"lumatv.homeStub":"native"}'
  printf '{"appId":"com.webos.app.home","exposed":true,"front":1,"id":2,"inGroup":false,"keyMask":1,"mapped":%s,"position":[0,0],"props":%s,"size":[%s],"state":"%s","surface":1,"title":"Home","type":"Card"}' "$m" "$props" "$(get home_size)" "${state:-Default}"
}
'''

LUNA = COMMON + r'''
LOGNAME=luna-calls
for a in "$@"; do payload=$a; case "$a" in luna://*) uri=$a ;; esac; done
log "$uri $payload"
case "$uri" in
  *applicationManager/running)
    printf '{"running":['
    first=1
    if [ -e "$TV/home_running" ]; then printf '{"webprocessid":"","defaultWindowType":"card","id":"com.webos.app.home","processid":"%s","appType":"%s"}' "$(get pid)" "$(get type)"; first=0; fi
    if [ -s "$TV/app" ]; then [ $first = 0 ] && printf ','; printf '{"id":"%s","processid":"77","appType":"native"}' "$(get app)"; fi
    printf '],"returnValue":true}\n' ;;
  *applicationManager/getAppInfo) printf '{"appInfo":{"id":"com.webos.app.home","type":"%s"},"returnValue":true}\n' "$(get type)" ;;
  *applicationManager/getForegroundAppInfo) printf '{"appId":"%s","processId":1,"returnValue":true}\n' "$(get front)" ;;
  *applicationManager/launch)
    case "$payload" in
      *com.webos.app.home*)
        sam_takes_launch
        if [ -e "$TV/home_running" ] && [ "$(get kind)" = native ] && [ ! -e "$TV/home_mapped" ]; then
          # a stub that was started for a preload and maps nothing: the launch has it started again, with no hint (what else would map it)
          start_home x
        elif [ -e "$TV/home_running" ]; then
          # SAM's answer to a launch of the native Home that is running: nothing (the set, 2026-11-05), or the process ended and started again.
          # The compositor hears the launch event first.
          launch_event
          flag launch_restarts && [ "$(get kind)" = native ] && restart_home
          [ "$(get kind)" = native ] || { : > "$TV/home_mapped"; put home_state Fullscreen; put front com.webos.app.home; }
        else
          flag wake_breaks_home && [ "$(get woke)" = 1 ] && exit 0
          start_home "$payload"
        fi ;;
      *) flag app_stays_covered && [ -s "$TV/app" ] || { put app "$(printf '%s' "$payload" | sed 's/.*"id":"\([^"]*\)".*/\1/')"; put app_state Fullscreen; put front "$(get app)"; [ -e "$TV/home_mapped" ] && put home_state Minimized; } ;;
    esac
    echo '{"returnValue":true}' ;;
  *applicationManager/closeByAppId) put app ""; [ -e "$TV/home_running" ] && { put front com.webos.app.home; put home_state Fullscreen; }; echo '{"returnValue":true}' ;;
  *tvpower/power/getPowerState) printf '{"state":"%s","returnValue":true}\n' "$(get power)" ;;
  *tvpower/power/powerOff) put power "Active Standby"; echo '{"returnValue":true}' ;;
  *tvpower/power/powerOn)
    put power Active; put woke 1
    # bootd's wake path launches Home: the program is alive after it, or started again by it
    if flag wake_breaks_home; then rm -f "$TV/home_running" "$TV/home_mapped"; put pid ""; put front ""; fi
    echo '{"returnValue":true}' ;;
  *tvpower/power/turnOnScreen) echo '{"returnValue":true}' ;;
  *tvpower/power2/turnOnScreenSaver) flag saver_dead || put saver true; echo '{"returnValue":true}' ;;
  *oledepl/getPixelRefresherInfoList)
    if flag refresh_due; then last=1; else last=90; fi
    printf '{"returnValue":true,"list":[{"offrsCount":3,"offrsLastTime":%s,"offrsInterval":4}]}\n' "$last" ;;
  *panelcontroller/getPanelUsageTime) echo '{"returnValue":true,"panelUsageTime":540}' ;;
  *) echo '{"returnValue":true}' ;;
esac
'''

CTL = COMMON + r'''
LOGNAME=ctl-calls
# $1 ctl  $2 --runtime-dir  $3 DIR  $4...: the command
shift 3
log "$*"
case "$1" in
  windows)
    printf '{"groups":[],"ok":true,"video":[],"windows":['
    first=1
    if [ -e "$TV/home_running" ]; then home_json; first=0; fi
    if [ -s "$TV/app" ]; then [ $first = 0 ] && printf ','; printf '{"appId":"%s","exposed":true,"front":2,"id":3,"inGroup":false,"keyMask":1,"mapped":true,"position":[0,0],"props":{},"size":[1920,1080],"state":"%s","surface":2,"title":"App","type":"Card"}' "$(get app)" "$(get app_state)"; fi
    printf ']}\n' ;;
  shell) printf '{"ok":true,"saver":%s}\n' "$([ "$(get saver)" = true ] && echo true || echo false)" ;;
  note) shift; jlog "note: $*"; echo '{"ok":true}' ;;
  journal)
    printf '{"lines":['; first=1
    while IFS= read -r line; do [ $first = 0 ] && printf ','; printf '"%s"' "$line"; first=0; done < "$TV/journal"
    printf '],"ok":true}\n' ;;
  remote)
    case "$2" in
      773)
        flag home_key_dead && exit 0
        if raises; then
          if [ "$(get front)" = com.webos.app.home ]; then
            jlog "input: Home: Home's card is in front, SAM not asked"
          else
            jlog "input: Home: the stub's card raised, SAM not asked"
            jlog "front: com.webos.app.home (window 7) raised to the front, not launched again"
            raise_home
          fi
        else
          # the Home key goes to SAM, which ended the stub and started another
          jlog "input: Home: launchDefaultApp"
          [ "$(get kind)" = native ] && restart_home
          # the Home that SAM started maps and takes the front (the web stub asks for fullscreen)
          put front com.webos.app.home; put home_state Fullscreen
          [ -s "$TV/app" ] && put app_state Minimized
        fi ;;
      106) put saver false ;;
    esac ;;
esac
'''

TOOL = COMMON + r'''
LOGNAME=tool-calls
log "$*"
status() {
  printf '{"setting":"%s","wanted":"%s","mounted":"%s","sam":"%s","nativeInstalled":%s}' "$(get setting)" "$(get setting)" "$(get kind)" "$(get type)" "$(flag not_installed && echo false || echo true)"
}
case "$1:$2" in
  homestub-kind:|homestub-kind:status) printf '{"ok":true,"homestubKind":%s}\n' "$(status)" ;;
  homestub-kind:native)
    if flag swap_fails || flag native_never_starts; then echo '{"ok":false,"error":"the native stub did not take; the web setting is back"}'; exit 1; fi
    put kind native; put type native; put setting native; put pid ""; start_home '{"reason":"undefined"}'
    printf '{"ok":true,"homestubKind":%s}\n' "$(status)" ;;
  homestub-kind:web)
    if flag revert_fails; then echo '{"ok":false,"error":"the web stub did not take"}'; exit 1; fi
    put kind web; put type web; put setting web; put pid ""; start_home '{"reason":"undefined"}'
    printf '{"ok":true,"homestubKind":%s}\n' "$(status)" ;;
  boot-direct:*) printf '{"ok":true,"bootDirect":{"flag":true,"stub":true,"kind":"%s","sam":"%s"}}\n' "$(get kind)" "$(get type)" ;;
esac
'''

PGREP = COMMON + r'''
case "$*" in
  *-x*lumatv-homestub*) [ "$(get kind)" = native ] && [ -n "$(get pid)" ] && { get pid; echo; exit 0; }; exit 1 ;;
  *flutter-client*) flag flutter_starts && [ "$(get kind)" = native ] && { echo 812; exit 0; }; exit 1 ;;
  *lumatv-compositor*) flag no_compositor && exit 1; exit 0 ;;
esac
exit 1
'''

PKILL = COMMON + r'''
LOGNAME=pkill-calls
log "$*"
case "$*" in
  *lumatv-homestub*)
    put pid ""; rm -f "$TV/home_running" "$TV/home_mapped"; put front ""
    # the card below comes back unless the compositor holds it (home_stub.rs); an older one journals the promotion
    if [ -s "$TV/app" ] && { flag old_compositor || flag no_announce || flag restart_flashes; }; then
      put app_state Fullscreen; put front "$(get app)"; jlog "window 3: fullscreen (the front card)"
    fi ;;
esac
'''


@unittest.skipUnless(os.access(BUSYBOX, os.X_OK), "busybox is the shell of the set")
class ProbeAgainstAStandInSet(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="homestub-probe-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.tv = self.tmp / "tv"
        self.bin = self.tmp / "bin"
        self.proc = self.tmp / "proc"
        self.comp = self.tmp / "comp"
        self.app = self.tmp / "app"
        for d in (self.tv, self.bin, self.proc, self.comp / "bin", self.comp / "lib", self.app / "tools", self.app / "payload" / "homestub-native"):
            d.mkdir(parents=True)
        self.script("luna-send", LUNA)
        self.script("pgrep", PGREP)
        self.script("pkill", PKILL)
        self.script("sleep", "#!/bin/sh\nexit 0\n", raw=True)
        self.script("setsid", '#!/bin/sh\nexec "$@"\n', raw=True)
        (self.comp / "lib" / "ld-linux-aarch64.so.1").write_text('#!/bin/sh\nshift 2\nexec "$@"\n')
        (self.comp / "bin" / "lumatv-compositor").write_text("#!/bin/sh\n" + CTL)
        (self.app / "tools" / "lumatv").write_text("#!/bin/sh\n" + TOOL)
        for p in (self.comp / "lib" / "ld-linux-aarch64.so.1", self.comp / "bin" / "lumatv-compositor", self.app / "tools" / "lumatv"):
            p.chmod(0o755)
        (self.app / "payload" / "homestub-native" / "lumatv-homestub").write_text("program")
        (self.app / "payload" / "homestub-native" / "appinfo.json").write_text("{}")
        (self.proc / "uptime").write_text("1000.00 0.00\n")
        (self.tmp / "messages").write_text("")
        (self.tmp / "dbg-log").write_text(OLD_SAM)
        # the set as the probe wants it: the web stub in, Home in front, the panel on, LG's refresh not due
        for name, value in (("kind", "web"), ("type", "web"), ("setting", "web"), ("front", "com.webos.app.home"), ("power", "Active"),
                            ("home_state", "Fullscreen"), ("home_size", "1920,1080"), ("pid", ""), ("app", ""), ("app_state", "Fullscreen"),
                            ("saver", "false")):
            (self.tv / name).write_text(value)
        (self.tv / "home_running").write_text("")
        (self.tv / "home_mapped").write_text("")
        (self.tv / "journal").write_text("")
        self.out = self.tmp / "receipt"

    def script(self, name, body, raw=False):
        path = self.bin / name
        path.write_text(body if raw else "#!/bin/sh\n" + body)
        path.chmod(0o755)

    def flag(self, *names):
        for n in names:
            (self.tv / ("flag-" + n)).write_text("")

    def state(self, name):
        return (self.tv / name).read_text() if (self.tv / name).exists() else ""

    def switches(self):
        """The calls that change what is in: every other call of the tool is a read."""
        return [c for c in self.calls() if c in ("homestub-kind native", "homestub-kind web")]

    def calls(self, name="tool-calls"):
        path = self.tv / name
        return path.read_text().splitlines() if path.exists() else []

    def env(self, **extra):
        env = dict(os.environ, FAKE_TV=str(self.tv), PROBE_PROC=str(self.proc), PROBE_COMPOSITOR=str(self.comp),
                   LUMATV_TOOL=str(self.app / "tools" / "lumatv"), PROBE_MESSAGES=str(self.tmp / "messages"), PROBE_DBGLOG=str(self.tmp / "dbg-log"), PROBE_DEADMAN="0",
                   PATH=str(self.bin) + os.pathsep + os.environ["PATH"])
        env.update(extra)
        return env

    def probe(self, *args, **extra):
        return subprocess.run([BUSYBOX, "sh", str(PROBE), "--out", str(self.out), *args], env=self.env(**extra), capture_output=True, text=True, timeout=60)

    def lines(self, result):
        return [l for l in result.stdout.splitlines() if l.startswith("PROBE")]

    def verdict(self, result):
        return self.lines(result)[-1]

    def line(self, result, name):
        """The check's verdict line (a check may add INFO lines of what it saw, which come after it)."""
        found = [l for l in self.lines(result) if l.startswith("PROBE %s:" % name)]
        self.assertTrue(found, "no line for %s in:\n%s" % (name, result.stdout))
        return next((l for l in found if not l.startswith("PROBE %s: INFO" % name)), found[0])

    def cause(self, result):
        """What the probe says broke when its launch of Home over the app did not bring Home back."""
        found = [l for l in self.lines(result) if l.startswith("PROBE launch-over-app: INFO cause ")]
        self.assertEqual(len(found), 1, result.stdout)
        return found[0].split("INFO cause ", 1)[1]

    # ------------------------------------------------------------------------------------------------ a run that goes well

    def test_a_clean_run_passes_every_check_and_puts_the_web_stub_back(self):
        result = self.probe()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for name in ("swap", "sam-type", "running", "process", "window", "foreground", "memory", "launch", "preload", "preload-launch", "app", "home-key",
                     "restart", "launch-over-app", "app-process", "saver", "saver-end", "wake", "flutter", "revert"):
            self.assertIn("PASS", self.line(result, name), name)
        self.assertIn("RSS 124 kB", self.line(result, "memory"))
        self.assertIn("1 pixel", self.line(result, "window"))
        self.assertIn("announced as the native stub", self.line(result, "window"))
        self.assertIn("SAM not asked, the same process", self.line(result, "home-key"), "the key's route is named, not just its result")
        self.assertRegex(self.line(result, "home-key"), r"the whole call took \d+ cs")
        self.assertIn("raised the stub's card on SAM's life event (SAM said launch)", self.line(result, "launch-over-app"))
        self.assertIn("stayed covered", self.line(result, "restart"))
        self.assertRegex(self.verdict(result), r"^PROBE: PASS \(\d+ passed, 0 failed, 0 not run\)")
        self.assertEqual(self.switches(), ["homestub-kind native", "homestub-kind web"])
        self.assertEqual((self.state("kind"), self.state("type")), ("web", "web"), "left as it was found")
        for name in ("probe.log", "before.txt", "after.txt", "native-end.txt", "sam-dbg-log.txt", "launch-over-app.txt"):
            self.assertTrue((self.out / name).exists(), name)
        self.assertTrue((self.out / "native-end-program.txt").exists(), "the program's own /proc files are in the receipt")
        self.assertRegex((self.out / "probe.log").read_text(), r"hashes: tool [0-9a-f]{64} program [0-9a-f]{64} appinfo [0-9a-f]{64}")
        self.assertFalse((self.out / "deadman.armed").exists(), "the detached switch has nothing left to do")
        step = (self.out / "launch-over-app.txt").read_text()
        self.assertIn("API_REQUEST", step, "SAM's side of the launch is in the receipt")
        self.assertIn("the stub's card raised", step, "and the compositor's")
        self.assertNotIn("[900.000000000]", step, "not what SAM logged before the launch")
        self.assertNotIn("[900.000000000]", (self.out / "sam-dbg-log.txt").read_text(), "nor before the probe")
        self.assertTrue(self.cause(result).startswith("heard:"), self.cause(result))

    def test_keep_leaves_the_native_stub_in_place_when_everything_passed(self):
        result = self.probe("--keep")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.switches(), ["homestub-kind native"])
        self.assertNotIn("homestub-kind web", self.calls())
        self.assertEqual(self.state("kind"), "native")
        self.assertIn("left in place", self.verdict(result))

    def test_keep_does_not_keep_a_stub_that_failed_a_check(self):
        self.flag("home_key_dead")
        result = self.probe("--keep")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(self.state("kind"), "web", "failed: back to the web stub whatever was asked")

    def test_what_sam_does_on_a_launch_is_reported_either_way(self):
        self.assertIn("SAM keeps the running process", "\n".join(self.lines(self.probe())))
        self.flag("launch_restarts")
        self.assertIn("SAM restarted the process 3 time(s) of 3", "\n".join(self.lines(self.probe())))

    def test_the_options_skip_what_they_name_and_the_verdict_says_so(self):
        result = self.probe("--no-app", "--no-saver", "--no-wake")
        self.assertEqual(result.returncode, 0, result.stdout)
        for name in ("app", "saver", "wake"):
            self.assertIn("NOT RUN", self.line(result, name))
        self.assertRegex(self.verdict(result), r"0 failed, 3 not run; NOT RUN: app saver wake\)")
        self.assertFalse([c for c in self.calls("luna-calls") if "hdmi2" in c or "turnOnScreenSaver" in c or "powerOff" in c])

    def test_another_app_can_be_named_for_the_one_over_home(self):
        self.probe("--no-saver", "--no-wake", PROBE_APP="com.webos.app.livetv")
        self.assertTrue([c for c in self.calls("luna-calls") if "com.webos.app.livetv" in c])
        self.assertFalse([c for c in self.calls("luna-calls") if "hdmi2" in c])

    # --------------------------------------------------------------------------------------------- a run that finds a fault

    def test_a_failed_swap_fails_the_probe_and_asks_for_no_second_switch(self):
        self.flag("swap_fails")
        result = self.probe()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("FAIL", self.line(result, "swap"))
        self.assertEqual(self.switches(), ["homestub-kind native"], "the verb put things back itself: there is nothing to revert")
        self.assertEqual(self.state("kind"), "web")
        self.assertEqual([c for c in self.calls("ctl-calls") if c != "windows"], [], "and nothing was tried of a stub that is not there")

    def test_a_check_that_fails_is_reported_the_others_still_run_and_the_web_stub_is_back(self):
        self.flag("home_key_dead")
        result = self.probe()
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL", self.line(result, "home-key"))
        self.assertIn("did not bring Home back", self.line(result, "home-key"))
        self.assertIn("PASS", self.line(result, "launch-over-app"))
        self.assertIn("PASS", self.line(result, "saver"))
        self.assertIn("PASS", self.line(result, "revert"))
        self.assertEqual(self.switches()[-1], "homestub-kind web")
        self.assertRegex(self.verdict(result), r"^PROBE: FAIL \(\d+ passed, 1 failed")

    def test_a_program_over_the_memory_target_fails(self):
        self.flag("big_rss")
        result = self.probe()
        self.assertEqual(result.returncode, 1)
        self.assertIn("RSS 5120 kB", self.line(result, "memory"))
        self.assertIn("FAIL", self.line(result, "memory"))

    def test_a_compositor_that_leaves_the_key_and_the_launch_to_sam_fails_both_for_that_reason(self):
        # the set before home_stub.rs: the key made SAM end the stub and start another, a launch did nothing, and the card below came back
        self.flag("old_compositor")
        result = self.probe()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("through SAM and not through the compositor's raise", self.line(result, "home-key"))
        self.assertIn("1 launchDefaultApp line(s)", self.line(result, "home-key"))
        self.assertIn("FAIL", self.line(result, "launch-over-app"))
        self.assertTrue(self.cause(result).startswith("compositor-deaf:"), self.cause(result))
        self.assertIn("(window 3) was brought to the front", self.line(result, "restart"), "the app under the stub was brought back")
        self.assertIn("FAIL", self.line(result, "restart"))
        self.assertEqual(self.state("kind"), "web")

    def test_a_program_that_does_not_announce_itself_fails_the_window_check(self):
        self.flag("no_announce")
        result = self.probe()
        self.assertIn("FAIL", self.line(result, "window"))
        self.assertIn("no lumatv.homeStub property", self.line(result, "window"))
        self.assertIn("FAIL", self.line(result, "home-key"), "and nothing is raised for it")
        self.assertTrue(self.cause(result).startswith("no-card:"), "the card is not announced, and SAM sent its event: " + self.cause(result))

    def test_a_launch_event_that_does_not_come_fails_the_launch_over_an_app_alone(self):
        self.flag("launch_event_missed")
        result = self.probe()
        self.assertEqual(result.returncode, 1)
        self.assertIn("PASS", self.line(result, "home-key"), "the key is the compositor's own")
        self.assertIn("FAIL", self.line(result, "launch-over-app"))
        self.assertTrue(self.cause(result).startswith("compositor-deaf:"), self.cause(result))

    def test_a_sam_that_ends_the_stub_on_a_launch_still_gets_home_back_by_the_raise(self):
        self.flag("launch_restarts")
        result = self.probe()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("raised the stub's card on SAM's life event", self.line(result, "launch-over-app"))
        self.assertRegex(self.line(result, "launch-over-app"), r"stub pid \d+ -> \d+")

    def test_a_sam_that_takes_the_launch_and_sends_no_event_is_named_as_the_cause(self):
        self.flag("sam_silent")
        result = self.probe()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("FAIL", self.line(result, "launch-over-app"))
        self.assertTrue(self.cause(result).startswith("sam-silent:"), "the old event in the log is not this launch's: " + self.cause(result))
        step = (self.out / "launch-over-app.txt").read_text()
        self.assertIn("API_REQUEST", step)
        self.assertNotIn("getAppLifeEvents", step)

    def test_a_log_that_rotates_during_the_launch_is_read_across_the_rotation(self):
        self.flag("log_rotates", "launch_event_missed")
        result = self.probe()
        self.assertTrue(self.cause(result).startswith("compositor-deaf:"), "SAM's request is in the old file and its event in the new: " + self.cause(result))
        step = (self.out / "launch-over-app.txt").read_text()
        self.assertIn("API_REQUEST", step)
        self.assertIn("NEW_LOG_FILE", step)
        self.assertNotIn("[900.000000000]", step)
        run = (self.out / "sam-dbg-log.txt").read_text()
        self.assertIn("NEW_LOG_FILE", run)
        self.assertNotIn("[900.000000000]", run)

    def test_a_sam_log_that_cannot_be_read_is_said_and_the_cause_is_not_guessed(self):
        self.flag("launch_event_missed")
        result = self.probe(PROBE_DBGLOG=str(self.tmp / "no-such-dir" / "dbg-log"))
        self.assertIn("INFO", self.line(result, "dbg-log"))
        self.assertIn("FAIL", self.line(result, "launch-over-app"))
        self.assertTrue(self.cause(result).startswith("unknown:"), self.cause(result))

    def test_an_app_that_is_not_in_front_again_is_not_launched_over(self):
        self.flag("app_stays_covered")
        result = self.probe()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("FAIL", self.line(result, "app-again"))
        self.assertIn("NOT RUN", self.line(result, "launch-over-app"))
        self.assertFalse([c for c in self.calls("ctl-calls") if "probe-launch" in c], "Home was not launched over an app that was not in front")
        self.assertFalse((self.out / "launch-over-app.txt").exists())

    def test_an_app_that_shows_while_the_stub_is_gone_fails_the_restart(self):
        self.flag("restart_flashes")
        result = self.probe()
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL", self.line(result, "restart"))
        self.assertIn("the card the stub covered is not held", self.line(result, "restart"))
        self.assertIn("PASS", self.line(result, "home-key"))

    def test_a_flutter_home_that_starts_is_a_failure_of_its_own(self):
        self.flag("flutter_starts")
        result = self.probe()
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL", self.line(result, "flutter"))
        self.assertIn("LG's Flutter Home started", self.line(result, "flutter"))

    def test_a_home_that_does_not_survive_the_wake_fails_the_wake(self):
        self.flag("wake_breaks_home")
        result = self.probe()
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAIL", self.line(result, "wake"))
        self.assertEqual(self.state("kind"), "web")

    def test_the_wake_is_not_run_while_lgs_panel_refresh_is_due(self):
        self.flag("refresh_due")
        result = self.probe()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("NOT RUN", self.line(result, "wake"))
        self.assertIn("refresh is due", self.line(result, "wake"))
        self.assertIn("NOT RUN: wake", self.verdict(result))
        self.assertFalse([c for c in self.calls("luna-calls") if "powerOff" in c], "the set was not put in standby")

    def test_a_revert_that_does_not_take_is_said_loudly_and_fails(self):
        self.flag("revert_fails")
        result = self.probe()
        self.assertEqual(result.returncode, 1)
        self.assertIn("NOT BACK", self.line(result, "revert"))
        self.assertIn("Home held 10 s", self.line(result, "revert"))
        self.assertIn("THE WEB STUB IS NOT BACK", self.verdict(result))

    # ------------------------------------------------------------------------------------------------ refused, nothing changed

    def refused(self, why, **extra):
        result = self.probe(**extra)
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("REFUSED (nothing was changed)", result.stdout)
        self.assertIn(why, result.stdout)
        self.assertEqual(self.switches(), [], "no switch")
        return result

    def test_it_refuses_before_changing_anything(self):
        self.flag("not_installed")
        self.refused("payload/homestub-native is not installed")
        (self.tv / "flag-not_installed").unlink()
        (self.tv / "kind").write_text("native")
        self.refused("the web stub is not the one in place")
        (self.tv / "kind").write_text("web")
        self.flag("no_compositor")
        self.refused("compositor is not live")
        (self.tv / "flag-no_compositor").unlink()
        (self.tv / "power").write_text("Active Standby")
        self.refused("the set is not Active")
        (self.tv / "power").write_text("Active")
        (self.tv / "home_mapped").unlink()
        self.refused("no mapped Home card")
        (self.tv / "home_mapped").write_text("")
        self.refused("no tool", LUMATV_TOOL=str(self.tmp / "missing"))

    def test_it_refuses_a_set_with_a_program_of_that_name_running(self):
        (self.tv / "kind").write_text("native")
        (self.tv / "pid").write_text("99")
        (self.tv / "type").write_text("native")
        self.refused("is not the one in place")

    def test_an_unknown_option_prints_the_usage_and_changes_nothing(self):
        result = self.probe("--sideways")
        self.assertEqual(result.returncode, 2)
        self.assertIn("homestub-native-tv.sh [--keep]", result.stdout)
        self.assertEqual(self.calls(), [])

    # ------------------------------------------------------------------------------------- the ways back when it is cut short

    def start(self, *args, **extra):
        return subprocess.Popen([BUSYBOX, "sh", str(PROBE), "--out", str(self.out), *args], env=self.env(**extra), stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, text=True, start_new_session=True)

    def wait_call(self, text, seconds=10):
        return self.wait_until(lambda: any(text in c for c in self.calls()), seconds)

    def wait_until(self, done, seconds=10):
        end = time.time() + seconds
        while time.time() < end:
            if done():
                return True
            time.sleep(0.01)
        return False

    # What a `sleep` does in these tests, and nothing of it is the machine's speed. Once the native stub is in, a sleep holds the probe in
    # place (and leaves the file `held`) for as long as it takes to cut it short; the detached switch's own `sleep 7` (--budget 7) waits for
    # the file `release`, so the order of events is the test's.
    HOLD = '[ -e "$FAKE_TV/home_running" ] && [ "$(cat "$FAKE_TV/kind")" = native ] && { : > "$FAKE_TV/held"; exec /bin/sleep 5; }\n'
    BUDGET = 'case "$1" in 7) while [ ! -e "$FAKE_TV/release" ]; do /bin/sleep 0.01; done; : > "$FAKE_TV/woke"; exit 0 ;; esac\n'

    def test_a_signal_ends_with_the_web_stub_back(self):
        self.script("sleep", "#!/bin/sh\n" + self.HOLD + "exit 0\n", raw=True)
        proc = self.start()
        self.assertTrue(self.wait_until(lambda: (self.tv / "held").exists()))
        os.killpg(proc.pid, signal.SIGTERM)
        out, _ = proc.communicate(timeout=30)
        proc.stdout.close()
        self.assertEqual(proc.returncode, 1, out)
        self.assertIn("PROBE interrupted", out)
        self.assertEqual(self.switches()[-1], "homestub-kind web")
        self.assertEqual(self.state("kind"), "web")

    def test_a_probe_that_is_killed_hard_is_reverted_by_the_detached_switch(self):
        # SIGKILL runs no trap: what is left is the switch the probe started first.
        self.script("sleep", "#!/bin/sh\n" + self.BUDGET + self.HOLD + "exit 0\n", raw=True)
        proc = self.start("--budget", "7", PROBE_DEADMAN="1")
        self.assertTrue(self.wait_until(lambda: (self.tv / "held").exists()))
        proc.kill()
        proc.wait()
        proc.stdout.close()
        self.assertEqual(self.state("kind"), "native", "killed in the middle of it")
        (self.tv / "release").write_text("")
        self.assertTrue(self.wait_until(lambda: self.state("kind") == "web"), "the detached switch put it back")
        self.assertIn("deadman", (self.out / "probe.log").read_text())

    def test_the_detached_switch_does_nothing_after_a_probe_that_finished(self):
        self.script("sleep", "#!/bin/sh\n" + self.BUDGET + "exit 0\n", raw=True)
        result = self.probe("--budget", "7", "--no-app", "--no-saver", "--no-wake", PROBE_DEADMAN="1")
        self.assertEqual(result.returncode, 0, result.stdout)
        (self.tv / "release").write_text("")
        self.wait_until(lambda: (self.tv / "woke").exists(), 2)
        time.sleep(0.1)
        self.assertEqual(self.switches(), ["homestub-kind native", "homestub-kind web"], "no third: the probe's own revert was the only one")
        self.assertNotIn("deadman", (self.out / "probe.log").read_text())


class ProbeFiles(unittest.TestCase):
    def test_the_probe_is_posix_sh_and_never_touches_sam(self):
        for shell in (BUSYBOX, "/bin/sh"):
            if os.access(shell, os.X_OK):
                args = [shell, "sh", "-n"] if shell == BUSYBOX else [shell, "-n"]
                self.assertEqual(subprocess.run(args + [str(PROBE)], capture_output=True, text=True).returncode, 0, shell)
        text = PROBE.read_text()
        self.assertNotIn("systemctl", text)
        self.assertNotIn("restart sam", text)

    def test_its_header_says_what_it_needs_and_what_it_does(self):
        head = "\n".join(PROBE.read_text().splitlines()[:30])
        for word in ("--keep", "--no-wake", "docs/research/boot-direct.md", "Exit: 0", "payload/homestub-native"):
            self.assertIn(word, head)


if __name__ == "__main__":
    unittest.main()
