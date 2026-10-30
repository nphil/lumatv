import sys, time, json
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv, rustlib as R
tv.tv("""luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"youtube.leanback.v4","params":{"contentTarget":"https://www.youtube.com/tv?v=aqz-KE-bpKQ"}}'""", 20)
for i in range(60):
    if '"video":[{' in R.ctl("windows").replace(" ", "") if isinstance(R.ctl("windows"), str) else '"video":[{' in json.dumps(R.ctl("windows")).replace(" ", ""): break
    time.sleep(1)
else: print("no video window"); sys.exit(1)
time.sleep(8); print("playing after", i, "s")
