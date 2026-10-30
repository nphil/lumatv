#!/usr/bin/env python3
"""OverlayJank's idle read: YouTube playing full screen, nothing touched: ctl status, ctl ring 1, 30 s, ctl status, ctl ring 200, into video/oj/."""
import sys, time
sys.path.insert(0, "/tmp/r19w-build")
import ojcheck as O
tv = O.tv
O.say("YouTube full screen")
O.luna("luna://com.webos.service.applicationmanager/launch", '{"id":"youtube.leanback.v4","params":{"contentTarget":"https://www.youtube.com/tv?v=aqz-KE-bpKQ"}}')
time.sleep(30)
O.save("idle30-s0.json", O.ctl("status"))
O.ctl("ring 1")
time.sleep(30)
O.save("idle30-s1.json", O.ctl("status"))
O.save("idle30-ring.json", O.ctl("ring 200"))
O.luna("luna://com.webos.service.applicationmanager/closeByAppId", '{"id":"youtube.leanback.v4"}')
O.luna("luna://com.webos.service.applicationmanager/launch", '{"id":"com.webos.app.home"}')
time.sleep(5)
O.say("done")
