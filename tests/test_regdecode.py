"""Run with: python3 -m unittest tests.test_regdecode

tools/regdecode.py joins `/proc/lg/fbdev/regdump` captures with LG's OSD register headers. The layout and decode
tests run on a small header written here in LG's style (a master struct with a window union, `#if` branches, padding,
a macro inside the struct); the checks against LG's real headers and the repository's captures skip when the GPL tree
is not on this machine (set LG_KDRIVER to its kdriver/ directory).
"""
import contextlib
import importlib.util
import io
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
spec = importlib.util.spec_from_file_location("regdecode_under_test", os.path.join(ROOT, "tools", "regdecode.py"))
rd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rd)

KDRIVER = os.environ.get("LG_KDRIVER", rd.DEFAULT_KDRIVER)
HAVE_TREE = os.path.isfile(os.path.join(KDRIVER, rd.HEADER_DIR, "osd_reg_o22.h"))
E13 = os.path.join(ROOT, "docs", "research", "compositor", "osd01-probe-2026-10-29")
CAPTURE = os.path.join(ROOT, "docs", "research", "arch", "scanout-hw-02.txt")

OSD_HEADER = """
#ifndef __DEMO__
#define __DEMO__
typedef struct {
	UINT32
	reg_lo                          : 8,	//  0: 7
	                                : 8,	//  8:15 reserved
	reg_hi                          :16;	// 16:31
} DEMO_STATUS;

typedef struct {
	UINT32
	reg_x                           :12,	//  0:11
	                                : 4,	// 12:15 reserved
	reg_y                           :12;	// 16:27
} DEMO_POS;

typedef struct {
	UINT32
	reg_en                          : 1;	//     0
} DEMO_EN;

typedef union {
	/* OSD(0) only */
	struct {
		UINT32
		reg_zero_only               : 4;	//  0: 3
	};
	/* OSD(3) only */
	struct {
		UINT32
		reg_cursor_only             : 4;	//  0: 3
	};
} DEMO_SCALER;

typedef struct
{
	DEMO_STATUS                     	demo_mixer_status               ;	// 0x0000 : ''
	UINT32                          	                 __rsvd_00[   3];
	DEMO_EN                         	demo_mixer_en                   ;	// 0x0010 : ''
}
DEMO_MIXER;

typedef struct
{
	union
	{
		struct
		{
			DEMO_POS                    	pos                         ;	// 0x0000 : ''
			DEMO_SCALER                 	osd_ctrl_scaler             ;	// 0x0004 : ''
			DEMO_EN                     	osd_ctrl_path               ;	// 0x0008 : ''
		};
		struct
		{
			UINT32                      	                 __skip_00[   4];
			DEMO_STATUS                 	only_zero                   ;	// 0x0010 : ''
		};
		struct
		{
			UINT32                      	                 __skp_00[   4];
#if 1
			DEMO_EN                     	only_one                    ;	// 0x0010 : ''
#else
			DEMO_STATUS                 	dead_branch                 ;	// 0x0010 : ''
#endif
		};
		struct
		{
			UINT32                      	                 __skip_03[   4];
#if 0
			DEMO_STATUS                 	dead_too                    ;	// 0x0010 : ''
#endif
			DEMO_EN                     	only_three                  ;	// 0x0010 : ''
		};
		UINT32	__sep_1[64];
	};
}
OSDX_CTRL;

typedef struct
{
	union
	{
		struct
		{
			DEMO_STATUS                 	common_status               ;	// 0x0000 : ''
			UINT32                      	                 __rsvd_01[   7];
			DEMO_MIXER                  	mixer[2]                    ;
		};
		UINT32	__sep_0[128];
	};

#define DEMO_RD(_r)  ((g_demo->_r) = \\
                      (g_demo_shadow->_r))

	OSDX_CTRL                       	osdx_ctrl[4]                    ;
}
OSD_O22_REG_T;
#endif
"""

FLAT_HEADER = """
/*-----------------------------------------------------------------------------
    0x0000 flat_a ''
------------------------------------------------------------------------------*/
typedef struct
{
    UINT32
    start                           : 1,    //     0
                                    : 3,    //  1: 3 reserved
    ready                           : 12;   //  4:15
} FLAT_A;

/*-----------------------------------------------------------------------------
    0x0010 flat_b ''
------------------------------------------------------------------------------*/
typedef struct {
	UINT32
	count                           :16;	//  0:15
} FLAT_B;
"""


