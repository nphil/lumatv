#!/usr/bin/env python3
"""LumaGame's replay receiver: the NAS end of the game recorder.

The TV sends the stream it is playing as fragmented MP4 over one long HTTP PUT. This service keeps the newest part of it in a rolling
buffer under a hard byte cap, cuts "the last N minutes" out of that buffer at a keyframe into one file, and writes a recording
straight to the recordings folder while one is asked for. Standard library only.

    PORT=8765 BUFFER_DIR=/buffer RECORDINGS_DIR=/recordings python3 lumagame_replay.py

Requests (answers are `key=value` lines; a refusal is a 4xx or 5xx with `note=` saying why in one sentence):

    GET  /health                                         JSON: caps, what is held, free space
    POST /open?session=S&replay=0|1&reach=SECONDS&mbps=N  starts a stream; ends the one before it; grants the reach that fits the cap
    PUT  /live?session=S                                 the stream itself, a chunked body: ftyp, moov, then moof+mdat pairs
    POST /replay?session=S&on=0|1&reach=SECONDS&mbps=N    turns the rolling buffer on or off while the stream runs
    POST /save?session=S&seconds=N&game=NAME             cuts the last N seconds out of the buffer into one file
    POST /record/start?session=S&game=NAME&max=SECONDS   records from the next keyframe on
    POST /record/stop?session=S
    GET  /status?session=S                               whether the stream is live, what the buffer holds, the recording
    PUT  /clip?game=NAME                                 one finished clip from the TV's own memory

The buffer is one stream's: it is deleted when that stream's upload ends or stalls, and at start. Files go to
`RECORDINGS_DIR/<Game Name>/<YYYY-MM-DD HH.MM.SS>.mp4`. Plain HTTP: keep the port on the LAN; `TOKEN` adds a bearer check.
"""
import json
import os
import queue
import re
import struct
import sys
import threading
import time
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

VERSION = "1"
GB = 1 << 30
SYNC_MASK = 0x0001_0000  # sample_is_non_sync_sample


def log(*words):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), *words, flush=True)


class Refused(Exception):
    def __init__(self, status, note):
        super().__init__(note)
        self.status, self.note = status, note


def boxes(data, start=0, end=None):
    """Yields (type, body start, box end) for each whole box in data[start:end]."""
    end = len(data) if end is None else end
    at = start
    while at + 8 <= end:
        size, kind = struct.unpack_from(">I4s", data, at)
        head = 8
        if size == 1:
            if at + 16 > end:
                return
            size, head = struct.unpack_from(">Q", data, at + 8)[0], 16
        if size < head or at + size > end:
            return
        yield kind, at + head, at + size
        at += size


def timescales(init):
    """Reads the init segment: {track id: (timescale, is video)}."""
    tracks = {}
    for kind, a, b in boxes(init):
        if kind != b"moov":
            continue
        for kind, a, b in boxes(init, a, b):
            if kind != b"trak":
                continue
            track, scale, video = None, None, False
            for kind, ta, tb in boxes(init, a, b):
                if kind == b"tkhd":
                    track = struct.unpack_from(">I", init, ta + (20 if init[ta] == 1 else 12))[0]
                elif kind == b"mdia":
                    for kind, ma, _ in boxes(init, ta, tb):
                        if kind == b"mdhd":
                            scale = struct.unpack_from(">I", init, ma + (20 if init[ma] == 1 else 12))[0]
                        elif kind == b"hdlr":
                            video = init[ma + 8:ma + 12] == b"vide"
            if track is not None and scale:
                tracks[track] = (scale, video)
    return tracks


