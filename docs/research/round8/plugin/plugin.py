#!/usr/bin/env python3
"""The Round 8 test plugin: one card, one tile and one notification through LumaShow's plugin API level 1 (the helper's broker),
so that Home's At-home lane, the quick panel's plugin tiles and the notification drawer have something of a plugin to draw.
Stdlib only (the set has python3); runs on the set as the plugin's own user.

    plugin.py [--socket PATH] [--ctl FILE] [--log FILE] [--user UID --group GID] [--wait SECS]

Identity is the connection: the broker reads the peer's user id (SO_PEERCRED) and looks it up in the roster, so the process drops to
`--user` (with `--group`, the broker's socket is 0660 for group 2000) before it connects. Nothing is read from the network.

It says hello, registers a tile, a channel and an action, puts the card `r8-card` and the note `r8-note-1`, and then answers
`action.invoke` for `ping` (the tile flips on and off, the card's fact counts the pings). `--ctl` is a file the harness appends
commands to, one per line: `note N` posts `r8-note-N`, `end N` ends it, `quit` says bye and exits. A `shutdown` from the broker
ends it the same way. Every message sent and received is one line of `--log`.
"""
import argparse
import json
import os
import socket
import struct
import sys
import time

MAX_FRAME = 65536
SOCKET = "/run/lumashow-plugins/broker.sock"

REGISTER = {
    "t": "register", "title": "Round 8 check",
    "actions": [{"key": "ping", "label": "Ping"}],
    "views": [{"id": "main", "title": "Round 8 check", "presentation": "sheet"}],
    "channels": [{"id": "r8", "title": "Round 8", "icon": "base.icons/message", "importance": "normal"}],
    "tiles": [{"id": "r8-tile", "label": "Round 8", "icon": "base.icons/message", "state": {"type": "off"}, "action": "ping"}],
}


def card(pings):
    return {"t": "card.put", "id": "r8-card", "size": "half", "priority": "new",
            "content": {"type": "facts", "title": "Round 8 check", "icon": "base.icons/message", "role": "accent",
                        "facts": [{"label": "Plugin card", "value": "drawn on Home"}, {"label": "Pings", "value": str(pings)}]},
            "open": {"view": "main"}, "actions": [{"action": "ping", "label": "Ping", "icon": "base.icons/message"}]}


def tile(on):
    return {"t": "tile.put", "id": "r8-tile", "label": "Round 8", "icon": "base.icons/message",
            "state": {"type": "on" if on else "off"}, "action": "ping"}


def note(n):
    return {"t": "notify.post", "id": "r8-note-%d" % n, "channel": "r8", "title": "Round 8 note %d" % n,
            "body": "A plugin note for the notification drawer.", "expires_in_secs": 900}


def encode(message):
    body = json.dumps(message, separators=(",", ":")).encode()
    if not 0 < len(body) <= MAX_FRAME:
        raise ValueError("a frame of %d bytes" % len(body))
    return struct.pack("<I", len(body)) + body


class Reader:
    """Whole frames out of a stream that arrives in pieces."""

    def __init__(self):
        self.buf = b""

    def feed(self, data):
        self.buf += data
        out = []
        while len(self.buf) >= 4:
            n = struct.unpack("<I", self.buf[:4])[0]
            if n == 0 or n > MAX_FRAME:
                raise ValueError("a frame of %d bytes" % n)
            if len(self.buf) < 4 + n:
                break
            out.append(json.loads(self.buf[4:4 + n].decode("utf-8")))
            self.buf = self.buf[4 + n:]
        return out