def dump_text(osd=None, top=None):
    """A regdump in the C4's format: an OSD block at 0xCC480100 and a TOP block at 0xCC480000."""
    out = []
    for name, base, words in (("TOP", 0xCC480000, top or {}), ("OSD", 0xCC480100, osd or {})):
        out.append("FBDEV (%s) : 0x%08x (size=%d)" % (name.upper(), base, 4 * (max(words, default=0) // 4 + 1)))
        for off in range(0, max(words, default=0) + 4, 4):
            out.append(" 0x%08x (+0x%04x) = %08x" % (base + off, off, words.get(off, 0)))
    return "\n".join(out) + "\n"


class Tree:
    """A temporary kdriver/ directory holding the demo headers under LG's path."""

    def __enter__(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = os.path.join(self.tmp.name, rd.HEADER_DIR)
        os.makedirs(root)
        for name, text in (("osd_reg_o22.h", OSD_HEADER), ("osd_top_reg_o22.h", FLAT_HEADER),
                           ("osd_afbc_reg_o22.h", FLAT_HEADER), ("osd_shp_reg_o22.h", FLAT_HEADER)):
            with open(os.path.join(root, name), "w") as f:
                f.write(text)
        return self.tmp.name

    def __exit__(self, *exc):
        self.tmp.cleanup()


def run(*argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
        code = rd.main(list(argv))
    return code, out.getvalue()


class LayoutTests(unittest.TestCase):
    def setUp(self):
        self.slots = rd.osd_slots(rd.Layout(OSD_HEADER))

    def labels(self, offset):
        return sorted(s.label for s in self.slots.get(offset, []))

    def test_common_block_and_array_of_aggregates(self):
        self.assertEqual(self.labels(0x000), ["common_status"])
        self.assertEqual(self.labels(0x020), ["mixer[0].demo_mixer_status"])
        self.assertEqual(self.labels(0x030), ["mixer[0].demo_mixer_en"])
        # the second mixer starts one 0x14-byte struct later, as the compiler lays it out
        self.assertEqual(self.labels(0x034), ["mixer[1].demo_mixer_status"])

    def test_windows_start_after_the_512_byte_union(self):
        self.assertEqual(self.labels(0x200), ["OSD0.pos"])
        self.assertEqual(self.labels(0x300), ["OSD0_1.pos"])
        self.assertEqual(self.labels(0x500), ["CURSOR.pos"])

    def test_each_window_sees_only_its_own_union_member(self):
        self.assertEqual(self.labels(0x210), ["OSD0.only_zero"])
        self.assertEqual(self.labels(0x310), ["OSD0_1.only_one"])
        self.assertEqual(self.labels(0x410), [])  # OSD1 has only the common member
        self.assertEqual(self.labels(0x510), ["CURSOR.only_three"])

    def test_preprocessor_branches_padding_and_macros(self):
        everything = {s.label for slots in self.slots.values() for s in slots}
        self.assertNotIn("OSD0_1.dead_branch", everything)
        self.assertNotIn("CURSOR.dead_too", everything)
        self.assertFalse([n for n in everything if "__" in n])
        self.assertIn(0x500, self.slots)  # the macro inside the struct did not end the parse early

    def test_planes_without_a_scaler_lack_the_scaler_and_path_registers(self):
        self.assertEqual(self.labels(0x204), ["OSD0.osd_ctrl_scaler"])
        self.assertEqual(self.labels(0x208), ["OSD0.osd_ctrl_path"])
        for window in (0x300, 0x400):
            self.assertEqual(self.labels(window + 4), [])
            self.assertEqual(self.labels(window + 8), [])
        self.assertEqual(self.labels(0x504), ["CURSOR.osd_ctrl_scaler"])

    def test_union_of_bit_field_structs_picks_by_window(self):
        zero = self.slots[0x204][0]
        cursor = self.slots[0x504][0]
        self.assertEqual([n for n, _, _, _ in zero.split(5)], ["reg_zero_only"])
        self.assertEqual([n for n, _, _, _ in cursor.split(5)], ["reg_cursor_only"])

    def test_fields_keep_positions_and_skip_reserved_bits(self):
        status = self.slots[0x000][0]
        self.assertEqual(status.split(0xABCD1234), [("reg_lo", 0, 8, 0x34), ("reg_hi", 16, 16, 0xABCD)])
        pos = self.slots[0x200][0]
        self.assertEqual(pos.split(0x0123F456), [("reg_x", 0, 12, 0x456), ("reg_y", 16, 12, 0x123)])

    def test_flat_headers_use_the_comment_offsets(self):
        flat = rd.flat_slots(FLAT_HEADER)
        self.assertEqual(sorted(flat), [0x0, 0x10])
        self.assertEqual(len(flat[0]), 1)
        self.assertEqual(flat[0][0].split(0xFF1)[0:2], [("start", 0, 1, 1), ("ready", 4, 12, 0xFF)])


class DumpTests(unittest.TestCase):
    def test_sections_words_and_devmem_lines(self):
        text = dump_text(osd={0x0: 7, 0x8: 9}) + "0xCC480360 0x02000001\n0x12345678 0x1\n"
        dump = rd.parse_dump(text)
        self.assertEqual(dump["OSD"], {0x0: 7, 0x4: 0, 0x8: 9, 0x260: 0x02000001})  # the devmem line joins its block
        self.assertEqual(set(dump), {"OSD", "TOP"})  # an address in no block is dropped

    def test_a_later_capture_replaces_an_earlier_one(self):
        text = dump_text(osd={0x4: 1}) + dump_text(osd={0x8: 2})
        self.assertEqual(rd.parse_dump(text)["OSD"], {0x0: 0, 0x4: 0, 0x8: 2})

    def test_evidence_files_with_other_sections_are_read(self):
        text = "######## chip/diag\npanel.width: 3840\n######## /proc/lg/fbdev/regdump\n" + dump_text(osd={0x4: 3})
        self.assertEqual(rd.parse_dump(text)["OSD"][0x4], 3)


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.tree = Tree()
        self.root = self.tree.__enter__()
        self.addCleanup(self.tree.__exit__, None, None, None)
        self.files = []

    def write(self, text):
        f = tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False)
        f.write(text)
        f.close()
        self.addCleanup(os.unlink, f.name)
        return f.name

    def test_decode_names_windows_and_splits_fields(self):
        dump = self.write(dump_text(osd={0x200: 0x0123F456, 0x310: 1}))
        code, out = run("--kdriver", self.root, "decode", dump, "--block", "OSD")
        self.assertEqual(code, 0)
        self.assertIn("OSD0.pos", out)
        self.assertIn("reg_x=0x456(1110) reg_y=0x123(291)", out)
        self.assertIn("OSD0_1.only_one", out)
        self.assertIn("reg_en=1", out)
        self.assertIn("0xcc480300", out)  # absolute address = block base + offset

    def test_decode_hides_zero_registers_unless_asked(self):
        dump = self.write(dump_text(osd={0x200: 5}))
        self.assertNotIn("common_status", run("--kdriver", self.root, "decode", dump)[1])
        self.assertIn("common_status", run("--kdriver", self.root, "decode", dump, "--all")[1])

    def test_diff_reports_changed_fields_only_and_exits_1(self):
        a = self.write(dump_text(osd={0x200: 0x00010002}))
        b = self.write(dump_text(osd={0x200: 0x00010009}))
        code, out = run("--kdriver", self.root, "diff", a, b)
        self.assertEqual(code, 1)
        self.assertIn("00010002 -> 00010009", out)
        self.assertIn("reg_x 2->9", out)
        self.assertNotIn("reg_y", out)

    def test_diff_of_equal_dumps_is_empty_and_exits_0(self):
        a = self.write(dump_text(osd={0x200: 1}))
        self.assertEqual(run("--kdriver", self.root, "diff", a, a), (0, ""))

    def test_noise_pair_and_ignore_hide_registers(self):
        a = self.write(dump_text(osd={0x200: 1, 0x300: 1}))
        b = self.write(dump_text(osd={0x200: 2, 0x300: 2}))
        n1 = self.write(dump_text(osd={0x200: 10}))
        n2 = self.write(dump_text(osd={0x200: 11}))
        code, out = run("--kdriver", self.root, "diff", a, b, "--noise", n1, n2)
        self.assertEqual(code, 1)
        self.assertNotIn("OSD0.pos", out)
        self.assertIn("OSD0_1.pos", out)
        self.assertEqual(run("--kdriver", self.root, "diff", a, b, "--ignore", "pos")[0], 0)

    def test_find_lists_absolute_addresses(self):
        code, out = run("--kdriver", self.root, "find", "only_one")
        self.assertEqual(code, 0)
        self.assertIn("0xcc480410", out)

    def test_missing_tree_names_the_fix(self):
        with self.assertRaises(SystemExit) as caught:
            run("--kdriver", "/nonexistent", "find", "x")
        self.assertIn("LG_KDRIVER", str(caught.exception))


@unittest.skipUnless(HAVE_TREE, "LG's GPL tree is not on this machine (set LG_KDRIVER)")
class LgTreeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.headers = rd.load_headers(KDRIVER)

    def where(self, label):
        return sorted(rd.BLOCKS["OSD"][1] + off for off, slots in self.headers["osd_reg_o22.h"].items()
                      if any(s.label == label for s in slots))

    def test_addresses_the_research_relies_on(self):
        self.assertEqual(self.where("OSD0.osd_ctrl_hdr0"), [0xCC480320])
        self.assertEqual(self.where("OSD0.osd_vsp_bosd_ctrl_0"), [0xCC480360])
        self.assertEqual(self.where("OSD0_1.osd_ctrl_hdr0"), [0xCC480420])
        self.assertEqual(self.where("OSD0_1.reg_4k_osd_main_ctrl_0"), [0xCC480440])
        self.assertEqual(self.where("CURSOR.osd_ctrl_hdr0"), [0xCC480620])
        self.assertEqual(self.where("osdx_blend_osd_mixer[1].ctrl_bosd_mixer_hole"), [0xCC48029C])
        self.assertEqual(self.where("osdx_blend_osd_mixer[1].ctrl_bosd_mixer_mux1"), [0xCC4802A4])

    def test_every_register_fits_in_a_word(self):
        for header, slots in self.headers.items():
            for off, group in slots.items():
                for s in group:
                    for alt in s.alts:
                        self.assertLessEqual(sum(w for _, _, w in alt), 32, (header, hex(off), s.label))
                        for _, lo, w in alt:
                            self.assertLessEqual(lo + w, 32, (header, hex(off), s.label))

    def test_osd_block_is_covered_to_its_last_word(self):
        self.assertIn(0x6D8, self.headers["osd_reg_o22.h"])  # the HDR-UI block ends at the dump's 1756 bytes

    @unittest.skipUnless(os.path.isfile(CAPTURE), "capture not in this checkout")
    def test_the_c4_capture_decodes_to_the_values_the_doc_quotes(self):
        code, out = run("--kdriver", KDRIVER, "decode", CAPTURE)
        self.assertEqual(code, 0)
        self.assertIn("osd_hdr_w_out=0xf14(3860)", out)
        self.assertIn("osd_hdr_h_out=0x87a(2170)", out)
        self.assertIn("r_contrast=0x200(512)", out)
        self.assertIn("reg_lti_2d_gain=0x10(16)", out)  # the sharpener gain `chip/diag` reports as ti_gain 0x10
        self.assertIn("hoffset=0xa(10)", out)

    @unittest.skipUnless(os.path.isdir(E13), "probe run not in this checkout")
    def test_the_osd0_1_probe_run_shows_the_probes_own_writes_and_the_orbit_step(self):
        noise = [os.path.join(E13, "noise-dry-run", n) for n in ("regdump-before.txt", "regdump-after.txt")]
        code, out = run("--kdriver", KDRIVER, "diff", os.path.join(E13, "regdump-before.txt"),
                        os.path.join(E13, "regdump-on.txt"), "--noise", *noise)
        self.assertEqual(code, 1)
        self.assertIn("OSD0_1.osd_ctrl_main", out)
        self.assertIn("osd_en 0->1", out)
        self.assertIn("osd_hdr_xpos 0->0x780(1920)", out)
        self.assertIn("osd_hdr_global_alpha 0xff(255)->0x80(128)", out)
        self.assertIn("hoffset 3->2", out)  # LG's orbit moved the crop offset between the two dumps
        self.assertNotIn("osd_status_pic_cnt", out)  # the counters tick by themselves and are in the noise pair


if __name__ == "__main__":
    unittest.main()
