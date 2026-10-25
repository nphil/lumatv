#!/bin/sh
# Drives the first-light run from the workstation (needs TV access: ask the main agent for "TV is yours").
#   tv-first-light.sh deploy      build (tools/build-tv.sh) and copy compositor/deploy to /tmp/lumatv on the set
#   tv-first-light.sh run [case]  run.sh <case> on the set (default all); prints the PASS/FAIL lines
#   tv-first-light.sh app [BIN]   the first-app step
#   tv-first-light.sh fetch       pull /tmp/lumatv/out to /tmp/lumatv-tv-out and write the log summary to
#                                 docs/research/compositor/first-light-<date>.txt
#   tv-first-light.sh kms         the C3 maintenance window (tools/tv_kms.py): LG's compositor stopped, LumaTV as DRM
#                                 master for ~40 s, LG's compositor restarted; results in /tmp/lumatv-tv-out/out/kms
#   tv-first-light.sh sm          the C4 supervisor/bridge dry run beside LG's compositor (bus names suffixed .lumatv)
#   tv-first-light.sh inspect     read-only: the compositor unit, its wrapper, the role files for /usr/bin/surface-manager,
#                                 initctl, the state of webapp-mgr; written to docs/research/compositor/set-inspect-<date>.txt
#   tv-first-light.sh cutover ARGS  /tmp/lumatv/cutover.sh ARGS on the set under a PTY (install | probe | trial SECS |
#                                 enable | disable | status | revert)
#   tv-first-light.sh gate [ARGS] the C4 gate (tools/tv_gate.py): enable, live, x1, netflix, hdmi, standby/wake, escape;
#                                 --enable-after makes LumaTV's compositor live when every check passed
#   tv-first-light.sh clean       kill everything of ours on the set
#   tv-first-light.sh probe       read-only (tools/tv-readonly-probe.sh): LG's wl_webos_* tables in every copy on the set
#                                 and the Wayland clients of LG's compositor; written to
#                                 docs/research/compositor/probe-<date>.txt; safe beside another session
# Transport: docs/research/arch/tv.py (paramiko). Before `run`: confirm the set is idle (getPowerState Active,
# getForegroundAppInfo) and that nothing else is being measured on it.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/.." && pwd)
tvpy=$repo/docs/research/arch/tv.py
tv() { python3 "$tvpy" "$@"; }
put() { python3 -c "import sys; sys.path.insert(0, '$repo/docs/research/arch'); import tv; tv.put(sys.argv[1], sys.argv[2])" "$1" "$2"; }
get() { python3 -c "import sys; sys.path.insert(0, '$repo/docs/research/arch'); import tv; tv.get(sys.argv[1], sys.argv[2])" "$1" "$2"; }

case "${1:-help}" in
    deploy)
        sh "$here/tools/build-tv.sh"
        tar -C "$here/deploy" -czf /tmp/lumatv-deploy.tgz .
        ls -la /tmp/lumatv-deploy.tgz
        tv "mkdir -p /tmp/lumatv && rm -rf /tmp/lumatv/bin /tmp/lumatv/lib"
        put /tmp/lumatv-deploy.tgz /tmp/lumatv/deploy.tgz
        tv "cd /tmp/lumatv && tar xzf deploy.tgz && rm deploy.tgz && chmod +x run.sh bin/* && ls -la bin && grep -E 'MemAvailable|CmaFree' /proc/meminfo"
        ;;
    run)
        tv "sh /tmp/lumatv/run.sh ${2:-all}"
        ;;
    app)
        shift
        tv "sh /tmp/lumatv/run.sh app $*"
        ;;
    fetch)
        tv "cd /tmp/lumatv && tar czf /tmp/lumatv-out.tgz out"
        mkdir -p /tmp/lumatv-tv-out
        get /tmp/lumatv-out.tgz /tmp/lumatv-tv-out/out.tgz
        tar -C /tmp/lumatv-tv-out -xzf /tmp/lumatv-tv-out/out.tgz
        mkdir -p "$repo/docs/research/compositor"
        day=$(date +%Y-%m-%d)
        {
            echo "# First-light run $day (compositor/tools/tv-first-light.sh)"
            for c in driver own shm app; do
                [ -d /tmp/lumatv-tv-out/out/$c ] || continue
                echo; echo "## $c"; echo
                grep -vE ' debug ' /tmp/lumatv-tv-out/out/$c/server.log | sed 's/^/    /'
                for f in /tmp/lumatv-tv-out/out/$c/client*.log; do [ -f "$f" ] && { echo; echo "    --- $(basename $f)"; grep -vE 'EGL display extensions' "$f" | sed 's/^/    /'; }; done
                ls /tmp/lumatv-tv-out/out/$c/*.png 2>/dev/null | sed 's/^/    PNG /'
            done
            [ -f /tmp/lumatv-tv-out/out/protocols.txt ] && { echo; echo "## protocols"; echo; sed 's/^/    /' /tmp/lumatv-tv-out/out/protocols.txt; }
        } > "$repo/docs/research/compositor/first-light-$day.txt"
        echo "wrote docs/research/compositor/first-light-$day.txt; PNGs under /tmp/lumatv-tv-out/out/"
        ;;
    kms)
        python3 "$here/tools/tv_kms.py" run
        ;;
    gate)
        shift
        python3 "$here/tools/tv_gate.py" "$@"
        ;;
    sm|cutover|inspect)
        # luna-send needs a PTY (HANDOFF), so the whole on-set script runs under one.
        case "$1" in
            sm) cmd="sh /tmp/lumatv/sm-dryrun.sh run" ;;
            cutover) shift; cmd="sh /tmp/lumatv/cutover.sh $*" ;;
            inspect) cmd="sh /tmp/lumatv/inspect.sh" ; put "$here/tools/tv-inspect.sh" /tmp/lumatv/inspect.sh ;;
        esac
        out=$(python3 - "$cmd" <<EOF2
import sys; sys.path.insert(0, "$repo/docs/research/arch"); import tv, paramiko
c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
_, out, _ = c.exec_command(sys.argv[1], timeout=600, get_pty=True)
print(out.read().decode("utf-8", "replace")); c.close()
EOF2
)
        echo "$out"
        if [ "$1" = inspect ]; then
            mkdir -p "$repo/docs/research/compositor"
            echo "$out" > "$repo/docs/research/compositor/set-inspect-$(date +%Y-%m-%d).txt"
            echo "wrote docs/research/compositor/set-inspect-$(date +%Y-%m-%d).txt"
        fi
        ;;
    kms-restore)
        python3 "$here/tools/tv_kms.py" restore
        ;;
    probe)
        tv "mkdir -p /tmp/lumatv/out"
        put "$here/tools/tv-readonly-probe.sh" /tmp/lumatv/probe.sh
        put "$repo/docs/research/arch/wlproto_from_elf.py" /tmp/lumatv/wlproto_from_elf.py
        tv "sh /tmp/lumatv/probe.sh"
        mkdir -p "$repo/docs/research/compositor"
        get /tmp/lumatv/out/probe/probe.txt "$repo/docs/research/compositor/probe-$(date +%Y-%m-%d).txt"
        echo "wrote docs/research/compositor/probe-$(date +%Y-%m-%d).txt"
        ;;
    clean)
        tv "sh /tmp/lumatv/run.sh clean"
        ;;
    *) sed -n 2,21p "$0" ;;
esac
