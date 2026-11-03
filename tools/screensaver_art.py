#!/usr/bin/env python3
"""Fetches the art mode's pictures and writes `compositor/assets/ui/art/` with its index.

    python3 tools/screensaver_art.py [--check]

The art mode (panels/Screensaver.qml) shows public-domain paintings that are on the set, never fetched at view time. Each picture
is listed below with the words the caption uses (the artist, the title, the year: written here by hand, not scraped) and the
Wikimedia Commons file it comes from. The licence is checked against Commons' own record when the picture is fetched: the file's
`LicenseShortName` must start with "Public domain" (or "PD"), or nothing is written. The index keeps what was checked: the licence
words, the page, the day, and the SHA-256 of the file that is on the set, so a later `--check` finds a picture that changed or a
file that is not the one the licence was read for.

Pictures are scaled to 1280 px on the long side (the print is shown at most 1040 by 700 on a 1080p screen) and saved as JPEG.
"""
import argparse
import datetime
import hashlib
import io
import json
import os
import sys
import urllib.parse
import urllib.request

from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
OUT = os.path.join(ROOT, "compositor", "assets", "ui", "art")
API = "https://commons.wikimedia.org/w/api.php"
UA = "LumaTV/0.4 (screensaver art; licence-checked; https://nateshome.net)"
LONG_SIDE = 1280

# id, Commons file, artist, title, year words as the caption says them
PICTURES = [
    ("great-wave", "Tsunami by hokusai 19th century.jpg", "Katsushika Hokusai", "Under the Wave off Kanagawa", "about 1831"),
    ("starry-night", "Van Gogh - Starry Night - Google Art Project.jpg", "Vincent van Gogh", "The Starry Night", "1889"),
    ("impression-sunrise", "Claude Monet, Impression, soleil levant.jpg", "Claude Monet", "Impression, Sunrise", "1872"),
    ("fighting-temeraire", "The Fighting Temeraire, JMW Turner, National Gallery.jpg", "J. M. W. Turner", "The Fighting Temeraire", "1839"),
    ("view-of-delft", "Vermeer-view-of-delft.jpg", "Johannes Vermeer", "View of Delft", "about 1660"),
    ("sierra-nevada", "Albert Bierstadt - Among the Sierra Nevada, California - Google Art Project.jpg", "Albert Bierstadt",
     "Among the Sierra Nevada, California", "1868"),
]


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def commons(file_name):
    q = {"action": "query", "format": "json", "prop": "imageinfo", "iiprop": "url|size|extmetadata", "iiurlwidth": LONG_SIDE,
         "titles": "File:" + file_name}
    doc = json.loads(get(API + "?" + urllib.parse.urlencode(q)))
    page = next(iter(doc["query"]["pages"].values()))
    if "imageinfo" not in page:
        raise SystemExit("Commons has no file %r" % file_name)
    return page["imageinfo"][0]


def licence_ok(words):
    return words.startswith("Public domain") or words.startswith("PD")


def fetch():
    os.makedirs(OUT, exist_ok=True)
    today = datetime.date.today().isoformat()
    index = []
    for pid, file_name, artist, title, year in PICTURES:
        info = commons(file_name)
        words = info["extmetadata"].get("LicenseShortName", {}).get("value", "")
        if not licence_ok(words):
            raise SystemExit("%s: Commons says %r, not public domain: not used" % (pid, words))
        raw = get(info["thumburl"])
        im = Image.open(io.BytesIO(raw)).convert("RGB")
        scale = LONG_SIDE / max(im.size)
        if scale < 1:
            im = im.resize((round(im.width * scale), round(im.height * scale)), Image.LANCZOS)
        buf = io.BytesIO()
        im.save(buf, "JPEG", quality=82, optimize=True, progressive=True)
        data = buf.getvalue()
        with open(os.path.join(OUT, pid + ".jpg"), "wb") as f:
            f.write(data)
        index.append({"id": pid, "file": pid + ".jpg", "artist": artist, "title": title, "year": year, "licence": "Public domain",
                      "licenceChecked": words, "source": info["descriptionurl"], "checkedAt": today, "width": im.width,
                      "height": im.height, "sha256": hashlib.sha256(data).hexdigest()})
        print("%-20s %dx%d %6d bytes  %s" % (pid, im.width, im.height, len(data), words))
    with open(os.path.join(OUT, "index.json"), "w") as f:
        json.dump({"schema": 1, "pictures": index}, f, indent=2, ensure_ascii=False)
        f.write("\n")


def check():
    """The index and the files agree, and every picture carries a public-domain licence."""
    with open(os.path.join(OUT, "index.json")) as f:
        doc = json.load(f)
    bad = []
    for p in doc["pictures"]:
        path = os.path.join(OUT, p["file"])
        if not os.path.exists(path):
            bad.append("%s: file missing" % p["id"])
            continue
        with open(path, "rb") as f:
            if hashlib.sha256(f.read()).hexdigest() != p["sha256"]:
                bad.append("%s: not the file the licence was checked for" % p["id"])
        if not licence_ok(p["licenceChecked"]):
            bad.append("%s: licence %r" % (p["id"], p["licenceChecked"]))
    for line in bad:
        print(line)
    return 1 if bad else 0


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true", help="only verify the files against the index")
    a = ap.parse_args()
    sys.exit(check() if a.check else (fetch() or 0))