class Moof:
    """What one `moof` says: where its sequence number and each track's base time sit, and the video run's start, length and key."""

    def __init__(self, data, tracks):
        self.sequence_at = None
        self.times = []  # (offset of the base time, its width, base, is video)
        self.video_base = self.video_ticks = None
        self.key = False
        for kind, a, b in boxes(data):
            if kind != b"moof":
                continue
            for kind, a, b in boxes(data, a, b):
                if kind == b"mfhd":
                    self.sequence_at = a + 4
                elif kind == b"traf":
                    self.traf(data, a, b, tracks)

    def traf(self, data, a, b, tracks):
        track = default_duration = default_flags = None
        base = base_at = width = None
        ticks, first_flags, runs = 0, None, 0
        for kind, a, b in boxes(data, a, b):
            if kind == b"tfhd":
                flags = struct.unpack_from(">I", data, a)[0] & 0xFFFFFF
                track = struct.unpack_from(">I", data, a + 4)[0]
                at = a + 8 + (8 if flags & 1 else 0) + (4 if flags & 2 else 0)
                if flags & 8:
                    default_duration = struct.unpack_from(">I", data, at)[0]
                    at += 4
                at += 4 if flags & 0x10 else 0
                if flags & 0x20:
                    default_flags = struct.unpack_from(">I", data, at)[0]
            elif kind == b"tfdt":
                width = 8 if data[a] == 1 else 4
                base_at = a + 4
                base = struct.unpack_from(">Q" if width == 8 else ">I", data, base_at)[0]
            elif kind == b"trun":
                flags = struct.unpack_from(">I", data, a)[0] & 0xFFFFFF
                count = struct.unpack_from(">I", data, a + 4)[0]
                at = a + 8 + (4 if flags & 1 else 0)
                run_first = None
                if flags & 4:
                    run_first = struct.unpack_from(">I", data, at)[0]
                    at += 4
                step = sum(4 for bit in (0x100, 0x200, 0x400, 0x800) if flags & bit)
                for i in range(count):
                    if at + step > b:
                        break
                    ticks += struct.unpack_from(">I", data, at)[0] if flags & 0x100 else (default_duration or 0)
                    if i == 0 and run_first is None and flags & 0x400:
                        run_first = struct.unpack_from(">I", data, at + sum(4 for bit in (0x100, 0x200) if flags & bit))[0]
                    at += step
                if runs == 0:
                    first_flags = run_first if run_first is not None else default_flags
                runs += 1
        if track is None or base is None:
            return
        video = tracks.get(track, (0, False))[1]
        self.times.append((base_at, width, base, video))
        if video and self.video_base is None:
            self.video_base, self.video_ticks = base, ticks
            self.key = first_flags is not None and not first_flags & SYNC_MASK


def rebased(fragment, moof, sequence, video_origin, scales):
    """The fragment with its sequence number set and every track's base time moved so the cut starts at zero."""
    out = bytearray(fragment)
    if moof.sequence_at is not None:
        struct.pack_into(">I", out, moof.sequence_at, sequence)
    video_scale = next((s for s, v in scales.values() if v), 90_000)
    for at, width, base, video in moof.times:
        if video:
            origin = video_origin
        else:
            scale = next((s for s, v in scales.values() if not v), 48_000)
            origin = video_origin * scale // video_scale
        struct.pack_into(">Q" if width == 8 else ">I", out, at, max(0, base - origin))
    return out


def icon(size=192):
    """The Docker tab's icon, drawn here so the container needs no file from anywhere else: a record dot in a ring."""
    rows, mid = bytearray(), size / 2
    for y in range(size):
        rows.append(0)
        for x in range(size):
            d = ((x - mid + 0.5) ** 2 + (y - mid + 0.5) ** 2) ** 0.5 / size
            rows += bytes((235, 64, 52, 255) if d < 0.19 else (240, 240, 245, 255) if 0.30 < d < 0.36 else (24, 26, 32, 255) if d < 0.48 else (0, 0, 0, 0))
    part = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return b"\x89PNG\r\n\x1a\n" + part(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) + part(b"IDAT", zlib.compress(bytes(rows), 9)) + part(b"IEND", b"")


