"""Video-plane container format regression tests."""
import os
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(__file__)), "tools"))
import vplane_clips as vc

def read(path):
    with open(path, "rb") as f:
        return f.read()

def write(path, data):
    with open(path, "wb") as f:
        f.write(data)

class Container(unittest.TestCase):
    def test_a_clip_reads_back_what_was_written(self):
        with tempfile.TemporaryDirectory() as d:
            units = [(b"\x00\x00\x00\x01\x46\x01" + b"k" * 90, True)] + [(b"\x00\x00\x00\x01\x02\x01" + bytes([i]) * 10, False) for i in range(1, 9)]
            path = os.path.join(d, "a.vpc")
            vc.write_vpc(path, "hevc", (1856, 1024), 60, units)
            got = vc.read_vpc(path)
            self.assertEqual((got["codec"], got["width"], got["height"], got["fps"], got["loopFrom"]), ("hevc", 1856, 1024, 60.0, 0))
            self.assertEqual(got["units"], units)
            blob = read(path)
            self.assertEqual(blob[:4], b"VPC1")
            self.assertEqual(struct.unpack_from("<I", blob, 32)[0], 40 + 8 * 9, "the payload starts after the table")
            write(path, blob[:-1])
            with self.assertRaises(ValueError):
                vc.read_vpc(path)

    def test_a_stream_is_cut_at_its_access_unit_delimiters_and_keys_are_found(self):
        sc = b"\x00\x00\x00\x01"
        # H.265: AUD is NAL type 35, IDR_W_RADL 19, a trailing picture 1; the NAL header's first byte is type << 1
        def h265(t, body=b"xx"):
            return sc + bytes([t << 1, 1]) + body
        stream = h265(35) + h265(32) + h265(33) + h265(34) + h265(19, b"idr") + h265(35) + h265(1, b"p1") + h265(35) + h265(1, b"p2")
        units = vc.split(stream, "hevc")
        self.assertEqual([k for _, k in units], [True, False, False])
        self.assertEqual(b"".join(u for u, _ in units), stream, "nothing lost, nothing added")
        self.assertTrue(units[0][0].startswith(sc + bytes([35 << 1, 1])))
        # H.264: AUD is type 9, IDR slice 5, non-IDR slice 1
        def h264(t, body=b"xx"):
            return sc + bytes([0x60 | t]) + body
        stream = h264(9) + h264(7) + h264(8) + h264(5, b"i") + h264(9) + h264(1, b"p")
        units = vc.split(stream, "h264")
        self.assertEqual([k for _, k in units], [True, False])
        with self.assertRaises(SystemExit):
            vc.split(h265(1) + h265(35), "hevc")     # a stream that does not begin with a delimiter

    def test_three_byte_start_codes_are_found_too(self):
        stream = b"\x00\x00\x01" + bytes([35 << 1, 1, 0x50]) + b"\x00\x00\x01" + bytes([19 << 1, 1, 5])
        self.assertEqual([k for _, k in vc.split(stream, "hevc")], [True])


