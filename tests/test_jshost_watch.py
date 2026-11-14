"""Run with: python3 -m unittest tests.test_jshost_watch

docs/research/jshost/watch.py: the lines a window prints on the set as JSON, and the windows as instances of LG's JavaScript service
host with the launch that started each one.
"""
import importlib.util
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_spec = importlib.util.spec_from_file_location("watch", os.path.join(ROOT, "docs", "research", "jshost", "watch.py"))
watch = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(watch)

BOOT = "e10720d2-4f47-4e95-88b9-bda65dbbb0c0"
GW = "/usr/bin/run-service run-js-service -u -g -k -n /usr/palm/services/com.webos.service.secondscreen.gateway"
USS = "/usr/bin/run-service /usr/bin/run-unified-service-server"
# The host that ran when the record began exits; a connection through nsl starts it again; the next window finds it still there.
FIRST = f"""B {BOOT} 1000.00 1794629000
R 6712 1000.00 95000
sh: a message of the set's shell
X 6712 1030.50
S 7001 1041.20 104050
E 1060.00
P 7001|ss.gateway|ss.gateway |80 20 104050|27444|17850
G 7001 com.webos.service.secondscreen.gateway
G 7001 secondscreen.client
C F201A8C0:0BB9 9201A8C0:D431
C 0000000000000000FFFF0000F201A8C0:0BB8 0000000000000000FFFF00009201A8C0:D432
N 7193
L 950.10|com.webos.service.secondscreen.gateway|com.webos.service.nsl-3813|{GW}
L 950.30|com.webos.service.jsserver|com.webos.lunasend-3820|{USS}
L 1040.20|com.webos.service.secondscreen.gateway|com.webos.service.nsl-7190|{GW}
L 1040.42|com.webos.service.jsserver|com.webos.lunasend-7195|{USS}
"""
SECOND = f"""B {BOOT} 1061.00 1794629061
R 7001 1061.00 104050
E 1121.00
P 7001|ss.gateway|ss.gateway |150 30 104050|30000|19000
G 7001 com.webos.service.homeconnect
N
L 1040.20|com.webos.service.secondscreen.gateway|com.webos.service.nsl-7190|{GW}
"""


class Watch(unittest.TestCase):
    def test_windows_become_instances_with_the_launch_that_started_each(self):
        first = watch.parse(FIRST)
        self.assertEqual(first["ssap"], [{"local": "192.168.1.242:3001", "remote": "192.168.1.146:54321"},
                                         {"local": "192.168.1.242:3000", "remote": "192.168.1.146:54322"}])
        s = watch.summarize([first, watch.parse(SECOND), {"error": "OSError: timed out", "t": 1794629122}])
        old, new = s["instances"]
        self.assertEqual((old["pid"], old["counted"], old["end"], old["residentS"], old["trigger"]),
                         (6712, False, 1030.5, 30.5, "com.webos.service.nsl"))
        self.assertEqual((new["counted"], new["censored"], new["firstGuest"], new["trigger"], new["residentS"]),
                         (True, "end of the record", "com.webos.service.secondscreen.gateway", "com.webos.service.nsl", 80.5))
        self.assertEqual(new["guests"], ["com.webos.service.secondscreen.gateway", "secondscreen.client", "com.webos.service.homeconnect"])
        self.assertEqual((new["rssKb"], new["pssKb"], new["cpuS"]), (30000, 19000, 1.8))
        self.assertEqual((s["windows"], s["unanswered"], s["starts"], round(s["startsPerHour"], 1)), (3, 1, 1, 29.8))
        self.assertIn("starts 1 (29.75 per hour)", watch.report(s))


if __name__ == "__main__":
    unittest.main()
