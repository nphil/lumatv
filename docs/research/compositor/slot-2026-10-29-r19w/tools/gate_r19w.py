import sys
sys.path[:0] = ['/tmp/lumatv-r19w/compositor/tools', '/tmp/lumatv-r19w/docs/research/arch']
import tv_gate
# The receiver is handled by the agent through the HA MCP tools; the gate's REST calls are no-ops here.
tv_gate.avr_mute = lambda on: print("avr: left to the agent (%s)" % on)
tv_gate.avr_volume = lambda: None
sys.argv = ["tv_gate.py"] + sys.argv[1:]
tv_gate.main()
