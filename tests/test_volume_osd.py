"""Receiver display freshness through the running offline QML host."""
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

FIND = """function find(p) {
    if (p.staleMs !== undefined) return p;
    for (var i=0; i<p.children.length; i++) { var found=find(p.children[i]); if(found) return found; }
    return null;
} var osd=find(root); if(!osd) throw Error('volume display not built');"""
PUBLISH = """var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/receiver');
x.setRequestHeader('Content-Type','application/json'); x.onreadystatechange=function(){if(x.readyState===4) console.info('[receiver-publish] '+x.status)};
x.send(JSON.stringify(root.theme.testReceiver));"""


def expr(body):
    return '(function(){' + FIND + body + '})()'


@unittest.skipUnless(DISPLAY and HAVE_QT, 'needs Xvfb and PyQt5')
class VolumeOsd(HostCase):
    def test_replayed_reading_expires_and_new_reading_recovers(self):
        rc, out = self.host(
            '--eval', '6000:' + expr("osd.staleMs=1200; osd.holdMs=6000; root.theme.testReceiver={reachable:true,fresh:true,updatedAt:Date.now(),volumeText:'35.5',volumeAbs:35.5,mute:false};" + PUBLISH + 'osd.show(); return 1;'),
            '--eval', '6400:' + expr('return osd.fresh && osd.level === "35.5";'),
            '--eval', '6900:' + expr(PUBLISH + 'return 1;'),
            '--eval', '8500:' + expr('return osd.stale && osd.level === "" && osd.offline;'),
            '--eval', '8700:' + expr('root.theme.testReceiver.updatedAt=Date.now();' + PUBLISH + 'return 1;'),
            '--eval', '9200:' + expr('return osd.fresh && osd.level === "35.5" && !osd.offline;'),
            '--wait', '9500')
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(self.values(out)[1::2], ['true', 'true', 'true'], out[-2500:])

    def test_old_snapshot_is_hidden_immediately(self):
        rc, out = self.host(
            '--eval', '6000:' + expr("osd.holdMs=6000; root.theme.testReceiver={reachable:true,fresh:true,updatedAt:Date.now()-7000,volumeText:'35.5',volumeAbs:35.5,mute:false};" + PUBLISH + 'osd.show(); return 1;'),
            '--eval', '6400:' + expr('return !osd.fresh && osd.level === "" && osd.offline;'),
            '--wait', '6800')
        self.assertEqual(rc, 0, out[-2500:])
        self.assertEqual(self.values(out)[-1], 'true', out[-2500:])


if __name__ == '__main__':
    unittest.main()
