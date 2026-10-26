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


class Packaged(unittest.TestCase):
    def setUp(self):
        self.build = read(BUILD)

    def test_everything_the_tool_and_the_boot_hook_call_is_in_the_package(self):
        names = wanted()
        self.assertGreater(len(names), 12, "the closure found almost nothing: %s" % sorted(names))
        self.assertEqual(missing(self.build, names), [])

    def test_the_closure_reaches_the_scripts_it_should(self):
        names = wanted()
        for must in ("lumatv", "autostart.sh", "bootcap.sh", "brand.sh", "compositor.sh", "inputcommon.sh", "inputcommon_patch.py", "keyboard_patch.py",
                     "notification_patch.py", "tokens.py", "preload.py", "theme.py", "keyfilter.py", "systemui.py", "systemui_patch.py", "tileicons.js"):
            self.assertIn(must, names)

    def test_the_input_banner_and_keyboard_restyle_are_shipped(self):
        # the three that were missing, and the modules they import
        for name in ("inputcommon.sh", "inputcommon_patch.py", "keyboard_patch.py", "notification_patch.py", "tokens.py"):
            self.assertIn(name, copied(self.build), name)
            self.assertIn(name, required(self.build), name)

    def test_what_the_build_script_copies_is_in_the_repository_and_in_the_required_list(self):
        for name in copied(self.build):
            self.assertTrue(os.path.exists(os.path.join(TOOLS, name)), name + " is copied and does not exist")
        self.assertEqual(sorted(n for n in copied(self.build) if n not in required(self.build)), [])

    def test_the_checker_finds_a_script_that_is_not_shipped(self):
        names = wanted()
        without_cp = self.build.replace("cp tools/inputcommon.sh tools/inputcommon_patch.py tools/keyboard_patch.py", "cp tools/inputcommon_patch.py tools/keyboard_patch.py")
        self.assertEqual(missing(without_cp, names), ["inputcommon.sh"])
        without_listed = self.build.replace("tools/keyboard_patch.py\n", "", 1)
        self.assertEqual(missing(without_listed, names), ["keyboard_patch.py (not in REQUIRED_FILES)"])
        without_import = self.build.replace("tools/notification_patch.py ", "")
        self.assertEqual(missing(without_import, names), ["notification_patch.py"])

    def test_the_dry_run_names_the_three_as_present(self):
        r = subprocess.run(["sh", BUILD, "--dry-run"], cwd=ROOT, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        for name in ("inputcommon.sh", "inputcommon_patch.py", "keyboard_patch.py", "tileicons.js", "systemui.py", "systemui_patch.py"):
            self.assertIn("✓ tools/%s\n" % name, r.stdout)
        self.assertNotIn("✗ tools/", r.stdout)

    def test_python_tools_are_run_by_python3_and_shell_ones_by_sh_so_no_execute_bit_is_needed(self):
        # the three are started as `sh <script>` / `python3 <script>`; a copy that lost its mode still runs
        text = read(os.path.join(TOOLS, "lumatv"))
        self.assertIn('sh "$INPUTCOMMON_TOOL"', text)
        self.assertIn('python3 "$KEYBOARD_TOOL"', text)
        self.assertIn('python3 "$IC_PATCH_TOOL"', read(os.path.join(TOOLS, "inputcommon.sh")))


if __name__ == "__main__":
    unittest.main()
