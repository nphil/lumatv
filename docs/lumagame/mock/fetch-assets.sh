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
