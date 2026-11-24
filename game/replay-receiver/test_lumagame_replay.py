"""python3 -m unittest test_lumagame_replay.py"""
import http.client
import json
import os
import socket
import struct
import tempfile
import threading
import time
import unittest

import lumagame_replay as replay

KEY, DELTA = 0x0200_0000, 0x0101_0000


def box(kind, *parts):
    body = b"".join(parts)
    return struct.pack(">I4s", 8 + len(body), kind) + body


def full(kind, version, flags, *parts):
    return box(kind, struct.pack(">I", version << 24 | flags), *parts)


def init():
    def trak(track, scale, handler):
        return box(
            b"trak",
            full(b"tkhd", 0, 3, struct.pack(">III", 0, 0, track), bytes(72)),
            box(b"mdia", full(b"mdhd", 0, 0, struct.pack(">IIII", 0, 0, scale, 0)), full(b"hdlr", 0, 0, bytes(4), handler, bytes(13))),
        )

    return box(b"ftyp", b"isom", bytes(4), b"isom") + box(b"moov", trak(1, 90_000, b"vide"), trak(2, 48_000, b"soun"))


def fragment(second, key, payload=b"v" * 1500, audio=True):
    """One second of video starting at `second`, with one second of audio beside it."""

    def traf(track, base, duration, flags, size):
        return box(
            b"traf",
            full(b"tfhd", 0, 0x020000, struct.pack(">I", track)),
            full(b"tfdt", 1, 0, struct.pack(">Q", base)),
            full(b"trun", 0, 0x0701, struct.pack(">Ii", 1, 0), struct.pack(">III", duration, size, flags)),
        )

    trafs = [traf(1, second * 90_000, 90_000, KEY if key else DELTA, len(payload))]
    if audio:
        trafs.append(traf(2, second * 48_000, 48_000, KEY, 4))
    return box(b"moof", full(b"mfhd", 0, 0, struct.pack(">I", second + 100)), *trafs) + box(b"mdat", payload, b"aaaa" if audio else b"")


def read(path):
    """A saved file as (init length, [(sequence, video base, audio base, key, mdat)])."""
    with open(path, "rb") as f:
        data = f.read()
    tracks = replay.timescales(data)
    out, start, first = [], None, None
    for kind, a, b in replay.boxes(data):
        if kind == b"moof":
            first = first if first is not None else a - 8
            start = a - 8
        elif kind == b"mdat":
            moof = replay.Moof(data[start:b], tracks)
            sequence = struct.unpack_from(">I", data, start + moof.sequence_at)[0]
            audio = next((base for _, _, base, video in replay.Moof(replay.rebased(data[start:b], moof, sequence, 0, tracks), tracks).times if not video), None)
            out.append((sequence, moof.video_base, audio, moof.key, data[a:b]))
    return first, out


