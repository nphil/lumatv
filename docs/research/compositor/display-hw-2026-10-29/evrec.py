import os,select,struct,sys,time
fds={}
for n in range(2,8):
    try: fds[os.open('/dev/input/event%d'%n,os.O_RDONLY|os.O_NONBLOCK)]=n
    except Exception as e: print('open',n,e,flush=True)
end=time.time()+float(sys.argv[1])
t0=time.time()
while time.time()<end:
    r,_,_=select.select(list(fds),[],[],1)
    for f in r:
        try: d=os.read(f,16*32)
        except Exception: continue
        for i in range(0,len(d)-15,16):
            s,us,t,c,v=struct.unpack('iiHHi',d[i:i+16])
            if t!=0: print('%.2f event%d type=%d code=%d val=%d'%(time.time()-t0,fds[f],t,c,v),flush=True)
print('end',flush=True)
