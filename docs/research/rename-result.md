# Rename to LumaTV: result (2026-10-24)

## Resume here

Phase 1 step 1 is finished in the tree and pushed; nothing has touched the
set. Next on the set: build the ipk (`sh tools/build-ipk.sh`), install it,
run any verb of `tools/lumatv` once, and check the migration below. Then go
on with phase 1 (T9 plus disk-cache env lines, shared 100 ms clock, Home stub,
heap shim, Rust helper).

## What changed

- `brand.json` holds `displayName`, `shortId`, `appId`, `dataDir`, `qmlModule`
  and `repo` (the GitHub slug used in release URLs; the repository itself still
  has to be renamed, or `repo` edited). `tools/brand.sh` reads and validates it;
  the build, the tool and the boot hook all source that one file.
- Sources carry `{{displayName}}`, `{{shortId}}`, `{{appId}}`, `{{dataDir}}`,
  `{{qmlModule}}`. `tools/build-ipk.sh` fills them in on the staged copy
  (QML, JS, CSS, HTML, JSON, sed), renames the tool, the QML module dir, the
  key filter and the stylesheet to the brand's names, and fails on any
  placeholder left over. Source files keep the checked-in name `lumatv`
  (`SRC_NAME` in the build script and `sh -n tools/lumatv` in CI); that name
  only locates files and is not shipped under a new brand.
- ipk: `org.nphil.lumatv_0.4.0_all.ipk`, appinfo id `org.nphil.lumatv`, frame
  app `org.nphil.lumatv.frame`. A test build with a different `brand.json`
  renamed everything shipped (ipk, tool, module dir, key filter, data dir in
  QML, titles).
- Tool: `tools/lumaglass` is now `tools/lumatv` and reads names from
  `brand.json`. `env.extra` allowlist and `QML_FORCE_DISK_CACHE` handling are
  unchanged. `LUMAGLASS_BOOT` is now `LUMA_BOOT`; tmp markers, the overlay
  device name and the init.d link follow the short id.
- CI (`.gitea`, `.github`) read the ipk name and artifact name from
  `brand.json`; `release.yml` builds `repo.json` from the manifest with `jq`;
  `gen-manifest.js` takes id, title and URLs from `brand.json`.
- `repo.json` is `{"packages": []}` until the first LumaTV release regenerates
  it; it listed 0.3.6 under the old id.
- The ipk now also ships `palette.py`, `systemui.py`, `systemui_patch.py`; the
  old build left them out although `systemui on` and `webglass on` call them.
- `assets/splash.png` and `splash.svg` carry "LumaTV" as artwork (the PNG was
  edited, not regenerated from the SVG).
- `org.nphil.lumaglass_0.4.0_all.ipk` at the repo root was an untracked,
  gitignored build artefact (`*.ipk`); the release builds its own. Deleted
  locally.
- `payload/frame/` is no longer written by the build (the frame ipk is built
  under `build/` and copied into the package).
- Docs: README, CHANGELOG (Unreleased) and this file's HANDOFF "Resume here"
  updated; other docs and research files keep the old names.

## On-set migration (`migrate_legacy` in `tools/lumatv`)

Runs at the start of every verb except `version` (before `ensure_config`,
which would otherwise create the new data dir and hide the old one). It does
nothing unless `/var/lib/lumaglass` is a directory and `/var/lib/lumatv` does
not exist, or `/var/lib/lumatv/.migrating` is left from an interrupted run.

1. Unmount the settings-app stylesheet overlays if `.webglass-on` is set
   (their upper layer names the old dir).
2. Take an exclusive lock on the old `mutate.lock` (waits up to 180 s for an
   apply or revert of the old tool), write `.migrating`, then
   `mv /var/lib/lumaglass /var/lib/lumatv` (one rename on the same
   filesystem; nothing is logged before it). Failure leaves the old layout
   untouched and the verb returns an error.
3. Delete what is derived from old names and rebuilt by apply: `luma/`,
   `systemui/`, `keyfilter/lumaglass.js`, `.verified`, and `lumaglass.css`
   in each webapps upper dir.
4. Rewrite every `/var/lib/lumaglass` in `/var/systemd/system/env/surface-manager.env`
   and `kdump.env` to the new dir (temp file plus rename). If the early-boot
   shim exists it is regenerated (new app dir, data dir and tmp marker).
5. Remove the init.d link `50-lumaglass`; if `config.json` says
   `persist: true`, create `50-lumatv` and the early hook (`persist_enable`).
