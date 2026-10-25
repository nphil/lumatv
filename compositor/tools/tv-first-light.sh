#!/bin/sh
# Drives the first-light run from the workstation (needs TV access: ask the main agent for "TV is yours").
#   tv-first-light.sh deploy      build (tools/build-tv.sh) and copy compositor/deploy to /tmp/lumatv on the set
#   tv-first-light.sh run [case]  run.sh <case> on the set (default all); prints the PASS/FAIL lines
#   tv-first-light.sh app [BIN]   the first-app step
#   tv-first-light.sh fetch       pull /tmp/lumatv/out to /tmp/lumatv-tv-out and write the log summary to
#                                 docs/research/compositor/first-light-<date>.txt
#   tv-first-light.sh clean       kill everything of ours on the set
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
    clean)
        tv "sh /tmp/lumatv/run.sh clean"
        ;;
    *) sed -n 2,10p "$0" ;;
esac
