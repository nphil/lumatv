#!/usr/bin/env python3
"""Shell diagnostics use the sole RAM log collector, never a retained producer file."""
import collections
import os
import selectors
import signal
import socket
import subprocess
import sys

SOCKET = "/tmp/lumatv-log.sock"
DIRECTORY = "/tmp/lumatv-logs"
MAX_RECORD = 4096


def send(sock, tag, message, severity=6):
    prefix = ("<%d>%s: " % (128 + severity, tag)).encode("ascii")
    message = "".join(" " if ord(c) < 32 or ord(c) == 127 else c for c in message)
    payload = message.encode("utf-8", "replace")
    room = MAX_RECORD - len(prefix)
    if len(payload) > room:
        payload = payload[:room - 3].decode("utf-8", "ignore").encode("utf-8") + b"..."
    if sock is not None:
        try:
            sock.sendto(prefix + payload, SOCKET)
        except OSError:
            pass


def new_socket():
    try:
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
        sock.setblocking(False)
        return sock
    except OSError:
        return None


def read_lines(tag, count, directory=DIRECTORY):
    lines = collections.deque(maxlen=max(0, count))
    for name in ("previous.log", "current.log"):
        try:
            with open(os.path.join(directory, name), "rb") as source:
                for line in source.read(524288).splitlines():
                    pri, sep, message = line.partition(b">")
                    if not sep or not pri.startswith(b"<") or not pri[1:].isdigit() or not 0 <= int(pri[1:]) <= 191:
                        continue
                    prefix = (tag + ": ").encode("ascii")
                    if message.startswith(prefix):
                        lines.append(message[len(prefix):].decode("utf-8", "replace"))
        except OSError:
            pass
    return list(lines)


def run(tag, command, both=False):
    sock = new_socket()
    try:
        child = subprocess.Popen(command, stderr=subprocess.STDOUT if both else subprocess.PIPE, stdout=subprocess.PIPE if both else None, close_fds=False)
    except OSError as error:
        send(sock, tag, str(error), 3)
        return 127
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, lambda number, _frame: child.send_signal(number) if child.poll() is None else None)
    pending = bytearray()
    cut = False
    stream = child.stdout if both else child.stderr
    os.set_blocking(stream.fileno(), False)
    selector = selectors.DefaultSelector()
    selector.register(stream, selectors.EVENT_READ)
    while True:
        exited = child.poll() is not None
        events = selector.select(0 if exited else 0.1)
        if not events and exited:
            break
        for key, _ in events:
            chunk = os.read(key.fd, 8192)
            if not chunk:
                selector.unregister(key.fd)
                break
            for byte in chunk:
                if byte == 10:
                    send(sock, tag, pending.decode("utf-8", "replace") + ("..." if cut else ""), 3)
                    pending.clear()
                    cut = False
                elif len(pending) < MAX_RECORD:
                    pending.append(byte)
                else:
                    cut = True
        if not selector.get_map():
            child.wait()
            break
    if pending or cut:
        send(sock, tag, pending.decode("utf-8", "replace") + ("..." if cut else ""), 3)
    stream.close()
    selector.close()
    if sock is not None:
        sock.close()
    code = child.wait()
    return code if code >= 0 else 128 - code


def main():
    mode, tag, *args = sys.argv[1:]
    if mode == "send":
        sock = new_socket()
        send(sock, tag, " ".join(args))
        if sock is not None:
            sock.close()
        return 0
    if mode == "read":
        for line in read_lines(tag, int(args[0]) if args else 20):
            print(line)
        return 0
    if mode in ("stderr", "output"):
        return run(tag, args, mode == "output")
    raise SystemExit("logrelay.py: send|read|stderr|output TAG [arguments]")


if __name__ == "__main__":
    sys.exit(main())