def folder_name(game):
    name = re.sub(r'[\x00-\x1f\x7f\\/:*?"<>|]', "", game or "")
    name = re.sub(r"\s+", " ", name).strip(" .")[:80].strip(" .")
    return name or "Unknown Game"


class Chunk:
    """One file of the rolling buffer."""

    def __init__(self, path):
        self.path, self.size, self.pins, self.gone = path, 0, 0, False
        self.fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_TRUNC, 0o660)


class Held:
    """One fragment in the buffer."""
    __slots__ = ("chunk", "offset", "size", "base", "ticks", "key")

    def __init__(self, chunk, offset, size, base, ticks, key):
        self.chunk, self.offset, self.size, self.base, self.ticks, self.key = chunk, offset, size, base, ticks, key


class Recording:
    """A file being recorded. The stream's thread only queues fragments; a writer thread owns the file, so a slow share cannot hold
    the stream up. When the queue is full the recording ends there."""

    def __init__(self, receiver, name, max_seconds):
        self.receiver, self.name = receiver, name
        self.target = os.path.join(receiver.recordings, name)
        self.max_seconds = max_seconds
        self.queue = queue.Queue()
        self.queued = 0
        self.lock = threading.Lock()
        self.origin = None
        self.sequence = 0
        self.seconds = 0.0
        self.bytes = 0
        self.note = ""
        self.closed = False
        self.failed = False
        self.thread = threading.Thread(target=self.write, name="record", daemon=True)
        self.thread.start()

    def put(self, data):
        with self.lock:
            self.queued += len(data)
        self.queue.put(data)

    def write(self):
        partial = self.target + ".part"
        out, last_check = None, 0.0
        try:
            while True:
                data = self.queue.get()
                if data is None:
                    break
                if self.failed:
                    continue
                try:
                    if out is None:
                        os.makedirs(os.path.dirname(self.target), exist_ok=True)
                        out = open(partial, "wb")
                    out.write(data)
                    if time.monotonic() - last_check > 2:
                        last_check = time.monotonic()
                        if self.receiver.free(self.receiver.recordings) < self.receiver.record_floor:
                            self.note = self.note or "Recording stopped: the share is nearly full"
                            self.failed = True
                except OSError as e:
                    self.note = self.note or f"Recording stopped: the share refused the write ({e.strerror})"
                    self.failed = True
                finally:
                    with self.lock:
                        self.queued -= len(data)
        finally:
            if out is not None:
                out.close()
                os.replace(partial, self.target)
                log("recorded", self.name, f"{self.seconds:.0f} s", self.bytes, "bytes", self.note)

    def close(self, note=""):
        if not self.closed:
            self.closed = True
            self.note = self.note or note
            self.queue.put(None)


