#!/usr/bin/env python3
"""Receiver for LumaGame's game recorder: stores the clips the TV sends with HTTP PUT.

    python3 lumagame-clip-receiver.py /path/to/clips [port]

The recorder sends `PUT /<folder>/<name>.mp4` with a chunked body. Plain HTTP and no login: run it on the LAN only. Only plain
file names under the root are accepted; anything else is refused.
"""
import os
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

NAME = re.compile(r"^[A-Za-z0-9._-]{1,128}$")


class Receiver(BaseHTTPRequestHandler):
    root = "."

    def do_PUT(self):
        parts = [p for p in self.path.split("?")[0].split("/") if p]
        if not parts or len(parts) > 2 or not all(NAME.match(p) and p not in (".", "..") for p in parts):
            return self.reply(400)
        folder = os.path.join(self.root, *parts[:-1])
        os.makedirs(folder, exist_ok=True)
        target = os.path.join(folder, parts[-1])
        partial = target + ".part"
        try:
            with open(partial, "wb") as out:
                if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
                    while True:
                        size = int(self.rfile.readline().split(b";")[0].strip() or b"0", 16)
                        if size == 0:
                            self.rfile.readline()
                            break
                        out.write(self.rfile.read(size))
                        self.rfile.readline()
                else:
                    left = int(self.headers.get("Content-Length", "0"))
                    while left:
                        block = self.rfile.read(min(left, 1 << 20))
                        if not block:
                            raise ConnectionError("short body")
                        out.write(block)
                        left -= len(block)
            os.replace(partial, target)
        except (ValueError, OSError):
            if os.path.exists(partial):
                os.remove(partial)
            return self.reply(500)
        self.reply(201)

    def reply(self, code):
        self.send_response(code)
        self.send_header("Content-Length", "0")
        self.end_headers()


if __name__ == "__main__":
    Receiver.root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else ".")
    ThreadingHTTPServer(("", int(sys.argv[2]) if len(sys.argv) > 2 else 8765), Receiver).serve_forever()
