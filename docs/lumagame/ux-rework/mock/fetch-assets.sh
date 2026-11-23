#!/usr/bin/env bash
# Fetches the extra art the UX rework mocks use, into mock/assets/: Hades II achievement icons
# and full-size screenshots from Steam's public CDN. The game art itself comes from
# ../../mock/fetch-assets.sh. All of it belongs to its publishers and stays out of git.
set -euo pipefail

cd "$(dirname "$0")"
bash ../../mock/fetch-assets.sh

icons=(
  "witch-of-the-woods.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/40ee8c3795ae62300c90157abd8da5f3d02e3d5d.jpg"
  "witch-of-the-depths.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/bacabdad32d575e03ef718f091249e639d394f9f.jpg"
  "witch-of-the-plains.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/ead0acbb6c3cb0739f7fd377c38faa8a144408b3.jpg"
  "witch-of-the-outskirts.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/7c5d6ddda789caf10e7a09726c5eac6e338028ac.jpg"
  "witch-of-the-waters.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/b6921206330324b72229f08c8a9560308167e298.jpg"
  "soothing-soak.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/e82f786b540bf24eb16c74cc660f1ee26d8cc610.jpg"
  "witch-of-the-abyss.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/4aa6f5855a9c785ecde5935211c969054797a68d.jpg"
  "the-arms-of-night.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/d3001619a5b39f634c6ee1462c02c6c7ab76f6b2.jpg"
  "so-mote-it-be.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/218670037563eb2f2fb9918c4c37ed63bc921d9b.jpg"
  "mind-for-magick.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/4d28974d1f966e6bac5221972ec64441a283944d.jpg"
  "death-to-chronos.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/0017f60f2a9c3b0737fbb149e4daa7236b3d3bda.jpg"
  "animal-whisperer.jpg https://shared.fastly.steamstatic.com/community_assets/images/apps/1145350/b5220fc40d52fa9bb519203160216a17e4a801b9.jpg"
)
mkdir -p assets/ach
for i in "${icons[@]}"; do
  read -r name url <<<"$i"
  out="assets/ach/$name"
  [ -s "$out" ] && continue
  curl -fsSL --retry 2 -o "$out" "$url" || { rm -f "$out"; echo "missing: ach/$name" >&2; }
done

# Full-size frames for the media viewer and the in-stream menus.
store=https://shared.akamai.steamstatic.com/store_item_assets/steam/apps
shots=(
  "full-1.jpg $store/1145350/ss_ef0f63061d0a0a9a7e46f3b84f125d25330e8f19.1920x1080.jpg"
  "full-2.jpg $store/1145350/ss_f28befd916e59b8bf0a8a801b8a498b8adaa01eb.1920x1080.jpg"
  "full-3.jpg $store/1145350/ss_b88cb7b48a86f07a7288bf37141f6558279f9bfc.1920x1080.jpg"
)
mkdir -p assets/shots
for s in "${shots[@]}"; do
  read -r name url <<<"$s"
  out="assets/shots/$name"
  [ -s "$out" ] && continue
  curl -fsSL --retry 2 -o "$out" "$url" || { rm -f "$out"; echo "missing: shots/$name" >&2; }
done