6. If the env file exists, register the key filter again so configd points at
   the new path. If `.systemui-on` is set, point Quick Settings at the new
   staged tree; if `.webglass-on` is set, mount the overlays again.
7. Remove apps `org.nphil.lumaglass.frame` and `org.nphil.lumaglass` with
   `appInstallService/dev/remove` (waits up to 30 s each for the directory to
   go; a timeout is logged, not fatal).
8. Delete `.migrating`, log `migrate: done`.

The first `apply` after that restarts the compositor once (it still runs with
the old import path) and rebuilds the shadow module under
`/var/lib/lumatv/qml/WebOSCompositor/lumatv`. `revert`, `persist off`,
`systemui off` and `webglass off` on a set that only has the old layout run the
migration first and then act on the moved data. Log lines are in
`/var/lib/lumatv/log` (`migrate: ...`).

A status call on first launch of the new app runs the migration, so that first
call can take about a minute (the two app removals).

Safety rules kept: no write to `/usr` or the rootfs; writes only to the data
dirs (old and new), `/tmp`, configd, bind/overlay mounts, the init.d link and
`/var/systemd/system/env`; SAM is not restarted. The migration adds two
things outside that list on purpose: the old data dir (moved) and the old apps
(removed through appInstallService).

## Not done / open

- Nothing run on the set. The migration was exercised in a sandbox copy of the
  tool with stubbed `luna-send`, `mount`, `python3`; the real
  `dev/remove` reply and timing are unverified.
- The GitHub repository and the Gitea repository are still named `lumaglass`
  (remotes). `brand.json` `repo` says `nphil/lumatv`; the release workflow
  needs the GitHub repo renamed to match.
- A moved set's compositor reads old `/var/lib/lumaglass` paths until the
  first apply restarts it; theme polls and lazy icons fail quietly in between.
- Icons (`assets/icon*.png`) carry no name and were not touched.

## Migration code (from tools/lumatv)

