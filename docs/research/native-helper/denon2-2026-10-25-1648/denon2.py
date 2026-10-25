#!/usr/bin/env python3
"""Second Denon control connection, held from the TV beside Home Assistant's: read queries only.
Prints one line per event: t=<s since start> rx|tx <text>. Ends after SECS seconds."""
import socket, sys, time
HOST, PORT = "192.168.1.93", 23
secs = int(sys.argv[1]) if len(sys.argv) > 1 else 60
t0 = time.time()
def log(kind, text):
    print("t=%6.2f %s %s" % (time.time() - t0, kind, text), flush=True)
try:
    s = socket.create_connection((HOST, PORT), timeout=5)
except Exception as e:
    log("err", "connect failed: %r" % (e,))
    sys.exit(2)
log("ok", "connected from %s:%d" % s.getsockname())
s.settimeout(0.5)
queries = ["PW?", "MV?", "MU?", "SI?"]
next_q, qi = 0.0, 0
buf = b""
while time.time() - t0 < secs:
    now = time.time() - t0
    if now >= next_q:
        q = queries[qi % len(queries)]
        try:
            s.sendall((q + "\r").encode())
            log("tx", q)
        except Exception as e:
            log("err", "send failed: %r" % (e,)); break
        qi += 1
        next_q = now + 10
    try:
        d = s.recv(4096)
        if not d:
            log("err", "peer closed the connection"); break
        buf += d
        while b"\r" in buf:
            line, buf = buf.split(b"\r", 1)
            log("rx", line.decode("ascii", "replace"))
    except socket.timeout:
        pass
    except Exception as e:
        log("err", "recv failed: %r" % (e,)); break
try:
    s.close()
except Exception:
    pass
log("ok", "closed after %d queries" % qi)
