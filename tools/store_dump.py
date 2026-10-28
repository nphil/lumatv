#!/usr/bin/env python3
"""A read-only inventory of a page WAM is showing, for the Content Store theme: its markup's class names, its stylesheets and effects, its
pictures and the time it took to come up. Python 3 standard library only; runs on the set (python 3.10) and on the workstation.

usage: store_dump.py [--port 9998] [--match TEXT] [--out FILE] [--list]

WAM starts with --remote-debugging-port=9998 (webapp-mgr.sh), so on the set the pages of every web app are listed at
http://127.0.0.1:9998/json/list when the inspector answers for them. The page is chosen by `--match` (a part of its url or title) or, with
none, as the one that is the Content Store's (its url holds lgwebostv.com or discovery) or the only one. One Runtime.evaluate asks the
page; nothing is clicked, set or loaded, and the answer holds structure and numbers and none of the page's words: class names, tags,
counts, stylesheet and picture hosts and paths without query strings, sizes, timings. Prints the JSON and writes it to --out.

Exit 0 with the inventory; 2 when no page was found or the choice is not one (the candidates are printed); 1 when the inspector does not answer.
"""
import base64
import hashlib
import json
import os
import socket
import struct
import sys
import urllib.request

# What the page is asked. Counts and names only; trimmed urls (host and the last part of the path, no query).
EXPRESSION = r"""
(async () => {
  const trim = u => { try { const x = new URL(u, location.href); return x.protocol === 'data:' ? 'data:' : x.host + x.pathname.slice(-48); } catch (e) { return ''; } };
  const out = { url: trim(location.href), protocol: location.protocol, readyState: document.readyState, viewport: [innerWidth, innerHeight], dpr: devicePixelRatio,
                marker: window.__lumatvStore || null };
  const nav = performance.getEntriesByType('navigation')[0];
  if (nav) out.navigation = { type: nav.type, domContentLoaded: Math.round(nav.domContentLoadedEventEnd), load: Math.round(nav.loadEventEnd), responseEnd: Math.round(nav.responseEnd), transferKB: Math.round(nav.transferSize / 1024) };
  out.paint = Object.fromEntries(performance.getEntriesByType('paint').map(p => [p.name, Math.round(p.startTime)]));
  try {
    const lcp = await new Promise(res => { const o = new PerformanceObserver(l => { res(l.getEntries().pop()); }); o.observe({ type: 'largest-contentful-paint', buffered: true }); setTimeout(() => res(null), 400); });
    if (lcp) out.lcp = { ms: Math.round(lcp.startTime), sizePx: lcp.size, element: lcp.element ? lcp.element.tagName.toLowerCase() : '' };
  } catch (e) { /* not supported */ }
  const res = performance.getEntriesByType('resource');
  const by = {};
  for (const r of res) { const t = r.initiatorType || 'other'; const b = by[t] || (by[t] = { count: 0, transferKB: 0, decodedKB: 0 }); b.count++; b.transferKB += r.transferSize / 1024; b.decodedKB += r.decodedBodySize / 1024; }
  for (const k in by) { by[k].transferKB = Math.round(by[k].transferKB); by[k].decodedKB = Math.round(by[k].decodedKB); }
  out.resources = { count: res.length, totalTransferKB: Math.round(res.reduce((a, r) => a + r.transferSize, 0) / 1024), byInitiator: by,
    largest: res.slice().sort((a, b) => b.transferSize - a.transferSize).slice(0, 10).map(r => ({ what: trim(r.name), type: r.initiatorType, transferKB: Math.round(r.transferSize / 1024), ms: Math.round(r.duration) })),
    slowest: res.slice().sort((a, b) => b.duration - a.duration).slice(0, 5).map(r => ({ what: trim(r.name), type: r.initiatorType, ms: Math.round(r.duration) })) };
  const imgs = [...document.images].filter(i => i.naturalWidth);
  const shown = i => { const r = i.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; };
  out.images = { count: document.images.length, decodedMB: Math.round(imgs.reduce((a, i) => a + i.naturalWidth * i.naturalHeight * 4, 0) / 1048576 * 10) / 10,
    lazy: imgs.filter(i => i.loading === 'lazy').length,
    oversized: imgs.map(i => ({ i, s: shown(i) })).filter(x => x.s[0] > 0 && x.i.naturalWidth > x.s[0] * 1.5).sort((a, b) => b.i.naturalWidth * b.i.naturalHeight - a.i.naturalWidth * a.i.naturalHeight).slice(0, 12)
      .map(x => ({ src: trim(x.i.currentSrc), natural: [x.i.naturalWidth, x.i.naturalHeight], shown: x.s, ratio: Math.round(x.i.naturalWidth / x.s[0] * 10) / 10 })) };
  const all = [...document.querySelectorAll('*')];
  const classes = {}, sand = {}, trans = {}, fonts = {};
  let depth = 0, shadows = 0, willChange = 0, backdrop = 0, transitions = 0, filters = 0;
  for (const e of all) {
    for (const c of (e.classList || [])) { const k = e.tagName.toLowerCase() + '.' + c; classes[k] = (classes[k] || 0) + 1; const m = c.match(/^enact_sandstone_([A-Za-z]+)_\1_([A-Za-z]+)__/); if (m) sand[m[1] + '_' + m[2]] = (sand[m[1] + '_' + m[2]] || 0) + 1; }
    let d = 0; for (let p = e; p; p = p.parentElement) d++; depth = Math.max(depth, d);
    const cs = getComputedStyle(e);
    if (cs.boxShadow !== 'none') shadows++;
    if (cs.willChange !== 'auto') willChange++;
    if (cs.backdropFilter && cs.backdropFilter !== 'none') backdrop++;
    if (cs.filter !== 'none') filters++;
    if (cs.transitionDuration !== '0s' && cs.transitionProperty !== 'none') { transitions++; trans[cs.transitionDuration] = (trans[cs.transitionDuration] || 0) + 1; }
    if (e.children.length === 0 && e.textContent && e.textContent.trim()) { const f = cs.fontFamily.split(',')[0].trim(); fonts[f] = (fonts[f] || 0) + 1; }
  }
  const top = o => Object.entries(o).sort((a, b) => b[1] - a[1]);
  out.dom = { elements: all.length, depth, classNames: Object.keys(classes).length, topClasses: top(classes).slice(0, 120), sandstone: top(sand).slice(0, 80) };
  out.effects = { elementsWithBoxShadow: shadows, elementsWithWillChange: willChange, elementsWithBackdropFilter: backdrop, elementsWithFilter: filters, elementsWithTransition: transitions, transitionDurations: top(trans).slice(0, 12) };
  out.styles = { sheets: [...document.styleSheets].map(s => ({ href: s.href ? trim(s.href) : 'inline', rules: (() => { try { return s.cssRules.length; } catch (e) { return 'cross-origin'; } })() })),
    adopted: document.adoptedStyleSheets ? document.adoptedStyleSheets.length : 0, bodyBackground: getComputedStyle(document.body).backgroundColor, htmlBackground: getComputedStyle(document.documentElement).backgroundColor,
    bodyFont: getComputedStyle(document.body).fontFamily, textFonts: top(fonts).slice(0, 8), fontFaces: [...document.fonts].map(f => f.family + ':' + f.status).slice(0, 12) };
  const f = document.activeElement;
  out.focus = f && f !== document.body ? { tag: f.tagName.toLowerCase(), classes: [...f.classList].slice(0, 4), outline: getComputedStyle(f).outline } : null;
  return out;
})()
"""