class Session:
    def __init__(self, receiver, ident, replay, reach):
        self.receiver, self.id = receiver, ident
        self.replay, self.reach = replay, reach
        self.opened = time.monotonic()
        self.live = False
        self.ended = False
        self.pending = bytearray()
        self.init = bytearray()
        self.scales = {}
        self.moof = None
        self.need_key = True
        self.chunks = []
        self.held = []
        self.live_bytes = 0
        self.next_chunk = 0
        self.recording = None
        self.record_wanted = None  # (name, max seconds) until the keyframe it starts on
        self.last_recording = {}
        self.saving = False
        self.fragments = self.dropped = 0

    # The stream.

    def feed(self, data):
        """Takes the next bytes of the upload and files every whole box in them."""
        self.pending += data
        at = 0
        view = self.pending
        while len(view) - at >= 8:
            size, kind = struct.unpack_from(">I4s", view, at)
            if size == 1:
                if len(view) - at < 16:
                    break
                size = struct.unpack_from(">Q", view, at + 8)[0]
            if size < 8 or size > self.receiver.max_box:
                raise Refused(400, "The upload is not the stream this receiver takes")
            if len(view) - at < size:
                break
            self.box(kind, bytes(view[at:at + size]))
            at += size
        del self.pending[:at]

    def box(self, kind, data):
        if kind in (b"ftyp", b"moov"):
            self.init += data
            if kind == b"moov":
                self.scales = timescales(bytes(self.init))
                if not any(video for _, video in self.scales.values()):
                    raise Refused(400, "The upload has no video track")
        elif kind == b"moof":
            self.moof = data
        elif kind == b"mdat" and self.moof is not None and self.scales:
            fragment, self.moof = self.moof + data, None
            with self.receiver.lock:
                if not self.ended:
                    self.fragment(fragment)

    def fragment(self, data):
        moof = Moof(data, self.scales)
        if moof.video_base is None:
            return
        self.fragments += 1
        if self.replay:
            self.keep(data, moof)
        self.record(data, moof)

    # The rolling buffer.

    def scale(self):
        return next(s for s, video in self.scales.values() if video)

    def used(self):
        return self.live_bytes + self.receiver.zombie_bytes

    def keep(self, data, moof):
        if self.need_key and not moof.key:
            return
        r = self.receiver
        if not self.chunks or self.chunks[-1].size >= r.chunk_bytes:
            try:
                self.chunks.append(Chunk(os.path.join(r.buffer, f"replay-{self.next_chunk:08d}.chunk")))
            except OSError as e:
                return self.lose(f"the buffer folder refused a file ({e.strerror})")
            self.next_chunk += 1
        self.trim(moof)
        while self.used() + len(data) > r.cap and len(self.chunks) > 1 and not self.holds_newest_key(self.chunks[0]):
            self.evict()
        if self.used() + len(data) > r.cap:
            return self.lose("one fragment does not fit under the cap")
        chunk = self.chunks[-1]
        try:
            os.pwrite(chunk.fd, data, chunk.size)
        except OSError as e:
            return self.lose(f"the buffer refused the write ({e.strerror})")
        self.held.append(Held(chunk, chunk.size, len(data), moof.video_base, moof.video_ticks, moof.key))
        chunk.size += len(data)
        self.live_bytes += len(data)
        self.need_key = False

    def trim(self, moof):
        """Drops the chunks older than the keyframe a cut of the full reach would start on."""
        target = moof.video_base - self.reach * self.scale()
        start = next((h for h in reversed(self.held) if h.key and h.base <= target), None)
        while start is not None and len(self.chunks) > 1 and self.chunks[0] is not start.chunk:
            self.evict()

    def holds_newest_key(self, chunk):
        """The chunk with the newest keyframe: dropping it leaves frames nothing can decode, so the cap never takes it."""
        newest = next((h for h in reversed(self.held) if h.key), None)
        return newest is not None and newest.chunk is chunk

    def evict(self):
        chunk = self.chunks.pop(0)
        self.held = [h for h in self.held if h.chunk is not chunk]
        self.live_bytes -= chunk.size
        self.receiver.release(chunk)

    def lose(self, why):
        """The buffer cannot take the stream as it comes: it starts over at the next keyframe."""
        self.dropped += 1
        self.receiver.note_drop(why)
        self.empty()

    def empty(self):
        while self.chunks:
            self.evict()
        self.need_key = True

    def held_seconds(self):
        keys = [h for h in self.held if h.key]
        if not keys:
            return 0.0
        last = self.held[-1]
        return (last.base + last.ticks - keys[0].base) / self.scale()

    def save(self, seconds, game):
        r = self.receiver
        if not self.replay:
            raise Refused(409, "Long replay is off for this stream")
        if self.saving:
            raise Refused(409, "A clip is still being written")
        keys = [i for i, h in enumerate(self.held) if h.key]
        if not keys:
            raise Refused(409, "The buffer holds no keyframe yet")
        if r.free(r.recordings) < r.record_floor:
            raise Refused(507, "The recordings share is nearly full")
        last = self.held[-1]
        target = last.base + last.ticks - seconds * self.scale()
        first = next((i for i in reversed(keys) if self.held[i].base <= target), keys[0])
        cut = self.held[first:]
        for chunk in {id(h.chunk): h.chunk for h in cut}.values():
            chunk.pins += 1
        name = r.new_name(game)
        length = (last.base + last.ticks - cut[0].base) / self.scale()
        size = len(self.init) + sum(h.size for h in cut)
        self.saving = True
        threading.Thread(target=self.copy, args=(name, bytes(self.init), cut, dict(self.scales)), name="save", daemon=True).start()
        return {"name": name, "seconds": round(length), "bytes": size}

    def copy(self, name, init, cut, scales):
        r = self.receiver
        target = os.path.join(r.recordings, name)
        partial = target + ".part"
        try:
            os.makedirs(os.path.dirname(target), exist_ok=True)
            with open(partial, "wb") as out:
                out.write(init)
                for sequence, h in enumerate(cut, 1):
                    data = os.pread(h.chunk.fd, h.size, h.offset)
                    out.write(rebased(data, Moof(data, scales), sequence, cut[0].base, scales))
            os.replace(partial, target)
            log("saved", name, sum(h.size for h in cut), "bytes")
        except OSError as e:
            log("save failed", name, e)
            try:
                os.remove(partial)
            except OSError:
                pass
        finally:
            with r.lock:
                self.saving = False
                for chunk in {id(h.chunk): h.chunk for h in cut}.values():
                    chunk.pins -= 1
                    r.release(chunk, unpin=True)

    # The recording.

    def record_start(self, game, max_seconds):
        r = self.receiver
        if self.recording or self.record_wanted:
            raise Refused(409, "A recording is already running")
        if not self.live:
            raise Refused(409, "The stream is not reaching the NAS")
        if r.free(r.recordings) < r.record_floor:
            raise Refused(507, "The recordings share is nearly full")
        name = r.new_name(game)
        self.record_wanted = (name, max(1, min(max_seconds or r.record_max, r.record_max)))
        self.last_recording = {}
        return {"name": name, "max": self.record_wanted[1]}

    def record(self, data, moof):
        if self.record_wanted and moof.key:
            self.recording = Recording(self.receiver, *self.record_wanted)
            self.record_wanted = None
            self.recording.origin = moof.video_base
            self.recording.put(bytes(self.init))
        rec = self.recording
        if rec is None:
            return
        if rec.failed:
            return self.record_stop(rec.note)
        if rec.queued + len(data) > self.receiver.record_queue:
            return self.record_stop("Recording stopped: the share could not keep up")
        rec.sequence += 1
        rec.put(bytes(rebased(data, moof, rec.sequence, rec.origin, self.scales)))
        rec.seconds = (moof.video_base + moof.video_ticks - rec.origin) / self.scale()
        rec.bytes += len(data) + (len(self.init) if rec.sequence == 1 else 0)
        if rec.seconds >= rec.max_seconds:
            self.record_stop(f"Recording stopped at its longest length, {rec.max_seconds // 60} min")

    def record_stop(self, note=""):
        rec, wanted = self.recording, self.record_wanted
        self.recording = self.record_wanted = None
        if rec is None:
            if wanted:
                self.last_recording = {"name": "", "seconds": 0, "bytes": 0, "note": note or "No keyframe arrived, so nothing was recorded"}
            return self.last_recording
        rec.close(note)
        self.last_recording = {"name": rec.name, "seconds": round(rec.seconds), "bytes": rec.bytes, "note": rec.note}
        return self.last_recording

    def end(self, why):
        if self.ended:
            return
        self.ended = True
        self.live = False
        self.record_stop("Recording stopped: the stream ended")
        self.empty()
        log("stream", self.id, "ended:", why, f"({self.fragments} fragments, {self.dropped} buffer restarts)")

    def status(self):
        rec = self.recording
        out = {
            "live": int(self.live),
            "replay": int(self.replay),
            "reach": self.reach,
            "held": int(self.held_seconds()),
            "recording": int(bool(rec or self.record_wanted)),
            "record_seconds": int(rec.seconds) if rec else 0,
        }
        if not out["recording"] and self.last_recording:
            out.update({f"last_{k}": v for k, v in self.last_recording.items()})
        return out


