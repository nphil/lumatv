import sys,time
sys.argv=['x']
import hwchain as H
def go(n,f):
    H.say("== "+n)
    try: f()
    except Exception as e: H.say("step %s raised %s"%(n,str(e)[:300]))
H.cma()
go("v23 idle",H.step_v23)
go("v45 youtube",H.step_v45)
go("v5",H.step_v5)
go("v6",H.step_v6)
go("v7",H.step_v7)
H.fetch(); H.say("hwall over")
