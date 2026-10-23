import sys, os, re, json
sys.path.insert(0, os.path.dirname(__file__))
import build
txt = build.tables()
extra = ''
p = os.path.join(os.path.dirname(__file__), 'noui.md')
if os.path.exists(p):
    extra = open(p).read()
full = txt + '\n' + extra
ids = json.load(open('/tmp/surfaces/all_ids.json'))
miss = [i for i in ids if i not in full]
print('app ids missing:', len(miss))
for i in miss: print('  ', i)
# compositor view files
base = '/data/home/lgtv/uiinv/qml/qml'
files = []
for root in ('WebOSCompositor/views', 'WebOSCompositorBase/views', 'WebOSCompositorBase/zoom'):
    for dp, dn, fn in os.walk(f'{base}/{root}'):
        for f in fn:
            files.append(os.path.relpath(os.path.join(dp, f), base))
fm = [f for f in files if os.path.basename(f) not in full]
print('view files:', len(files), 'missing:', len(fm))
for f in fm: print('  ', f)
