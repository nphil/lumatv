#!/bin/sh
# Round 8: a clean worktree of one commit, the git-ignored build products beside it, the helper built there, then the ipk.
#   sh /tmp/r8/build.sh <sha>
set -eu
SHA=$1
W=/tmp/lumatv-slot
M=/data/home/lumaglass
cd "$M"
git worktree remove --force "$W" 2>/dev/null || rm -rf "$W"
git worktree prune
git worktree add "$W" "$SHA"
ln -sfn /data/home/LumaShow /tmp/LumaShow
# Build products that are git-ignored: copied from the main checkout (their sources are checked against the commit by the caller).
for d in ambient/capture/build jpeg/build vplane/build rustui/build gfx64/build; do
    mkdir -p "$W/$(dirname "$d")"
    cp -a "$M/$d" "$W/$d"
done
# The cargo caches, so the release builds are incremental (registry crates keep their fingerprints; the path crates rebuild).
cp -a "$M/helper/target" "$W/helper/target"
cp -a "$M/compositor/target" "$W/compositor/target"
echo "== jpeg and vplane builds $(date +%H:%M:%S)"
(cd "$W" && sh jpeg/build.sh) 2>&1 | tail -3
(cd "$W" && sh vplane/build.sh) 2>&1 | tail -3
echo "== helper build $(date +%H:%M:%S)"
(cd "$W/helper" && ./build.sh) 2>&1 | tail -5
echo "== ipk build $(date +%H:%M:%S)"
(cd "$W" && sh tools/build-ipk.sh) 2>&1 | tail -15
ls -la "$W"/org.nphil.lumatv_*.ipk
echo "== done $(date +%H:%M:%S)"
