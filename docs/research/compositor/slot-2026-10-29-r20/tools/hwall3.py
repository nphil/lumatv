import sys,time
sys.argv=['x']
import hwchain as H
def go(n,f):
    H.say("== "+n)
    try: f()
    except Exception as e: H.say("step %s raised %s"%(n,str(e)[:300]))
go("p0",H.step_p0); go("p12",H.step_p12); go("a1",H.step_a1); go("a3",H.step_a3); go("orbit",H.step_orbit)
H.fetch(); H.say("hwall3 over")