class Plugin:
    def __init__(self, path, ctl, log, wait):
        self.path, self.ctl, self.log_path, self.wait = path, ctl, log, wait
        self.sock = None
        self.reader = Reader()
        self.inbox = []
        self.pings = 0
        self.on = False
        self.ctl_lines = 0

    def log(self, line):
        with open(self.log_path, "a") as f:
            f.write("%.3f %s\n" % (time.time(), line))

    def send(self, message):
        self.log("> " + json.dumps(message, separators=(",", ":")))
        self.sock.sendall(encode(message))

    def recv(self, timeout):
        """The next message from the broker, or None after `timeout` seconds."""
        end = time.time() + timeout
        while not self.inbox:
            left = end - time.time()
            if left <= 0:
                return None
            self.sock.settimeout(left)
            try:
                data = self.sock.recv(65536)
            except socket.timeout:
                return None
            if not data:
                raise ConnectionError("the broker closed the connection")
            self.inbox.extend(self.reader.feed(data))
        message = self.inbox.pop(0)
        self.log("< " + json.dumps(message, separators=(",", ":")))
        return message

    def connect(self):
        """A plugin that was switched on a moment ago may be told `not_a_plugin` (not yet): try again until `wait` has passed."""
        end = time.time() + self.wait
        while True:
            self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                self.sock.connect(self.path)
                self.reader, self.inbox = Reader(), []
                self.send({"t": "hello", "min": 1, "target": 1, "sdk": "round8/1"})
                reply = self.recv(10)
                if reply and reply.get("t") == "welcome":
                    return reply
                reason = (reply or {}).get("code", "no answer")
            except (OSError, ConnectionError) as e:
                reason = str(e)
            self.sock.close()
            if time.time() >= end:
                raise SystemExit("no welcome from %s: %s" % (self.path, reason))
            time.sleep(2)

    def commands(self):
        try:
            with open(self.ctl) as f:
                lines = f.read().splitlines()
        except OSError:
            return []
        new, self.ctl_lines = lines[self.ctl_lines:], len(lines)
        return [l.split() for l in new if l.strip()]

    def handle(self, message):
        kind = message.get("t")
        if kind == "action.invoke":
            if message.get("action") == "ping":
                self.pings += 1
                self.on = not self.on
                self.log("action ping %d" % self.pings)
                self.send({"t": "action.result", "op": message["op"], "outcome": {"type": "ok"}})
                self.send(tile(self.on))
                self.send(card(self.pings))
            else:
                self.send({"t": "action.result", "op": message["op"], "outcome": {"type": "failed", "reason": "unknown action"}})
            return True
        if kind == "shutdown":
            self.log("shutdown %s" % message.get("reason"))
            return False
        if kind in ("refused", "error"):
            self.log("refused %s" % json.dumps(message, separators=(",", ":")))
        return True

    def run(self):
        self.connect()
        self.send(REGISTER)
        reply = self.recv(10)
        if not reply or reply.get("t") != "registered":
            raise SystemExit("register answered %r" % (reply,))
        self.send(card(0))
        self.send(note(1))
        self.log("ready")
        alive = True
        while alive:
            message = self.recv(0.5)
            if message is not None:
                alive = self.handle(message)
            for cmd in self.commands():
                if cmd[0] == "note" and len(cmd) == 2 and cmd[1].isdigit():
                    self.send(note(int(cmd[1])))
                elif cmd[0] == "end" and len(cmd) == 2 and cmd[1].isdigit():
                    self.send({"t": "notify.end", "id": "r8-note-%s" % cmd[1]})
                elif cmd[0] == "quit":
                    alive = False
        self.send({"t": "bye", "reason": "done"})
        self.sock.close()
        self.log("bye")


def drop(uid, gid):
    os.setgroups([gid])
    os.setgid(gid)
    os.setuid(uid)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--socket", default=SOCKET)
    ap.add_argument("--ctl", default="/tmp/round8/plugin.cmd")
    ap.add_argument("--log", default="/tmp/round8/plugin.log")
    ap.add_argument("--user", type=int, help="drop to this user id before connecting (the roster's uid for this plugin)")
    ap.add_argument("--group", type=int, default=2000, help="and this group (the broker's socket is 0660 for it)")
    ap.add_argument("--wait", type=int, default=40, help="seconds to keep trying for a welcome")
    a = ap.parse_args(argv)
    if a.user is not None:
        drop(a.user, a.group)
    Plugin(a.socket, a.ctl, a.log, a.wait).run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
