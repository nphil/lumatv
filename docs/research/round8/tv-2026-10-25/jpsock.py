"""On the set: start lumatv-jpeg as the helper does (socket as fd 0) and ask for one decode; print the answer's status words."""
import os, socket, struct, subprocess, sys, time
exe = sys.argv[1]; path = sys.argv[2]; shift = int(sys.argv[3]) if len(sys.argv) > 3 else 0
a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
child = subprocess.Popen([exe, "-f", "0"], stdin=b.fileno(), stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, close_fds=True)
b.close()
jpeg = open(path, "rb").read()
req = struct.pack("<II4BIIIII", 0x3151504a, 1, 1, 3, shift, 0, 0, len(jpeg), 0, 0, 0)
a.sendall(req + jpeg)
hdr = b""
while len(hdr) < 64:
    chunk = a.recv(64 - len(hdr))
    if not chunk:
        break
    hdr += chunk
w = struct.unpack("<16I", hdr)
names = "magic id status width height stride fmt data_len decode_us convert_us total_us hw_pixfmt hw_width hw_height hw_bytes flags".split()
print(dict(zip(names, w)))
print("cpu" if w[15] & 1 else "hardware", "hw status bits", (w[15] >> 8) & 0xFF)
left = w[7]
while left > 0:
    d = a.recv(min(65536, left))
    if not d:
        break
    left -= len(d)
a.close()
try:
    child.wait(timeout=5)
except Exception:
    child.kill()
print("stderr:", child.stderr.read().decode(errors="replace")[-600:])