class Case(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.buffer, self.recordings = os.path.join(self.dir.name, "buffer"), os.path.join(self.dir.name, "rec")
        self.addCleanup(self.dir.cleanup)
        self.addCleanup(lambda: [t.join(5) for t in threading.enumerate() if t.name in ("record", "save")])
        replay.log = lambda *words: None

    def receiver(self, **settings):
        settings = {"cap": 64 << 10, "chunk_bytes": 8 << 10, "record_floor": 0, **settings}
        return replay.Receiver(self.buffer, self.recordings, **settings)

    def stream(self, r, reach=3600, replay_on=True):
        r.open("s", replay_on, reach, 0)
        s = r.session
        s.live = True
        s.feed(init())
        return s

    def on_disk(self):
        return sum(os.path.getsize(os.path.join(self.buffer, n)) for n in os.listdir(self.buffer))

    def wait(self, done):
        deadline = time.monotonic() + 5
        while not done():
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.002)

    def test_the_buffer_rolls_under_its_cap(self):
        r = self.receiver()
        s = self.stream(r)
        for second in range(200):
            s.feed(fragment(second, key=second % 4 == 0))
            self.assertLessEqual(self.on_disk(), r.cap)
            self.assertEqual(self.on_disk(), s.used())
        self.assertGreater(self.on_disk(), r.cap - 2 * r.chunk_bytes)
        self.assertEqual(s.held[-1].base, 199 * 90_000)
        self.assertGreater(s.held[0].base, 0)

    def test_the_buffer_keeps_no_more_than_the_reach(self):
        r = self.receiver(cap=1 << 20)
        s = self.stream(r, reach=10)
        for second in range(100):
            s.feed(fragment(second, key=True))
        self.assertLess(s.held_seconds(), 10 + 2 * r.chunk_bytes / 1700)
        self.assertGreaterEqual(s.held_seconds(), 10)

    def test_the_cap_never_keeps_frames_that_have_no_keyframe_to_start_from(self):
        r = self.receiver()
        s = self.stream(r)
        for second in range(200):
            s.feed(fragment(second, key=second == 0))
            self.assertTrue(not s.held or any(h.key for h in s.held), second)
        self.assertEqual(s.held, [], "the buffer waits for the next keyframe")

    def test_a_clip_starts_on_the_keyframe_at_or_before_its_reach(self):
        r = self.receiver()
        s = self.stream(r)
        for second in range(20):
            s.feed(fragment(second, key=second % 4 == 0, payload=bytes([second]) * 900))
        told = s.save(6, 'Hades: II / "Test"')
        self.assertEqual((told["seconds"], told["name"].split("/")[0]), (8, "Hades II Test"))
        self.wait(lambda: not s.saving)
        path = os.path.join(self.recordings, told["name"])
        self.assertEqual(os.path.getsize(path), told["bytes"])
        first, fragments = read(path)
        self.assertEqual(first, len(init()))
        self.assertEqual([f[0] for f in fragments], list(range(1, 9)))
        self.assertEqual([f[1] for f in fragments], [n * 90_000 for n in range(8)])
        self.assertEqual([f[2] for f in fragments], [n * 48_000 for n in range(8)])
        self.assertTrue(fragments[0][3])
        self.assertEqual([f[4][0] for f in fragments], list(range(12, 20)), "the samples are the stream's own, in order")
        self.assertEqual(s.save(3600, "Hades")["seconds"], 20 - s.held[0].base // 90_000, "a longer reach than is held gives what is held")

    def test_a_clip_being_written_survives_the_buffer_rolling_past_it(self):
        r = self.receiver()
        s = self.stream(r)
        for second in range(20):
            s.feed(fragment(second, key=True))
        asked = []
        s.copy = lambda *args: asked.append(args)
        told = s.save(10, "Hades")
        self.wait(lambda: asked)
        with self.assertRaises(replay.Refused):
            s.save(10, "Hades")
        for second in range(20, 200):
            s.feed(fragment(second, key=True))
            self.assertLessEqual(s.used(), r.cap, "what the clip still reads counts under the cap")
        self.assertGreater(r.zombie_bytes, 0)
        del s.copy
        s.copy(*asked[0])
        self.assertEqual([f[1] for f in read(os.path.join(self.recordings, told["name"]))[1]], [n * 90_000 for n in range(10)])
        self.assertEqual((r.zombie_bytes, s.saving, self.on_disk()), (0, False, s.used()))

    def test_the_end_of_the_stream_empties_the_buffer_and_closes_the_recording(self):
        r = self.receiver()
        s = self.stream(r)
        s.record_start("Hades", 0)
        s.feed(fragment(0, key=False))
        for second in range(1, 6):
            s.feed(fragment(second, key=second in (1, 4)))
        self.assertEqual(s.status()["record_seconds"], 5)
        s.end("the upload ended")
        self.assertEqual((os.listdir(self.buffer), s.used(), s.status()["recording"]), ([], 0, 0))
        name = s.last_recording["name"]
        self.wait(lambda: os.path.exists(os.path.join(self.recordings, name)))
        fragments = read(os.path.join(self.recordings, name))[1]
        self.assertEqual(([f[1] for f in fragments], fragments[0][3]), ([n * 90_000 for n in range(5)], True), "from the first keyframe after the request")
        self.assertEqual(os.listdir(os.path.dirname(os.path.join(self.recordings, name))), [name.split("/")[1]])
        s.feed(fragment(6, key=True))
        self.assertEqual(os.listdir(self.buffer), [], "nothing is kept for a stream that ended")

    def test_a_recording_bypasses_the_buffer_and_ends_at_its_longest_length(self):
        r = self.receiver(record_max=3)
        s = self.stream(r, replay_on=False)
        self.assertEqual(s.record_start("Hades", 7200)["max"], 3)
        for second in range(6):
            s.feed(fragment(second, key=True))
        self.assertEqual((s.status()["recording"], s.last_recording["seconds"], os.listdir(self.buffer)), (0, 3, []))
        self.assertIn("longest", s.status()["last_note"])
        self.assertEqual(s.record_stop()["seconds"], 3, "stopping it again says what became of it")

    def test_a_full_share_refuses_a_recording_and_a_clip(self):
        r = self.receiver(record_floor=1 << 60)
        s = self.stream(r)
        s.feed(fragment(0, key=True))
        for ask in (lambda: s.record_start("Hades", 0), lambda: s.save(5, "Hades")):
            with self.assertRaises(replay.Refused) as refused:
                ask()
            self.assertEqual(refused.exception.status, 507)

    def test_a_reach_that_does_not_fit_the_cap_is_shortened_with_the_reason(self):
        r = self.receiver(cap=6 * replay.GB, chunk_bytes=32 << 20)
        self.assertEqual(r.grant(300, 65), (300, ""))
        reach, note = r.grant(900, 150)
        self.assertEqual(reach, 340)
        self.assertEqual(note, "The NAS buffer is 6 GB: at 150 Mbit/s it holds 5 min 40 s, not 15 min")
        self.assertEqual(r.open("s", True, 1800, 65)["reach"], 780)

    def test_start_clears_an_old_buffer_and_keeps_an_unfinished_recording(self):
        os.makedirs(os.path.join(self.recordings, "Hades"))
        os.makedirs(self.buffer)
        for path in (os.path.join(self.buffer, "replay-00000001.chunk"), os.path.join(self.recordings, "Hades", "a.mp4.part")):
            with open(path, "wb") as f:
                f.write(b"x")
        self.receiver()
        self.assertEqual((os.listdir(self.buffer), os.listdir(os.path.join(self.recordings, "Hades"))), ([], ["a.mp4"]))

    def test_over_http_a_stream_is_buffered_cut_and_dropped_when_the_upload_is_cut(self):
        r = self.receiver(live_timeout=5)
        server = replay.serve(r, 0, token="secret")
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        port = server.server_address[1]

        def ask(method, path, token="secret", body=None):
            c = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
            c.request(method, path, body=body, headers={"Authorization": f"Bearer {token}"})
            answer = c.getresponse()
            text = answer.read().decode()
            c.close()
            return answer.status, dict(line.split("=", 1) for line in text.splitlines() if "=" in line) if not text.startswith("{") else json.loads(text)

        self.assertEqual(ask("POST", "/open?session=a&replay=1&reach=300&mbps=20", token="wrong")[0], 401)
        self.assertEqual(ask("GET", "/health", token="")[0], 200)
        self.assertEqual(ask("POST", "/open?session=a&replay=1&reach=300&mbps=20")[1]["reach"], "0", "this small cap holds nothing at that rate")
        self.assertEqual(ask("POST", "/open?session=a&replay=1&reach=300&mbps=0")[1]["reach"], "300")
        live = socket.create_connection(("127.0.0.1", port))
        live.sendall(b"PUT /live?session=a HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret\r\nTransfer-Encoding: chunked\r\n\r\n")
        body = init() + b"".join(fragment(second, key=second % 2 == 0) for second in range(6))
        for at in range(0, len(body), 1000):
            piece = body[at:at + 1000]
            live.sendall(b"%x\r\n%s\r\n" % (len(piece), piece))
        self.wait(lambda: ask("GET", "/status?session=a")[1].get("held") == "6")
        self.assertEqual(ask("POST", "/status?session=a")[1]["held"], "6")
        status, told = ask("POST", "/save?session=a&seconds=3&game=Hades")
        self.assertEqual((status, told["seconds"]), (200, "4"))
        self.assertEqual(ask("PUT", "/clip?game=Hades", body=b"clip")[0], 200)
        live.close()
        self.wait(lambda: ask("GET", "/health")[1]["stream"] is None)
        self.wait(lambda: not r.session.saving)
        self.assertEqual(os.listdir(self.buffer), [])
        self.assertEqual(len(os.listdir(os.path.join(self.recordings, "Hades"))), 2)
        self.assertEqual(ask("POST", "/save?session=a&seconds=3&game=Hades")[0], 409)


if __name__ == "__main__":
    unittest.main()
