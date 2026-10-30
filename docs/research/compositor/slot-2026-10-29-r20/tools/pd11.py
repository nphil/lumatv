from pd import *
p0=None
ctl("paneldim want 0.8")
try:
    time.sleep(40); p=pdj(); say("ramp done: bl %s written %s writes %s reads %s"%(bl(),p['written'],p['writes'],p['reads']))
    ctl("wake reset"); t0=time.time(); time.sleep(60)
    w=ctl("wake"); p2=pdj(); j=json.loads(w[w.index('{'):])
    say("60 s dimmed: writes %s->%s reads %s->%s | %s"%(p['writes'],p2['writes'],p['reads'],p2['reads'],j['line'][:230]))
finally:
    ctl("paneldim release"); time.sleep(1); say("end bl %s"%bl())
