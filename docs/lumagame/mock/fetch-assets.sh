#!/usr/bin/env bash
# Fetches the Steam store art the LumaGame mocks use into mock/assets/<appid>/.
# The art belongs to its publishers and stays out of git (see ../.gitignore).
set -euo pipefail

cd "$(dirname "$0")"
cdn=https://cdn.cloudflare.steamstatic.com/steam/apps

apps=(
  1091500 1245620 1086940 1145350 367520 1174180 292030 1551360 413150
  504230 620 632470 1426210 1057090 1903340 2358720 2379780 553420
  1868140 2246340 252950 870780 2280 2310 1536610
)
files=(library_600x900_2x.jpg library_hero.jpg logo.png header.jpg)

for app in "${apps[@]}"; do
  mkdir -p "assets/$app"
  for f in "${files[@]}"; do
    out="assets/$app/$f"
    [ -s "$out" ] && continue
    curl -fsSL --retry 2 -o "$out" "$cdn/$app/$f" || { rm -f "$out"; echo "missing: $app/$f" >&2; }
  done
done

# Trailer stills and screenshots for the hub's Media tab (Hades II).
store=https://shared.akamai.steamstatic.com/store_item_assets/steam/apps
media=(
  "trailer-1.jpg $store/257204779/2f6767807df172946f5d638093dcc1c933eb24bb/movie_600x337.jpg"
  "trailer-2.jpg $store/257206304/15bba1cdfa1aa30b3b772c1c5535e1c6bf9d9fc1/movie_600x337.jpg"
  "shot-1.jpg $store/1145350/ss_ef0f63061d0a0a9a7e46f3b84f125d25330e8f19.600x338.jpg"
  "shot-2.jpg $store/1145350/ss_f28befd916e59b8bf0a8a801b8a498b8adaa01eb.600x338.jpg"
  "shot-3.jpg $store/1145350/ss_b88cb7b48a86f07a7288bf37141f6558279f9bfc.600x338.jpg"
  "shot-4.jpg $store/1145350/ss_c8d2b18451a2cc4d5b4fdd78ed84a5e64e051eac.600x338.jpg"
  "shot-5.jpg $store/1145350/ss_f366f20f4fb699f6735581e04c7c45a1ef7bd1b8.600x338.jpg"
)
mkdir -p assets/1145350/media
for m in "${media[@]}"; do
  read -r name url <<<"$m"
  out="assets/1145350/media/$name"
  [ -s "$out" ] && continue
  curl -fsSL --retry 2 -o "$out" "$url" || { rm -f "$out"; echo "missing: media/$name" >&2; }
done

# Full-size game frames behind the stats overlay mock: a bright one (Forza Horizon 5) and a dark one (Cyberpunk 2077).
frames=(
  "1551360 $store/1551360/ss_00f0090174380eeaf8753bd3d1028b6772c3aebf.1920x1080.jpg"
  "1091500 $store/1091500/f678ed77c2fe7060ac9c64e32e1472e3ec3ab7dc/ss_f678ed77c2fe7060ac9c64e32e1472e3ec3ab7dc.1920x1080.jpg"
)
for f in "${frames[@]}"; do
  read -r app url <<<"$f"
  out="assets/$app/frame.jpg"
  [ -s "$out" ] && continue
  curl -fsSL --retry 2 -o "$out" "$url" || { rm -f "$out"; echo "missing: $app/frame.jpg" >&2; }
done