```sh
LEGACY_STATE=/var/lib/lumaglass
LEGACY_APP=org.nphil.lumaglass
LEGACY_FRAME_APP=org.nphil.lumaglass.frame
LEGACY_AUTOSTART_LINK=/var/lib/webosbrew/init.d/50-lumaglass
LEGACY_KEYFILTER=lumaglass.js
LEGACY_CSS=lumaglass.css
# Present in the new data dir from the move until every step below has run, so
# a power cut or a failed step is picked up again by the next verb.
MIGRATING="$STATE/.migrating"

rewrite_legacy_paths() {
  # $1 = file. Points every mention of the old data dir at the new one; the
  # file is replaced in one rename so a reader never sees a half-written line.
  [ -f "$1" ] || return 0
  grep -qF "$LEGACY_STATE" "$1" 2>/dev/null || return 0
  sed "s#$LEGACY_STATE#$STATE#g" "$1" > "$1.tmp.$$" && mv -f "$1.tmp.$$" "$1"
}

remove_legacy_app() {
  # $1 = app id. Removed through appInstallService, the way it was installed,
  # never by deleting its directory; done means the directory is gone.
  dir="$(dirname "$APPDIR")/$1"
  [ "$dir" != "$APPDIR" ] && [ -d "$dir" ] || return 0
  luna-send -n 3 -f luna://com.webos.appInstallService/dev/remove "{\"id\":\"$1\",\"subscribe\":true}" >/dev/null 2>&1 &
  lpid=$!
  w=0
  while [ -d "$dir" ] && [ "$w" -lt 30 ]; do sleep 1; w=$((w+1)); done
  kill "$lpid" 2>/dev/null
  if [ -d "$dir" ]; then
    log_line "migrate: $1 is still installed after 30s"
  else
    log_line "migrate: removed app $1"
  fi
  return 0
}

rewrite_legacy_paths() {
  # $1 = file. Points every mention of the old data dir at the new one; the
  # file is replaced in one rename so a reader never sees a half-written line.
  [ -f "$1" ] || return 0
  grep -qF "$LEGACY_STATE" "$1" 2>/dev/null || return 0
  sed "s#$LEGACY_STATE#$STATE#g" "$1" > "$1.tmp.$$" && mv -f "$1.tmp.$$" "$1"
}

remove_legacy_app() {
  # $1 = app id. Removed through appInstallService, the way it was installed,
  # never by deleting its directory; done means the directory is gone.
  dir="$(dirname "$APPDIR")/$1"
  [ "$dir" != "$APPDIR" ] && [ -d "$dir" ] || return 0
  luna-send -n 3 -f luna://com.webos.appInstallService/dev/remove "{\"id\":\"$1\",\"subscribe\":true}" >/dev/null 2>&1 &
  lpid=$!
  w=0
  while [ -d "$dir" ] && [ "$w" -lt 30 ]; do sleep 1; w=$((w+1)); done
  kill "$lpid" 2>/dev/null
  if [ -d "$dir" ]; then
    log_line "migrate: $1 is still installed after 30s"
  else
    log_line "migrate: removed app $1"
  fi
  return 0
}

migrate_legacy() {
  # Returns 1 only when the data could not be moved; every later step is
  # logged and skipped on failure, because the data is already in place and
  # apply puts the rest right.
  if [ -f "$MIGRATING" ]; then
    from="$STATE"
  elif [ "$LEGACY_STATE" != "$STATE" ] && [ -d "$LEGACY_STATE" ] && [ ! -L "$LEGACY_STATE" ] && [ ! -e "$STATE" ]; then
    from="$LEGACY_STATE"
  else
    return 0
  fi

  # The overlay mounts over the settings apps name the old data dir as their
  # upper layer. Unmounted before the move and mounted again after it.
  [ -f "$from/.webglass-on" ] && webglass_set off

  if [ "$from" = "$LEGACY_STATE" ]; then
    # Wait out an apply or revert of the old tool, and keep new ones out, for
    # as long as the move takes. Nothing is logged yet: log_line would create
    # the new data dir and the move would then never happen.
    exec 8>"$LEGACY_STATE/mutate.lock" 2>/dev/null
    if ! flock -x -w "$LOCK_WAIT" 8 2>/dev/null; then
      exec 8>&-
      printf '%s migrate: old tool still running, not moved\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" >> "$LEGACY_STATE/log" 2>/dev/null
      return 1
    fi
    : > "$LEGACY_STATE/.migrating"
    if ! mv "$LEGACY_STATE" "$STATE" 2>/dev/null; then
      rm -f "$LEGACY_STATE/.migrating"
      exec 8>&-
      printf '%s migrate: could not move to %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$STATE" >> "$LEGACY_STATE/log" 2>/dev/null
      return 1
    fi
    exec 8>&-
  fi
  log_line "migrate: data now in $STATE"

  # Everything derived from the old names is rebuilt by apply: the staged QML
  # directory, the key-filter copy, the settings-app stylesheet copy and the
  # patched Quick Settings tree (its marker comment changed with the name).
  rm -rf "$LUMA_STAGE" "$LUMA_STAGE.next" "$SYSTEMUI_STAGE" 2>/dev/null
  rm -f "$KEYFILTER_DIR/$LEGACY_KEYFILTER" "$VERIFIED" 2>/dev/null
  for f in "$WEBGLASS_STAGE"/*/upper/webOSUserScripts/"$LEGACY_CSS"; do
    [ -e "$f" ] && rm -f "$f"
  done

  # The compositor's env file and the early-boot env file hold the old data
  # dir; the early-boot shim holds it and the old app dir in its body, so it
  # is written again rather than patched.
  rewrite_legacy_paths "$ENV_FILE"
  rewrite_legacy_paths "$EARLY_ENV_FILE"
  if [ -x "$EARLY_SHIM" ]; then
    install_early_hook || log_line "migrate: early boot hook could not be rewritten"
  fi

  # The init.d link points into the old app, which goes below.
  rm -f "$LEGACY_AUTOSTART_LINK" 2>/dev/null
  if [ "$(cfg_get_bool persist)" = true ]; then
    persist_enable || log_line "migrate: boot hook could not be re-created"
  fi

  # configd still holds the old data dir in the key filter's path and in the
  # Quick Settings entry.
  if [ -f "$ENV_FILE" ]; then
    keyfilter_set add || log_line "migrate: could not register the key filter with configd"
  fi
  [ -f "$SYSTEMUI_ON" ] && { systemui_set on || log_line "migrate: could not point Quick Settings at the staged tree"; }
  [ -f "$WEBGLASS_ON" ] && { webglass_set on || log_line "migrate: could not mount the settings-app stylesheet"; }

  remove_legacy_app "$LEGACY_FRAME_APP"
  remove_legacy_app "$LEGACY_APP"
  rm -f "$MIGRATING"
  log_line "migrate: done"
  return 0
}

```
