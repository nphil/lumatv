"""Run with: python3 -m unittest tests.test_package_tools

What tools/build-ipk.sh puts under tools/ on the set against what the tool calls there. The first round-7 TV run of `lumatv inputcommon`
answered "tools/inputcommon.sh did not answer" because that script and the two it needs were in the repository and not in the package.
The check is the closure: from the tool and the boot hook, every `$APPDIR/tools/<script>` a shell file calls and every sibling module a
python file imports, followed through the files it finds; each must be copied by a `cp ... "$DATA_ROOT/tools/"` line of the build
script and listed in its REQUIRED_FILES (the list the dry run and the early error read).
"""
import os
import re
import shlex
import subprocess
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLS = os.path.join(ROOT, "tools")
BUILD = os.path.join(TOOLS, "build-ipk.sh")
# The tool itself is installed under the brand's short id and built from tools/lumatv; the boot hook is its own entry point.
ENTRY = ("lumatv", "autostart.sh")


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def required(build_text):
    """The tools/ names of the REQUIRED_FILES block."""
    m = re.search(r'REQUIRED_FILES="\n(.*?)"\n', build_text, re.S)
    assert m, "REQUIRED_FILES is gone from build-ipk.sh"
    return {l.split("/", 1)[1] for l in m.group(1).replace("$SRC_NAME", "lumatv").split("\n") if l.startswith("tools/")}


def copied(build_text):
    """The tools/ names a `cp` line of the build script puts in the package's tools directory."""
    out = set()
    text = build_text.replace("\\\n", " ")
    for line in text.split("\n"):
        if not line.startswith("cp "):
            continue
        parts = shlex.split(line.replace("$SRC_NAME", "lumatv"), comments=True)
        if not parts[-1].startswith("$DATA_ROOT/tools/"):
            continue
        out.update(p.split("/", 1)[1] for p in parts[1:-1] if p.startswith("tools/"))
    return out


def shell_calls(path):
    """The tools/ scripts a shell file calls, from its code lines (not its comments)."""
    out = set()
    for line in read(path).split("\n"):
        if line.lstrip().startswith("#"):
            continue
        out.update(re.findall(r'\$\{?APPDIR\}?"?/tools/([A-Za-z0-9_.-]+)', line))
    return out


def python_imports(path):
    """The sibling modules of tools/ a python file imports at its top level."""
    out = set()
    for line in read(path).split("\n"):
        m = re.match(r"(?:from|import)\s+([A-Za-z0-9_]+)", line)
        if m and os.path.exists(os.path.join(TOOLS, m.group(1) + ".py")):
            out.add(m.group(1) + ".py")
    return out


def wanted():
    """Every tools/ file the entry points need, directly or through the files they call."""
    seen, todo = set(ENTRY), list(ENTRY)
    while todo:
        name = todo.pop()
        path = os.path.join(TOOLS, name)
        if name.endswith(".py"):
            found = python_imports(path)
        else:
            found = shell_calls(path)
        for n in found - seen:
            seen.add(n)
            todo.append(n)
    return seen


def missing(build_text, names):
    """What of `names` the build script does not copy or does not list as required (the tool itself is copied under its own name)."""
    have, listed = copied(build_text), required(build_text)
    return sorted(n for n in names if n not in have) + sorted(n + " (not in REQUIRED_FILES)" for n in names if n in have and n not in listed)


def required_all(build_text):
    """Every line of the REQUIRED_FILES block (the tools/ ones are `required`'s)."""
    m = re.search(r'REQUIRED_FILES="\n(.*?)"\n', build_text, re.S)
    assert m, "REQUIRED_FILES is gone from build-ipk.sh"
    return {l for l in m.group(1).replace("$SRC_NAME", "lumatv").split("\n") if l}


def staged_payload(build_text):
    """What `cp` lines of the build script put into one directory of the package's payload: {directory under payload/: [sources]}."""
    out = {}
    for line in build_text.replace("\\\n", " ").split("\n"):
        if not line.startswith("cp "):
            continue
        parts = shlex.split(line, comments=True)
        m = re.fullmatch(r'\$DATA_ROOT/payload/([^$]+?)/?', parts[-1])
        if m and not parts[1].startswith("-"):
            out.setdefault(m.group(1), []).extend(parts[1:-1])
    return out


def chmod_755(build_text):
    """The paths a `chmod 755` line of the build script marks executable."""
    out = set()
    for line in build_text.split("\n"):
        if line.startswith("chmod 755 "):
            out.update(shlex.split(line, comments=True)[2:])
    return out


