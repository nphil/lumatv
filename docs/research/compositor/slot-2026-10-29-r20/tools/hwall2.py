import sys,time
sys.argv=['x']
import hwchain as H
_ctl=H.ctl
def ctl(*w):
    if w==("vwin","idle"): w=("vwin","apps")
    return _ctl(*w)
H.ctl=ctl
def go(n,f):
    H.say("== "+n+" (apps mode)")
    try: f()
    except Exception as e: H.say("step %s raised %s"%(n,str(e)[:300]))
go("v23",H.step_v23); go("v5",H.step_v5); go("v6",H.step_v6); go("v7",H.step_v7)
H.fetch(); H.say("hwall2 over")
