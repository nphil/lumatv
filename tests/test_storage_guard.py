"""Run with: python3 -m unittest tests.test_storage_guard

/var is a 966 MiB partition shared with LG, whose startup cleanup empties unprotected files at 98 % (blocks or inodes). An apply stages
whole copies beside the live ones; it refuses, before changing anything, when that peak would not leave a tenth of the partition free.
"""
import os
import subprocess
import tempfile
import unittest

from tests.test_tool_rules import TOOLS, function

FUNCS = "".join(function(n) for n in ("storage_capacity", "storage_need", "storage_fits", "storage_guard"))


def df_script(total_kb, free_kb, itotal, ifree, broken=False):
    if broken:
        return "#!/bin/sh\nexit 1\n"
    return (
        "#!/bin/sh\n"
        'case "$1" in\n'
        "  -Pi) echo 'Filesystem Inodes Used Available Use%% Mounted on'; echo '/dev/x %d %d %d 1%% /v' ;;\n"
        "  *) echo 'Filesystem 1024-blocks Used Available Capacity Mounted on'; echo '/dev/x %d %d %d 1%% /v' ;;\n"
        "esac\n"
    ) % (itotal, itotal - ifree, ifree, total_kb, total_kb - free_kb, free_kb)


class Guard(unittest.TestCase):
    def run_guard(self, total_kb, free_kb, itotal=64000, ifree=47000, need_build=0, payload_kb=1024, files=10, broken=False, current=False):
        with tempfile.TemporaryDirectory() as d:
            bindir = os.path.join(d, "bin")
            os.mkdir(bindir)
            with open(os.path.join(bindir, "df"), "w") as f:
                f.write(df_script(total_kb, free_kb, itotal, ifree, broken))
            os.chmod(os.path.join(bindir, "df"), 0o755)
            with open(os.path.join(bindir, "du"), "w") as f:
                f.write("#!/bin/sh\nprintf '%d\\t%%s\\n' \"$2\"\n" % payload_kb)
            os.chmod(os.path.join(bindir, "du"), 0o755)
            src = os.path.join(d, "luma")
            os.mkdir(src)
            for i in range(files):
                with open(os.path.join(src, "f%d" % i), "wb") as f:
                    f.write(b"x" * (payload_kb * 1024 // files))
            script = (
                'STATE="%s/state"; LUMA_SRC="%s"; STOCK_QML_MODULE=/nonexistent; FONTS_SRC=/nonexistent; AVATARS_SRC=/nonexistent\n'
                'THEME_SRC=/nonexistent; HELPER_SRC=/nonexistent; JPEG_SRC=/nonexistent; WALL_SRC=/nonexistent; STOCK_ASSETS=/nonexistent\n'
                'FONTS_DIR="%s/state/fonts"; AVATARS_DIR="%s/state/avatars"; THEME_DIR="%s/state/theme"; HELPER_BIN=/nonexistent2\n'
                'JPEG_BIN=/nonexistent2; WALLPAPER_STAGE=/nonexistent2; LUMA_STAGE=/nonexistent2\n'
                'luma_stage_current() { return %d; }\nshadow_current() { return %d; }\n'
                'log_line() { echo "log: $1"; }\n' % (d, src, d, d, d, 0 if current else 1, 0 if current else 1)
            ) + FUNCS + 'storage_guard %d; echo "rc=$? why=$STORAGE_WHY"\n' % need_build
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, env={"PATH": bindir + ":/usr/bin:/bin"}, timeout=30)
            return r.stdout.strip(), r.stderr

    def test_a_partition_with_room_passes(self):
        out, err = self.run_guard(989032, 800000)
        self.assertIn("rc=0", out, err)

    def test_the_partition_as_measured_on_the_set_is_refused(self):
        out, err = self.run_guard(989032, 21980)
        self.assertIn("rc=1", out, err)
        self.assertIn("not enough space", out)

    def test_the_boundary_is_the_tenth_that_stays_free(self):
        total = 989032
        keep = total // 10
        need = 2 * (1024 + 4)  # the staged Home tree and its copy in the shadow module
        ok, err = self.run_guard(total, keep + need)
        self.assertIn("rc=0", ok, err)
        no, err = self.run_guard(total, keep + need - 1)
        self.assertIn("rc=1", no, err)

    def test_a_small_partition_keeps_at_least_64_mib(self):
        out, err = self.run_guard(200000, 60000)
        self.assertIn("rc=1", out, err)

    def test_the_inodes_are_checked_on_their_own(self):
        out, err = self.run_guard(989032, 800000, itotal=64000, ifree=6000, files=10)
        self.assertIn("rc=1", out, err)
        self.assertIn("inodes", out)
        out, err = self.run_guard(989032, 800000, itotal=64000, ifree=6500, files=10)
        self.assertIn("rc=0", out, err)

    def test_an_apply_that_writes_nothing_passes_on_a_full_partition_and_one_that_writes_does_not(self):
        out, err = self.run_guard(989032, 21980, current=True)
        self.assertIn("rc=0", out, err)
        out, err = self.run_guard(989032, 21980, current=False)
        self.assertIn("rc=1", out, err)

    def test_an_unreadable_filesystem_is_logged_not_guessed(self):
        out, err = self.run_guard(1, 1, broken=True)
        self.assertIn("rc=0", out, err)
        self.assertIn("not checked", out)


class StagedTree(unittest.TestCase):
    def test_luma_stage_current_follows_what_stage_luma_writes(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(d + "/src/sub")
            os.makedirs(d + "/th")
            for rel in ("src/a.qml", "src/sub/b.qml", "th/theme.json", "th/layout.json"):
                with open(os.path.join(d, rel), "w") as f:
                    f.write(rel)
            script = (
                'LUMA_SRC=%s/src; THEME_SRC=%s/th; LUMA_STAGE=%s/stage; QML_MODULE=lumatv; cfg_get_bool() { echo false; }\n' % (d, d, d)
                + function("stage_luma") + function("luma_stage_current")
                + 'luma_stage_current && echo before; stage_luma false; luma_stage_current && echo staged\n'
                'echo x > "$LUMA_STAGE/lumatv/a.qml"; luma_stage_current && echo edited\n'
                'stage_luma false; touch "$LUMA_STAGE/lumatv/extra"; luma_stage_current && echo extra\n'
            )
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
            self.assertEqual(r.stdout.split(), ["staged"], r.stderr)


class ApplyOrder(unittest.TestCase):
    def test_the_guard_runs_before_apply_changes_anything(self):
        body = function("apply_run")
        guard = body.index("storage_guard")
        for mutation in ('cp -R . "$ASSETS.building"', 'rm -rf "$ASSETS"', "stage_compositor ", "stage_luma ", "build_shadow", "write_env"):
            self.assertLess(guard, body.index(mutation), mutation)
        self.assertIn("emit_error", body[guard:guard + 400])


if __name__ == "__main__":
    unittest.main()