# The frame app's ipk is built by build-ipk.sh itself (build_ipk) and copied in: its own early checks apply, not these.
BUILT_BY_THE_BUILD = {"frame/$FRAME_APP.ipk"}


def payload_refs():
    """The paths under payload/ that tools/lumatv names with a `NAME="$APPDIR/payload/<path>"` line, brand variables spelt as the sources are checked in."""
    refs = {}
    for m in re.finditer(r'^([A-Z_]+)="\$APPDIR/payload/([^"]+)"$', read(os.path.join(TOOLS, "lumatv")), re.M):
        refs[m.group(1)] = m.group(2).replace("$QML_MODULE", "lumatv").replace("$SHORT_ID", "lumatv")
    return refs


def payload_problems(build_text):
    """What of the payload files tools/lumatv names the package would lack. A file of the repository's payload/ comes with the tree copy; a built
    one (a program in a git-ignored build directory) needs a `cp` of its own into the same directory and its source in REQUIRED_FILES, so that a
    package built without it stops early and says so."""
    problems = []
    staged, listed = staged_payload(build_text), required_all(build_text)
    for name, path in sorted(payload_refs().items()):
        if path in BUILT_BY_THE_BUILD:
            continue
        directory, base = os.path.split(path)
        if os.path.exists(os.path.join(ROOT, "payload", path)):
            if 'cp -R payload/. "$DATA_ROOT/payload/"' not in build_text:
                problems.append("%s: payload/%s is in the repository and the build script no longer copies the payload tree" % (name, path))
            continue
        sources = [s for s in staged.get(directory, []) if os.path.basename(s) == base]
        if not sources:
            problems.append("%s: payload/%s is built and no cp puts it in $DATA_ROOT/payload/%s/" % (name, path, directory))
        elif not any(s in listed for s in sources):
            problems.append("%s: %s is copied and not in REQUIRED_FILES" % (name, sources[0]))
    return problems


