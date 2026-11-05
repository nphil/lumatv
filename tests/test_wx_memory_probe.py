"""Run with: python3 -m unittest tests.test_wx_memory_probe

The weather view's memory probe (tools/wx-memory-probe.sh) without a TV: the verdict it gives on a table of samples, which is what a TV session
reads, and the shape of a row it takes. The key presses and the readings of a set's processes are the TV's; what is judged here is that a view that
gives its memory back passes, that each way of not giving it back fails and names itself, and that what could not be read is skipped, not failed.
"""
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(os.path.dirname(HERE), "tools", "wx-memory-probe.sh")

COLUMNS = ("label uptime_s ui_rss ui_anon ui_file ui_shmem ui_pss ui_layer_files ui_layer_kib comp_rss comp_anon comp_file comp_shmem comp_pss comp_gpu_kib "
           "help_rss help_anon help_file help_shmem help_pss wx_state mem_available cma_free").split()

# a Home with its view closed: the processes at rest
REST = dict(uptime_s=100, ui_rss=150000, ui_anon=90000, ui_file=40000, ui_shmem=20000, ui_pss=120000, ui_layer_files=60, ui_layer_kib=30000, comp_rss=200000, comp_anon=60000,
            comp_file=40000, comp_shmem=100000, comp_pss=150000, comp_gpu_kib=180000, help_rss=30000, help_anon=20000, help_file=8000, help_shmem=2000, help_pss=29000,
            wx_state=409, mem_available=900000, cma_free=100000)


def row(label, **over):
    fields = {**REST, **over}
    return "\t".join([label] + [str(fields[c]) for c in COLUMNS[1:]])


def judged(rows, **env):
    with tempfile.TemporaryDirectory() as out:
        with open(os.path.join(out, "samples.tsv"), "w") as f:
            f.write("\t".join(COLUMNS) + "\n" + "\n".join(rows) + "\n")
        r = subprocess.run(["sh", SCRIPT, "judge"], env={**os.environ, "OUT": out, **env}, capture_output=True, text=True)
        return r.returncode, r.stdout


def cycle(closed=None, **over):
    """The four samples of a good run, with `closed` overriding the row after the close."""
    return [row("baseline"), row("open", ui_layer_files=68, ui_layer_kib=62000, ui_pss=170000, help_rss=70000, wx_state=200, **over),
            row("zoomed", ui_layer_files=70, ui_layer_kib=66000, ui_pss=175000, help_rss=75000, wx_state=200, **over), row("closed", **(closed or {}))]


class Verdict(unittest.TestCase):
    def test_a_view_that_gave_its_memory_back_passes(self):
        code, out = judged(cycle())
        self.assertEqual(code, 0, out)
        self.assertIn("VERDICT PASS", out)
        self.assertNotIn("FAIL ", out)

    def test_each_way_of_not_giving_it_back_fails_and_says_which(self):
        for what, closed in [("closed UI Pss", dict(ui_pss=130000)),
                             ("closed UI layer files", dict(ui_layer_files=64)),
                             ("closed UI layer KiB", dict(ui_layer_kib=32500)),
                             ("closed helper RSS", dict(help_rss=40000)),
                             ("closed helper view", dict(wx_state=200)),
                             ("closed compositor RSS", dict(comp_rss=210000)),
                             ("closed compositor GPU KiB", dict(comp_gpu_kib=190000))]:
            with self.subTest(what):
                code, out = judged(cycle(closed))
                self.assertEqual(code, 1, out)
                self.assertIn("FAIL  " + what, out)
                self.assertIn("VERDICT FAIL", out)

    def test_a_view_that_holds_too_much_while_open_fails(self):
        rows = cycle()
        rows[1] = row("open", ui_layer_files=200, ui_layer_kib=90000, help_rss=130000, wx_state=200)
        code, out = judged(rows)
        self.assertEqual(code, 1, out)
        for what in ("open UI layer files", "open UI layer KiB", "open helper RSS"):
            self.assertIn("FAIL  " + what, out)

    def test_what_could_not_be_read_is_skipped_and_the_rest_decides(self):
        code, out = judged(cycle(dict(comp_gpu_kib="n/a"), comp_gpu_kib="n/a"))
        self.assertEqual(code, 0, out)
        self.assertIn("SKIP  closed compositor GPU KiB", out)
        code, out = judged(cycle(dict(comp_gpu_kib="n/a", help_rss=60000), comp_gpu_kib="n/a"))
        self.assertEqual(code, 1, out)

    def test_the_later_row_is_judged_when_the_compositor_was_given_time(self):
        rows = cycle(dict(comp_rss=240000)) + [row("closed_late")]
        code, out = judged(rows)
        self.assertEqual(code, 0, out)
        self.assertIn("judged on the row 'closed_late'", out)

    def test_a_row_is_as_wide_as_its_header(self):
        # on a machine with none of the set's processes every reading is `n/a`, and the columns are still all there
        with tempfile.TemporaryDirectory() as out:
            r = subprocess.run(["sh", SCRIPT, "sample", "baseline"], env={**os.environ, "OUT": out, "PORT": "1", "CTL": "/nonexistent"}, capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            with open(os.path.join(out, "samples.tsv")) as f:
                lines = f.read().splitlines()
            self.assertEqual([len(line.split("\t")) for line in lines], [len(COLUMNS)] * 2, lines)
            self.assertEqual(lines[0].split("\t"), COLUMNS)


if __name__ == "__main__":
    unittest.main()
