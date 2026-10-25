#!/bin/sh
# Read-only look at what the cutover depends on, executed ON THE SET as /tmp/lumatv/inspect.sh. Nothing is written.
sec() { echo; echo "=== $*"; }
sec "release"; cat /etc/starfish-release 2>/dev/null | head -3
sec "surface-manager-daemon.service (unit file)"; grep -vE '^#|^$' /etc/systemd/system/surface-manager-daemon.service
sec "unit properties"; systemctl show surface-manager-daemon.service -p Type,NotifyAccess,TimeoutStartUSec,TimeoutStopUSec,MainPID,ExecMainStartTimestampMonotonic,Restart,StartLimitBurst,StartLimitIntervalUSec,KillMode,NRestarts
sec "DefaultTimeoutStartSec"; grep -E 'DefaultTimeout|DefaultStartLimit' /etc/systemd/system.conf 2>/dev/null; systemctl show -p DefaultTimeoutStartUSec,DefaultStartLimitBurst,DefaultStartLimitIntervalUSec 2>/dev/null
sec "/usr/bin/surface-manager.sh"; cat /usr/bin/surface-manager.sh 2>/dev/null
sec "surface-manager process"; p=$(pgrep -x surface-manager | head -1); echo "pid $p ppid $(awk '/^PPid/{print $2}' /proc/$p/status) exe $(readlink /proc/$p/exe)"; tr '\0' '\n' < /proc/$p/cmdline; echo "--- environ (names only)"; tr '\0' '\n' < /proc/$p/environ | cut -d= -f1 | sort | tr '\n' ' '; echo
sec "/tmp/xdg (socket directory and socket modes)"; ls -la /tmp/xdg/ 2>/dev/null; ls -la /var/run/xdg 2>/dev/null
sec "env file"; cat /var/systemd/system/env/surface-manager.env 2>/dev/null
sec "/usr/bin/surface-manager"; ls -la /usr/bin/surface-manager; stat -c '%s %Y' /usr/bin/surface-manager
sec "webapp-mgr.service"; systemctl show webapp-mgr.service -p ActiveState,SubState,Result,NRestarts,StartLimitBurst,StartLimitIntervalUSec,ExecMainStartTimestampMonotonic; ls -la /tmp/lsm-ready 2>/dev/null
sec "initctl"; ls -la /sbin/initctl 2>/dev/null; file /sbin/initctl 2>/dev/null; head -c 400 /sbin/initctl 2>/dev/null | strings | head -5
sec "luna-service2 directories"; ls -d /usr/share/luna-service2/*/ /var/luna-service2/*/ /etc/luna-service2 2>/dev/null; ls /usr/share/luna-service2/ 2>/dev/null
sec "role files naming surface-manager"; grep -rl 'surface-manager' /usr/share/luna-service2 /var/luna-service2 /etc/luna-service2 2>/dev/null | head -40
for f in $(grep -rl '"/usr/bin/surface-manager"' /usr/share/luna-service2 /var/luna-service2 2>/dev/null | head -8); do echo "--- $f"; cat "$f"; echo; done
sec "role files naming com.webos.surfacemanager (allowedNames / provides)"; grep -rl 'com.webos.surfacemanager"' /usr/share/luna-service2 /var/luna-service2 2>/dev/null | head -20
sec "ls-hubd configuration"; ls -la /etc/luna-service2/ 2>/dev/null; grep -E 'security|Security' /etc/luna-service2/ls-hubd.conf 2>/dev/null
sec "/etc/ld.so.preload"; cat /etc/ld.so.preload 2>/dev/null
sec "DRM"; ls -la /dev/dri/; cat /sys/class/drm/card0/device/uevent 2>/dev/null | head -5
sec "power state"; timeout 10 luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}' 2>&1 | head -c 300; echo
sec "foreground"; timeout 10 luna-send -n 1 -f luna://com.webos.applicationManager/getForegroundAppInfo '{}' 2>&1 | head -c 300; echo
sec "memory"; grep -E 'MemAvailable|CmaFree' /proc/meminfo
sec "space for the bundle (about 38 MB in /var/lib/lumatv/compositor)"; df -h /var /tmp 2>/dev/null; du -sh /var/lib/lumatv/compositor 2>/dev/null
sec "mounts over LG's paths"; grep -E ' /usr/bin/surface-manager | /usr/palm/applications/com.webos.app.home ' /proc/self/mountinfo