class Packaged(unittest.TestCase):
    def setUp(self):
        self.build = read(BUILD)

    def test_everything_the_tool_and_the_boot_hook_call_is_in_the_package(self):
        names = wanted()
        self.assertGreater(len(names), 12, "the closure found almost nothing: %s" % sorted(names))
        self.assertEqual(missing(self.build, names), [])

    def test_the_closure_reaches_the_scripts_it_should(self):
        names = wanted()
        for must in ("lumatv", "autostart.sh", "bootcap.sh", "brand.sh", "compositor.sh", "inputcommon.sh", "inputcommon_patch.py", "voice.sh", "voice_patch.py", "maliit.sh", "maliit_patch.py", "keyboard_patch.py",
                     "notification_patch.py", "tokens.py", "preload.py", "theme.py", "keyfilter.py", "systemui.py", "systemui_patch.py", "tileicons.js"):
            self.assertIn(must, names)

    def test_the_input_banner_and_keyboard_restyle_are_shipped(self):
        # the three that were missing, and the modules they import
        for name in ("inputcommon.sh", "inputcommon_patch.py", "voice.sh", "voice_patch.py", "maliit.sh", "maliit_patch.py", "keyboard_patch.py", "notification_patch.py", "tokens.py"):
            self.assertIn(name, copied(self.build), name)
            self.assertIn(name, required(self.build), name)

    def test_what_the_build_script_copies_is_in_the_repository_and_in_the_required_list(self):
        for name in copied(self.build):
            self.assertTrue(os.path.exists(os.path.join(TOOLS, name)), name + " is copied and does not exist")
        self.assertEqual(sorted(n for n in copied(self.build) if n not in required(self.build)), [])

    def test_the_checker_finds_a_script_that_is_not_shipped(self):
        names = wanted()
        without_cp = self.build.replace("cp tools/inputcommon.sh tools/inputcommon_patch.py tools/voice.sh", "cp tools/inputcommon_patch.py tools/voice.sh")
        self.assertEqual(missing(without_cp, names), ["inputcommon.sh"])
        without_listed = self.build.replace("tools/keyboard_patch.py\n", "", 1)
        self.assertEqual(missing(without_listed, names), ["keyboard_patch.py (not in REQUIRED_FILES)"])
        without_import = self.build.replace("tools/notification_patch.py ", "")
        self.assertEqual(missing(without_import, names), ["notification_patch.py"])

    def test_the_dry_run_names_the_three_as_present(self):
        r = subprocess.run(["sh", BUILD, "--dry-run"], cwd=ROOT, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        for name in ("inputcommon.sh", "inputcommon_patch.py", "voice.sh", "voice_patch.py", "maliit.sh", "maliit_patch.py", "keyboard_patch.py", "tileicons.js", "systemui.py", "systemui_patch.py"):
            self.assertIn("✓ tools/%s\n" % name, r.stdout)
        self.assertNotIn("✗ tools/", r.stdout)

    def test_python_tools_are_run_by_python3_and_shell_ones_by_sh_so_no_execute_bit_is_needed(self):
        # the three are started as `sh <script>` / `python3 <script>`; a copy that lost its mode still runs
        text = read(os.path.join(TOOLS, "lumatv"))
        self.assertIn('sh "$INPUTCOMMON_TOOL"', text)
        self.assertIn('python3 "$KEYBOARD_TOOL"', text)
        self.assertIn('python3 "$IC_PATCH_TOOL"', read(os.path.join(TOOLS, "inputcommon.sh")))
        self.assertIn('sh "$VOICE_TOOL"', text)
        self.assertIn('exec sh "$APPDIR/tools/inputcommon.sh"', read(os.path.join(TOOLS, "voice.sh")))
        self.assertIn('sh "$KEYS_TOOL"', text)
        self.assertIn('python3 "$PATCH_TOOL"', read(os.path.join(TOOLS, "maliit.sh")))

    def test_every_payload_file_the_tool_names_is_in_the_package(self):
        self.assertGreaterEqual(len(payload_refs()), 15, sorted(payload_refs()))
        self.assertEqual(payload_problems(self.build), [])

    def test_the_checker_finds_a_payload_file_that_is_not_shipped(self):
        self.assertIn("VPLANE_SRC", payload_refs())
        no_cp = self.build.replace("cp vplane/build/arm/lumatv-vplane payload/vplane/vplane.sh", "cp payload/vplane/vplane.sh")
        self.assertEqual(payload_problems(no_cp), ["VPLANE_SRC: payload/vplane/lumatv-vplane is built and no cp puts it in $DATA_ROOT/payload/vplane/"])
        not_listed = self.build.replace("vplane/build/arm/lumatv-vplane\n", "", 1)
        self.assertEqual(payload_problems(not_listed), ["VPLANE_SRC: vplane/build/arm/lumatv-vplane is copied and not in REQUIRED_FILES"])
        no_tree = self.build.replace('cp -R payload/. "$DATA_ROOT/payload/"', "")
        self.assertTrue(any(p.startswith("VPLANE_SH:") for p in payload_problems(no_tree)))

    def test_the_video_plane_player_comes_with_its_script_role_and_clips_and_the_two_executables_are_marked(self):
        listed, staged, marked = required_all(self.build), staged_payload(self.build), chmod_755(self.build)
        for f in ("vplane/build/arm/lumatv-vplane", "payload/vplane/vplane.sh", "payload/vplane/role.json", "payload/vplane/clips/index.json"):
            self.assertIn(f, listed)
        self.assertEqual(sorted(staged["vplane"]), ["payload/vplane/role.json", "payload/vplane/vplane.sh", "vplane/build/arm/lumatv-vplane"])
        self.assertEqual(staged["vplane/clips"], ["payload/vplane/clips/*"])
        # vplane.sh install refuses a program that is not executable, and the tool runs the script with sh but the program is started as it stands
        self.assertIn("$DATA_ROOT/payload/vplane/lumatv-vplane", marked)
        self.assertIn("$DATA_ROOT/payload/vplane/vplane.sh", marked)
        self.assertIn("The 32-bit video-plane player is built by vplane/build.sh", self.build)

    def test_the_files_the_tool_names_for_the_player_are_the_ones_the_build_stages(self):
        refs = payload_refs()
        self.assertEqual({k: v for k, v in refs.items() if k.startswith("VPLANE_")},
                         {"VPLANE_SH": "vplane/vplane.sh", "VPLANE_SRC": "vplane/lumatv-vplane", "VPLANE_ROLE": "vplane/role.json", "VPLANE_CLIPS": "vplane/clips"})
        self.assertTrue(os.path.isdir(os.path.join(ROOT, "payload", "vplane", "clips")))

    def test_the_dry_run_lists_the_player_and_what_comes_with_it(self):
        r = subprocess.run(["sh", BUILD, "--dry-run"], cwd=ROOT, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        for f in ("payload/vplane/vplane.sh", "payload/vplane/role.json", "payload/vplane/clips/index.json"):
            self.assertIn("✓ %s\n" % f, r.stdout)
        # the program is git-ignored, so a checkout that has not run `sh vplane/build.sh` says so
        self.assertTrue("✓ vplane/build/arm/lumatv-vplane\n" in r.stdout or "✗ vplane/build/arm/lumatv-vplane (missing)\n" in r.stdout)


if __name__ == "__main__":
    unittest.main()