def http_json(port, path):
    with urllib.request.urlopen("http://127.0.0.1:%d%s" % (port, path), timeout=5) as r:
        return json.loads(r.read().decode("utf-8", "replace"))


class Socket:
    """The smallest WebSocket client the inspector needs: one text request at a time, masked frames out, any frames in."""

    def __init__(self, port, path):
        self.s = socket.create_connection(("127.0.0.1", port), timeout=20)
        key = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall(("GET %s HTTP/1.1\r\nHost: 127.0.0.1:%d\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: %s\r\n"
                        "Sec-WebSocket-Version: 13\r\n\r\n" % (path, port, key)).encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.s.recv(4096)
            if not chunk:
                raise OSError("the inspector closed the connection")
            head += chunk
        status, _, rest = head.partition(b"\r\n\r\n")
        if b" 101 " not in status.split(b"\r\n")[0]:
            raise OSError("no upgrade: " + status.split(b"\r\n")[0].decode("latin1"))
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest())
        if accept not in status:
            raise OSError("the upgrade answer is not for this key")
        self.buf = rest

    def read(self, n):
        while len(self.buf) < n:
            chunk = self.s.recv(65536)
            if not chunk:
                raise OSError("the inspector closed the connection")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, text):
        data = text.encode()
        n = len(data)
        head = bytearray([0x81])
        if n < 126:
            head.append(0x80 | n)
        elif n < 65536:
            head += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            head += bytes([0x80 | 127]) + struct.pack(">Q", n)
        mask = os.urandom(4)
        self.s.sendall(bytes(head) + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def recv(self):
        message = b""
        while True:
            b0, b1 = self.read(2)
            n = b1 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self.read(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self.read(8))[0]
            payload = self.read(n)
            op = b0 & 0x0F
            if op == 0x8:
                raise OSError("the inspector closed the connection")
            if op == 0x9:          # ping
                self.s.sendall(bytes([0x8A, 0x80]) + b"\0\0\0\0")
                continue
            if op in (0x1, 0x0, 0x2):
                message += payload
                if b0 & 0x80:
                    return message.decode("utf-8", "replace")

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


def choose(pages, match):
    """The one page asked for: by `match`, else the store's, else the only one. A list of candidates when it is not one."""
    pages = [p for p in pages if p.get("type") == "page"]
    if match:
        found = [p for p in pages if match in p.get("url", "") or match in p.get("title", "")]
    else:
        found = [p for p in pages if "lgwebostv.com" in p.get("url", "") or "discovery" in p.get("url", "")] or (pages if len(pages) == 1 else [])
    return found[0] if len(found) == 1 else None, (found or pages)


def dump(port, match=None):
    pages = http_json(port, "/json/list")
    page, candidates = choose(pages, match)
    if page is None:
        return None, [{"title": p.get("title", ""), "url": p.get("url", "")[:100]} for p in candidates]
    path = "/" + page["webSocketDebuggerUrl"].split("/", 3)[3]
    ws = Socket(port, path)
    try:
        ws.send(json.dumps({"id": 1, "method": "Runtime.evaluate", "params": {"expression": EXPRESSION, "awaitPromise": True, "returnByValue": True}}))
        while True:
            reply = json.loads(ws.recv())
            if reply.get("id") == 1:
                break
    finally:
        ws.close()
    result = reply.get("result", {})
    if "exceptionDetails" in result or "value" not in result.get("result", {}):
        raise OSError("the page did not answer the inventory: %s" % json.dumps(result)[:300])
    return result["result"]["value"], None


def main(argv):
    port, match, out, list_only = 9998, None, None, False
    rest = argv[1:]
    while rest:
        flag = rest.pop(0)
        if flag == "--port" and rest:
            port = int(rest.pop(0))
        elif flag == "--match" and rest:
            match = rest.pop(0)
        elif flag == "--out" and rest:
            out = rest.pop(0)
        elif flag == "--list":
            list_only = True
        else:
            print(__doc__.split("\n\n")[1].strip(), file=sys.stderr)
            return 2
    try:
        if list_only:
            for p in http_json(port, "/json/list"):
                print(json.dumps({"type": p.get("type"), "title": p.get("title", ""), "url": p.get("url", "")[:100]}))
            return 0
        value, candidates = dump(port, match)
    except (OSError, ValueError) as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        return 1
    if value is None:
        print(json.dumps({"ok": False, "error": "no single page matched", "candidates": candidates}))
        return 2
    text = json.dumps({"ok": True, "page": value}, sort_keys=True)
    if out:
        with open(out, "w") as f:
            f.write(text + "\n")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
