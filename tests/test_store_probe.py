"""Run with: python3 -m unittest tests.test_store_probe

The Content Store theme's probe (tools/store_probe.py and the probe part of payload/store/userScript.js): what the set could not give, the
store's markup, comes from the page itself. `Receiver` is the loopback server the page posts to: what it accepts, what it refuses, how it
answers the browser's preflight, that it is bounded and ends by itself. `Report` reads the files it wrote. `Privacy` runs the real script
in the system Chromium against a page full of things that must not leave it (its words, its URLs' queries, its attribute values) and checks
none of them reached the receiver, while the structure the next round needs did.
"""
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(ROOT, "tools"))
import store_probe  # noqa: E402
import store_theme  # noqa: E402

TOOL = os.path.join(ROOT, "tools", "store_probe.py")
PAYLOAD = os.path.join(ROOT, "payload", "store")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
PLAYWRIGHT = "/data/home/lucent-ha/node_modules/playwright-core/index.mjs"
CHROMIUM = "/usr/bin/chromium"
BROWSER = os.path.exists(PLAYWRIGHT) and os.path.exists(CHROMIUM)


def ask(request):
    """What the receiver's handler answers to raw request bytes, over a socket pair (no process, no port)."""
    a, b = socket.socketpair()
    try:
        a.sendall(request)
        a.shutdown(socket.SHUT_WR)
        rx = ask.rx
        rx.handle(b)
        out = b""
        a.settimeout(5)
        while True:
            chunk = a.recv(65536)
            if not chunk:
                break
            out += chunk
        return out
    finally:
        a.close()


def split(response):
    head, _, body = response.partition(b"\r\n\r\n")
    lines = head.decode("latin1").split("\r\n")
    headers = dict(l.split(": ", 1) for l in lines[1:])
    return int(lines[0].split(" ")[1]), headers, body


def post_request(path, body, extra=""):
    return ("POST %s HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://app.lgwebostv.com\r\nContent-Type: text/plain\r\nContent-Length: %d\r\n%s\r\n" % (path, len(body), extra)).encode() + body


