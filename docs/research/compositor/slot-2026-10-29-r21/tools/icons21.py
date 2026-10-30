import sys, time
sys.path.insert(0, "/tmp/r21-build")
import rustlib as R
from PIL import Image
D = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
def crop(src, cx, cy, out):
    im = Image.open(src).convert("RGB"); im.crop((cx-80, cy-80, cx+80, cy+80)).save(out)
    return out
R.capture_png(D + "/r21-luma-home-after-full.png")
print(Image.open(D + "/r21-luma-home-after-full.png").size)
for n, x in (("netflix", 348), ("plx", 484), ("appletv", 620), ("youtube", 1567)):
    print(crop(D + "/r21-luma-home-after-full.png", x, 759, D + "/r21-luma-home-icon-%s-after-1to1.png" % n))