class Receiver:
    def __init__(self, buffer, recordings, cap=6 * GB, chunk_bytes=32 << 20, record_max=7200, record_floor=100 * GB, live_timeout=15.0):
        self.buffer, self.recordings = buffer, recordings
        self.cap, self.chunk_bytes = cap, chunk_bytes
        self.record_max, self.record_floor = record_max, record_floor
        self.record_queue = 128 << 20
        self.max_box = 64 << 20
        self.max_clip = 192 << 20
        self.live_timeout = live_timeout
        self.lock = threading.RLock()
        self.session = None
        self.zombie_bytes = 0
        self.drops = 0
        self.last_drop = ""
        self.last_drop_logged = 0.0
        self.names = set()
        os.makedirs(buffer, exist_ok=True)
        os.makedirs(recordings, exist_ok=True)
        self.clean()

    def clean(self):
        """At start: the buffer of whatever ran before goes, and a file a crash left unfinished keeps what it has under its name."""
        for name in os.listdir(self.buffer):
            if name.startswith("replay-") and name.endswith(".chunk"):
                os.remove(os.path.join(self.buffer, name))
        for folder, _, names in os.walk(self.recordings):
            for name in names:
                if name.endswith(".mp4.part"):
                    path = os.path.join(folder, name)
                    if os.path.getsize(path):
                        os.replace(path, path[:-5])
                    else:
                        os.remove(path)

    def free(self, path):
        s = os.statvfs(path)
        return s.f_bavail * s.f_frsize

    def release(self, chunk, unpin=False):
        """A chunk left the buffer, or a clip let go of it: its file goes once nothing reads it, and counts under the cap until then."""
        if not unpin:
            chunk.gone = True
            try:
                os.remove(chunk.path)
            except OSError:
                pass
            if chunk.pins:
                self.zombie_bytes += chunk.size
        if chunk.gone and chunk.pins == 0 and chunk.fd is not None:
            os.close(chunk.fd)
            chunk.fd = None
            if unpin:
                self.zombie_bytes -= chunk.size

    def note_drop(self, why):
        self.drops += 1
        self.last_drop = why
        if time.monotonic() - self.last_drop_logged > 60:
            self.last_drop_logged = time.monotonic()
            log("buffer restarted:", why, f"({self.drops} so far)")

    def new_name(self, game):
        folder = folder_name(game)
        stamp = time.strftime("%Y-%m-%d %H.%M.%S")
        name, n = f"{folder}/{stamp}.mp4", 1
        while name in self.names or os.path.exists(os.path.join(self.recordings, name)) or os.path.exists(os.path.join(self.recordings, name + ".part")):
            n += 1
            name = f"{folder}/{stamp} ({n}).mp4"
        self.names = {x for x in self.names if x.split("/")[1].startswith(stamp)} | {name}
        return name

    def grant(self, reach, mbps):
        """The reach the cap can hold at this bitrate, and a sentence when that is less than asked."""
        reach = max(0, reach)
        if mbps <= 0 or reach == 0:
            return reach, ""
        fits = int((self.cap - self.chunk_bytes) / (mbps * 125_000)) // 10 * 10
        if fits >= reach:
            return reach, ""
        size = f"{self.cap / GB:.1f}".rstrip("0").rstrip(".")
        return fits, f"The NAS buffer is {size} GB: at {mbps} Mbit/s it holds {fits // 60} min {fits % 60} s, not {reach // 60} min"

    def open(self, ident, replay, reach, mbps):
        if not ident:
            raise Refused(400, "The stream has no name")
        with self.lock:
            if self.session is not None:
                self.session.end("another stream opened")
            granted, note = self.grant(reach, mbps) if replay else (0, "")
            self.session = Session(self, ident, replay and granted > 0, granted)
            log("stream", ident, "opened:", f"replay {int(replay)}, reach {granted} s of {reach} s asked, {mbps} Mbit/s")
            return {"reach": granted, "asked": reach, "cap": self.cap, "note": note}

    def find(self, ident):
        s = self.session
        if s is None or s.id != ident or s.ended:
            raise Refused(409, "The NAS holds no such stream")
        return s

    def set_replay(self, ident, on, reach, mbps):
        with self.lock:
            s = self.find(ident)
            granted, note = self.grant(reach, mbps) if on else (0, "")
            s.replay, s.reach = on and granted > 0, granted
            if not s.replay:
                s.empty()
            return {"reach": granted, "asked": reach, "note": note}

    def sweep(self):
        """Ends a stream that was opened and never sent anything."""
        with self.lock:
            s = self.session
            if s is not None and not s.live and not s.ended and time.monotonic() - s.opened > self.live_timeout:
                s.end("no upload arrived")

    def health(self):
        with self.lock:
            s = self.session if self.session is not None and not self.session.ended else None
            return {
                "ok": True,
                "version": VERSION,
                "cap_bytes": self.cap,
                "buffer_bytes": (s.used() if s else self.zombie_bytes),
                "buffer_free_bytes": self.free(self.buffer),
                "recordings_free_bytes": self.free(self.recordings),
                "record_floor_bytes": self.record_floor,
                "record_max_seconds": self.record_max,
                "buffer_restarts": self.drops,
                "last_restart": self.last_drop,
                "stream": dict(s.status(), session=s.id) if s else None,
            }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    receiver = None
    token = ""

    def log_message(self, *_):
        pass

    def answer(self, status, fields=None, body=None, kind="text/plain; charset=utf-8"):
        if body is None:
            body = "".join(f"{k}={str(v).replace(chr(10), ' ')}\n" for k, v in (fields or {}).items()).encode()
        self.close_connection = True
        try:
            self.send_response(status)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
        except OSError:
            pass

    def parts(self):
        url = urlsplit(self.path)
        q = {k: v[0] for k, v in parse_qs(url.query).items()}
        return url.path.rstrip("/") or "/", q

    def allowed(self):
        return not self.token or self.headers.get("Authorization", "") == f"Bearer {self.token}"

    def number(self, q, key, default=0):
        try:
            return int(q.get(key, default))
        except ValueError:
            raise Refused(400, f"{key} is not a number") from None

    def handle_one(self, routes):
        path, q = self.parts()
        try:
            if path not in ("/health", "/icon.png") and not self.allowed():
                raise Refused(401, "The receiver wants its token")
            route = routes.get(path)
            if route is None:
                raise Refused(404, "No such request")
            self.receiver.sweep()
            result = route(q)
            if result is not None:
                self.answer(200, result)
        except Refused as e:
            self.answer(e.status, {"note": e.note})
        except (OSError, ValueError) as e:
            self.answer(500, {"note": f"The receiver failed: {e}"})

    def do_GET(self):
        r = self.receiver
        self.handle_one({
            "/health": lambda q: self.answer(200, body=json.dumps(r.health()).encode(), kind="application/json"),
            "/icon.png": lambda q: self.answer(200, body=icon(), kind="image/png"),
            "/": lambda q: self.answer(200, body=(json.dumps(r.health(), indent=2) + "\n").encode()),
            "/status": lambda q: self.locked(lambda: r.find(q.get("session", "")).status()),
        })

    def locked(self, call):
        with self.receiver.lock:
            return call()

    def do_POST(self):
        r = self.receiver
        on = lambda q, key: q.get(key, "0") == "1"
        self.handle_one({
            "/open": lambda q: r.open(q.get("session", ""), on(q, "replay"), self.number(q, "reach"), self.number(q, "mbps")),
            "/replay": lambda q: r.set_replay(q.get("session", ""), on(q, "on"), self.number(q, "reach"), self.number(q, "mbps")),
            "/save": lambda q: self.locked(lambda: r.find(q.get("session", "")).save(max(1, self.number(q, "seconds", 300)), q.get("game", ""))),
            "/record/start": lambda q: self.locked(lambda: r.find(q.get("session", "")).record_start(q.get("game", ""), self.number(q, "max"))),
            "/record/stop": lambda q: self.locked(lambda: r.find(q.get("session", "")).record_stop() or {"name": "", "seconds": 0, "bytes": 0}),
            "/status": lambda q: self.locked(lambda: r.find(q.get("session", "")).status()),
        })

    def do_PUT(self):
        self.handle_one({"/live": self.live, "/clip": self.clip})

    def body(self):
        """Yields the request body as it arrives."""
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            while True:
                size = int(self.rfile.readline(64).split(b";")[0].strip() or b"0", 16)
                if size == 0:
                    self.rfile.readline(64)
                    return
                while size:
                    block = self.rfile.read(min(size, 1 << 20))
                    if not block:
                        raise ConnectionError("the upload was cut")
                    size -= len(block)
                    yield block
                self.rfile.readline(64)
        else:
            left = int(self.headers.get("Content-Length", "0"))
            while left:
                block = self.rfile.read(min(left, 1 << 20))
                if not block:
                    raise ConnectionError("the upload was cut")
                left -= len(block)
                yield block

    def live(self, q):
        r = self.receiver
        with r.lock:
            s = r.find(q.get("session", ""))
            if s.live:
                raise Refused(409, "This stream is already uploading")
            s.live = True
        self.connection.settimeout(r.live_timeout)
        why = "the upload ended"
        try:
            for block in self.body():
                if s.ended:
                    why = "ended"
                    break
                s.feed(block)
        except Refused as e:
            why = e.note
        except (OSError, ValueError) as e:
            why = f"the upload stopped ({e})"
        with r.lock:
            s.end(why)
        return {"fragments": s.fragments}

    def clip(self, q):
        r = self.receiver
        if r.free(r.recordings) < r.record_floor:
            raise Refused(507, "The recordings share is nearly full")
        data = bytearray()
        for block in self.body():
            data += block
            if len(data) > r.max_clip:
                raise Refused(413, "The clip is larger than a clip from the TV's memory can be")
        with r.lock:
            name = r.new_name(q.get("game", ""))
        target = os.path.join(r.recordings, name)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target + ".part", "wb") as out:
            out.write(data)
        os.replace(target + ".part", target)
        log("clip", name, len(data), "bytes")
        return {"name": name, "bytes": len(data)}


def serve(receiver, port, token=""):
    handler = type("Bound", (Handler,), {"receiver": receiver, "token": token})
    server = ThreadingHTTPServer(("", port), handler)
    server.daemon_threads = True
    return server


def main():
    env = os.environ
    gb = lambda key, default: int(float(env.get(key, default)) * GB)
    receiver = Receiver(
        env.get("BUFFER_DIR", "/buffer"),
        env.get("RECORDINGS_DIR", "/recordings"),
        cap=gb("BUFFER_CAP_GB", "6"),
        record_max=int(float(env.get("RECORD_MAX_MINUTES", "120")) * 60),
        record_floor=gb("RECORD_FLOOR_GB", "100"),
        live_timeout=float(env.get("LIVE_TIMEOUT_S", "15")),
    )
    port = int(env.get("PORT", "8765"))
    log(f"lumagame-replay {VERSION} on :{port}: buffer {receiver.buffer} capped at {receiver.cap / GB:.1f} GB, recordings in {receiver.recordings}")
    serve(receiver, port, env.get("TOKEN", "")).serve_forever()


if __name__ == "__main__":
    sys.exit(main())