class Receiver(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = self.tmp.name
        ask.rx = store_probe.Receiver(self.dir)

    def tearDown(self):
        self.tmp.cleanup()

    def files(self):
        return sorted(n for n in os.listdir(self.dir) if n[0].isdigit())

    def test_a_post_is_written_whole_under_its_kind_and_acknowledged(self):
        body = json.dumps({"kind": "dom", "data": {"nodes": [1, 2, 3]}}).encode()
        status, headers, answer = split(ask(post_request("/p", body)))
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(answer), {"ok": True, "n": 1, "file": "001-dom.json"})
        with open(os.path.join(self.dir, "001-dom.json"), "rb") as f:
            self.assertEqual(f.read(), body)
        self.assertEqual(headers["Connection"], "close")
        self.assertEqual(headers["Content-Length"], str(len(answer)))

    def test_the_browsers_preflight_and_the_cors_headers_are_answered_for_a_page_on_another_origin(self):
        status, headers, _ = split(ask(b"OPTIONS /p HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://app.lgwebostv.com\r\n"
                                       b"Access-Control-Request-Method: POST\r\nAccess-Control-Request-Private-Network: true\r\n\r\n"))
        self.assertEqual(status, 204)
        self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
        self.assertEqual(headers["Access-Control-Allow-Private-Network"], "true")
        status, headers, _ = split(ask(post_request("/p", b'{"kind":"hello"}')))
        self.assertEqual((headers["Access-Control-Allow-Origin"], headers["Access-Control-Allow-Private-Network"]), ("*", "true"))

    def test_a_kind_cannot_name_a_path_and_what_is_not_json_is_kept_as_text(self):
        status, _, answer = split(ask(post_request("/p", b'{"kind":"../../etc/x"}')))
        self.assertEqual(json.loads(answer)["file"], "001-etcx.json")
        status, _, answer = split(ask(post_request("/p", b"not json at all")))
        self.assertEqual(json.loads(answer)["file"], "002-raw.txt")
        self.assertEqual(self.files(), ["001-etcx.json", "002-raw.txt"])

    def test_the_beacon_fallback_is_logged_and_nothing_else_is_served(self):
        status, _, _ = split(ask(b"GET /b?k=dom&d=%7B%22a%22%3A1%7D HTTP/1.1\r\nHost: x\r\n\r\n"))
        self.assertEqual(status, 204)
        with open(os.path.join(self.dir, "beacons.log")) as f:
            self.assertIn('dom {"a":1}', f.read())
        self.assertEqual(split(ask(b"GET /ping HTTP/1.1\r\nHost: x\r\n\r\n"))[2], b"ok")
        for req in (b"GET /etc/passwd HTTP/1.1\r\nHost: x\r\n\r\n", b"POST /other HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n", b"DELETE /p HTTP/1.1\r\nHost: x\r\n\r\n"):
            self.assertEqual(split(ask(req))[0], 404, req)
        self.assertEqual(self.files(), [])

    def test_a_body_over_the_limit_is_refused_unread(self):
        status, _, _ = split(ask(("POST /p HTTP/1.1\r\nHost: x\r\nContent-Length: %d\r\n\r\n" % (store_probe.MAX_BODY + 1)).encode()))
        self.assertEqual(status, 413)
        self.assertEqual(self.files(), [])

    def test_it_takes_a_bounded_number_of_files_and_bytes(self):
        old = store_probe.MAX_FILES, store_probe.MAX_BYTES
        try:
            store_probe.MAX_FILES = 3
            for i in range(3):
                self.assertEqual(split(ask(post_request("/p", b'{"kind":"x"}')))[0], 200)
            self.assertEqual(split(ask(post_request("/p", b'{"kind":"x"}')))[0], 429)
            self.assertEqual(len(self.files()), 3)
            store_probe.MAX_FILES = 100
            store_probe.MAX_BYTES = 40
            ask.rx = store_probe.Receiver(tempfile.mkdtemp(dir=self.dir))
            self.assertEqual(split(ask(post_request("/p", b'{"kind":"x","data":"' + b"y" * 60 + b'"}')))[0], 200)
            self.assertEqual(split(ask(post_request("/p", b'{"kind":"x"}')))[0], 429, "the bytes taken are bounded too")
        finally:
            store_probe.MAX_FILES, store_probe.MAX_BYTES = old

    def test_a_request_that_is_not_http_is_dropped_without_a_file(self):
        out = ask(b"\x00\x01\x02 not a request\r\n\r\n")
        self.assertIn(out[:12], (b"", b"HTTP/1.1 404"))
        self.assertEqual(self.files(), [])


class Serving(unittest.TestCase):
    """The process: where it listens, that it says so only then, and that it ends."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.portfile = os.path.join(self.d, "port")
        self.pidfile = os.path.join(self.d, "pid")
        self.procs = []

    def tearDown(self):
        for p in self.procs:
            if p.poll() is None:
                p.kill()
            p.wait()
        self.tmp.cleanup()

    def start(self, *extra):
        p = subprocess.Popen([sys.executable, TOOL, "serve", "--dir", os.path.join(self.d, "out"), "--port", "0", "--portfile", self.portfile,
                              "--pidfile", self.pidfile, *extra], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        deadline = time.time() + 10
        while not os.path.exists(self.portfile) and time.time() < deadline:
            time.sleep(0.02)
        with open(self.portfile) as f:
            return p, int(f.read())

    def test_the_port_file_appears_only_when_it_can_be_connected_to(self):
        p, port = self.start("--seconds", "30")
        c = socket.create_connection(("127.0.0.1", port), timeout=5)
        c.sendall(b"GET /ping HTTP/1.1\r\nHost: x\r\n\r\n")
        self.assertIn(b"200 OK", c.recv(4096))
        c.close()
        with open(self.pidfile) as f:
            self.assertEqual(int(f.read()), p.pid)

    def test_it_ends_by_itself_and_takes_its_files_away_but_not_the_data(self):
        p, port = self.start("--seconds", "1")
        socket.create_connection(("127.0.0.1", port), timeout=5).close()
        self.assertEqual(p.wait(timeout=10), 0)
        self.assertFalse(os.path.exists(self.pidfile) or os.path.exists(self.portfile))
        with open(os.path.join(self.d, "out", "receiver.log")) as f:
            self.assertIn("ended", f.read())

    def test_sigterm_ends_it_cleanly(self):
        p, port = self.start("--seconds", "60")
        os.kill(p.pid, signal.SIGTERM)
        self.assertEqual(p.wait(timeout=10), 0)
        self.assertFalse(os.path.exists(self.pidfile))

    def test_a_port_that_is_taken_is_an_error_and_not_a_hang(self):
        busy = socket.socket()
        busy.bind(("127.0.0.1", 0))
        busy.listen(1)
        try:
            r = subprocess.run([sys.executable, TOOL, "serve", "--dir", os.path.join(self.d, "out"), "--port", str(busy.getsockname()[1]), "--seconds", "5"],
                               capture_output=True, text=True, timeout=20)
        finally:
            busy.close()
        self.assertEqual(r.returncode, 1)
        self.assertIn("cannot listen", r.stderr)

    def test_usage_is_an_error(self):
        for args in ([], ["serve"], ["serve", "--dir"], ["report"], ["nonsense"]):
            self.assertEqual(subprocess.run([sys.executable, TOOL, *args], capture_output=True, text=True).returncode, 2, args)

    def test_it_never_listens_on_anything_but_loopback(self):
        with open(TOOL) as f:
            text = f.read()
        self.assertIn('srv.bind(("127.0.0.1"', text)
        self.assertNotIn("0.0.0.0", text)


def node(i, tag, cls="", rect=(0, 0, 10, 10), **style):
    return {"i": i, "p": max(i - 1, -1), "t": tag, "c": cls, "a": {}, "r": list(rect), "x": style.pop("x", 0), "s": style}


class Report(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, name, kind, data, page="https://app.lgwebostv.com/discovery/x.html"):
        with open(os.path.join(self.d, name), "w") as f:
            json.dump({"kind": kind, "ms": 5, "page": page, "top": True, "data": data}, f)

    def test_the_report_names_what_the_next_round_needs(self):
        self.write("001-env.json", "env", {"ua": "Chrome/108", "vp": [1920, 1080, 1], "origin": "https://app.lgwebostv.com", "ready": "complete", "parts": "probe",
                                           "fonts": ["Sandstone|400|normal|loaded"], "sheets": [{"href": "https://x/app.css", "rules": 1200}],
                                           "vars": {"0": {"--sand-focus-bg-color-rgb": "230, 230, 230"}}, "csp": [], "families": {"Sandstone": 40, "MuseoSans": 3}})
        header = node(0, "header", "enact_sandstone_Header_Header_header__a", (0, 0, 1920, 232), bg="rgb(0, 0, 0)")
        tab = node(1, "div", "enact_sandstone_TabGroup_TabGroup_tab__b", (80, 154, 340, 76))
        tab["b"] = {"bg": "rgb(230, 230, 230)", "op": "0", "w": "340px", "h": "76px"}
        self.write("002-dom.json", "dom", {"reason": "load", "truncated": False, "nodes": [header, tab]})
        item = node(0, "div", "enact_sandstone_ImageItem_ImageItem_captions__c", (536, 878, 344, 126))
        bg = node(1, "div", "enact_sandstone_Item_Item_bg__d", (536, 878, 344, 126), bg="rgb(230, 230, 230)")
        word = node(2, "span", "enact_sandstone_ImageItem_ImageItem_label__e", (560, 890, 200, 30), x=12, co="rgb(75, 76, 80)")
        self.write("003-focus.json", "focus", {"found": {"plates": ["ps:div:43344"], "words": 1, "ms": 2}, "chain": [item], "sub": [item, bg, word]})
        self.write("004-classes.json", "classes", {"key": "arrow", "changes": [{"el": "div.item", "add": ["focused"], "rem": []}, {"el": "div.item", "add": [], "rem": ["focused"]}]})
        out = subprocess.run([sys.executable, TOOL, "report", self.d], capture_output=True, text=True)
        self.assertEqual(out.returncode, 0, out.stderr)
        text = out.stdout
        for wanted in ("CLASS CHANGES after a arrow key", "div.item +focused", "div.item  -focused", "posts: 4", "Sandstone", "--sand-focus-bg-color-rgb", "app.css", "black bands",
                       "header.enact_sandstone_Header_Header_header__a", "::before", "rgb(230, 230, 230)", "FOCUS 003-focus.json",
                       "ps:div:43344", "div.enact_sandstone_Item_Item_bg__d", "rgb(75, 76, 80)"):
            self.assertIn(wanted, text)

    def test_the_json_form_is_one_document(self):
        self.write("001-hello.json", "hello", {"parts": "probe"})
        out = subprocess.run([sys.executable, TOOL, "report", self.d, "--json"], capture_output=True, text=True)
        self.assertEqual(json.loads(out.stdout)["kinds"], {"hello": 1})

    def test_an_empty_or_half_written_directory_is_reported_not_crashed_on(self):
        with open(os.path.join(self.d, "001-dom.json"), "w") as f:
            f.write('{"kind": "dom", "da')
        out = subprocess.run([sys.executable, TOOL, "report", self.d], capture_output=True, text=True)
        self.assertEqual(out.returncode, 0)
        self.assertIn("posts: 0", out.stdout)


SECRETS = ("Fandango Secret Title", "MyTokenValue123", "private-query-9981", "AccountNameNate", "alt-text-secret", "placeholder-secret", "value-secret-77",
           "aria-secret-label", "data-attr-secret", "bg-image-secret")

PAGE = """<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Fandango Secret Title</title>
<style>
  html, body { margin: 0; background: #000; color: #eee; font-family: Sandstone, sans-serif; }
  .enact_sandstone_Header_Header_header__a { height: 232px; background: #000; }
  .cap { position: relative; display: block; width: 400px; height: 120px; padding: 10px; }
  .cap::before { content: ""; position: absolute; inset: 0; background: rgb(230, 230, 230); opacity: 0; }
  .cap:focus::before { opacity: 1; }
  .cap > span { position: relative; display: block; }
</style></head><body>
<header class="enact_sandstone_Header_Header_header__a"><h1>Fandango Secret Title</h1></header>
<a href="https://store.example.com/app?token=MyTokenValue123&who=private-query-9981#frag" id="link1" class="cap" tabindex="0" title="alt-text-secret"
   aria-label="aria-secret-label" data-x="data-attr-secret" style="background-image: url(https://img.example.com/p.png?sig=bg-image-secret)">
  <span class="enact_sandstone_ImageItem_ImageItem_label__e">AccountNameNate</span><span>Fandango Secret Title</span>
  <img src="https://img.example.com/icon.png?sig=MyTokenValue123" alt="alt-text-secret"></a>
<input id="in" value="value-secret-77" placeholder="placeholder-secret"><button class="b" tabindex="0">AccountNameNate</button>
<script>document.addEventListener('keydown', function () { document.getElementById('link1').classList.toggle('cur-marker'); });</script>
</body></html>"""


@unittest.skipUnless(BROWSER, "needs the system Chromium and playwright-core")
class Privacy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        d = cls.tmp.name
        cls.out = os.path.join(d, "out")
        cls.portfile = os.path.join(d, "port")
        cls.server = subprocess.Popen([sys.executable, TOOL, "serve", "--dir", cls.out, "--port", "0", "--portfile", cls.portfile, "--seconds", "120"])
        deadline = time.time() + 10
        while not os.path.exists(cls.portfile) and time.time() < deadline:
            time.sleep(0.02)
        with open(cls.portfile) as f:
            port = int(f.read())
        script = os.path.join(d, "userScript.js")
        text, _ = store_theme.build_script(PAYLOAD, SHIPPED, "/dev/null", store_theme.parse_parts("look,probe"), port)
        with open(script, "w") as f:
            f.write(text)
        page = os.path.join(d, "page.html")
        with open(page, "w") as f:
            f.write(PAGE)
        r = subprocess.run(["node", os.path.join(HERE, "store_probe_browser.mjs"), script, page, "#link1"], capture_output=True, text=True, timeout=120)
        if r.returncode != 0:
            raise AssertionError("the browser run failed: " + r.stderr[-1500:])
        cls.run_ = json.loads(r.stdout)
        cls.records = {}
        for name in sorted(os.listdir(cls.out)):
            if name[0].isdigit():
                with open(os.path.join(cls.out, name)) as f:
                    cls.records[name] = f.read()

    @classmethod
    def tearDownClass(cls):
        cls.server.kill()
        cls.server.wait()
        cls.tmp.cleanup()

    def test_the_page_posted_its_structure(self):
        kinds = sorted({n.split("-")[1].split(".")[0] for n in self.records})
        for kind in ("hello", "env", "dom", "focus", "classes"):
            self.assertIn(kind, kinds, self.run_)
        self.assertEqual(self.run_["errors"], [])

    def test_none_of_the_pages_words_or_values_left_it(self):
        everything = "\n".join(self.records.values())
        for secret in SECRETS:
            self.assertNotIn(secret, everything, secret)

    def test_what_the_next_round_needs_did(self):
        everything = "\n".join(self.records.values())
        for wanted in ("enact_sandstone_ImageItem_ImageItem_label__e", "enact_sandstone_Header_Header_header__a", "rgb(230, 230, 230)", "store.example.com/app", "img.example.com/icon.png"):
            self.assertIn(wanted, everything)
        self.assertNotIn("token=", everything, "a URL is posted as its host and path, never its query")
        focus = json.loads(next(v for k, v in self.records.items() if "focus" in k))
        self.assertTrue(focus["data"]["chain"] and focus["data"]["sub"])

    def test_how_the_page_marks_a_move_is_posted_by_the_kind_of_key_and_never_the_key(self):
        posts = [json.loads(v) for k, v in self.records.items() if "classes" in k]
        self.assertTrue(posts)
        changes = [c for p in posts for c in p["data"]["changes"]]
        self.assertTrue(any("cur-marker" in c["add"] + c["rem"] for c in changes), changes)
        self.assertTrue(all(p["data"]["key"] in ("arrow", "other") for p in posts), "a key is told by its kind: an arrow, enter, back or other")
        for p in posts:
            self.assertEqual(sorted(p["data"]), ["changes", "key"])

    def test_a_text_node_is_posted_as_its_length(self):
        dom = json.loads(next(v for k, v in self.records.items() if "dom" in k))
        label = [n for n in dom["data"]["nodes"] if "ImageItem_label" in (n.get("c") or "")][0]
        self.assertEqual(label["x"], len("AccountNameNate"))

    def test_the_report_reads_what_was_posted(self):
        r = subprocess.run([sys.executable, TOOL, "report", self.out], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("FOCUS", r.stdout)
        self.assertIn("black bands", r.stdout)
        self.assertIn("::before", r.stdout)
        for secret in SECRETS:
            self.assertNotIn(secret, r.stdout)


if __name__ == "__main__":
    unittest.main()
