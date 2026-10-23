# LG surface catalogue

Every screen, popup, overlay, banner, menu and notification the LG C4
(OLED65C4PUA, webOS 10.2.1) can show, with LumaTV's planned answer for each.
Companion to `PLAN.md`, whose Surfaces table names about twenty of them, and
to `mocks/`. Source: the firmware snapshot `/data/home/lgtv/uiinv`, the QML and
app files read from the set, and read-only luna getters.

## Resume here

{{RESUME}}

## Reading the tables

One row per surface, grouped by what Nitin sees. Columns: Surface (what it
shows) · LG id, kind and size · Seen · Plan · Answer with its reason · Hook ·
Ph.

- **Seen** is an estimate from how this set is used (Filmmaker mode, Denon
  AVR-S760H over eARC on HDMI 2, Live TV rare, Magic Remote pointer
  sometimes, Netflix, YouTube, Prime Video, Plex): `daily`, `sometimes`
  (about weekly), `rare` (a few times a year or less), `never` (cannot appear
  on this set, or only in factory, hotel or store modes). `[INFERENCE]` marks
  a frequency or fact not confirmed on the set.
- **Plan** is the coverage today: `Yes` named in the PLAN Surfaces table,
  `Part` named elsewhere in PLAN or only as a family, `Out` listed as not
  owned, `No` absent. A trailing `· mock NN` or `· no mock` says whether
  `mocks/` has the screen (24 screens, the list in `mocks/BATCH2.md`).
- **Answer**: Replace (LumaTV draws it), Restyle (LG's logic, LumaTV's look),
  Off (disabled or stubbed), Keep (LG's own, reason given). `Keep (dormant)`
  cannot appear on this set and needs no work.
- **Hook** uses the names of the PLAN Hooks table (System UI redirect, View
  swap, Shadow module, Overlay mount, Key filter and launch redirect) plus
  `Home stub` and `Launcher filter` (both in PLAN under Glasshouse), `LG
  setting` (switched off through settingsservice or configd), `Service off`
  (masked through Glasshouse) and `Overlay mount (theme)` (the shared web
  theme, below).
- **Ph** is the phase of PLAN (1-9). A `*` marks a phase proposed here
  because PLAN gives none; `—` means no work.
- **Kind** is web, qml, native, flutter, compositor-qml, system-qml, service
  or js. Sizes are the snapshot's `size_kb` (or `appsize` from `listApps`,
  or `du` on the set where the folder is not in the snapshot).
- Web apps: the `main.js` bundles of 43 of 44 checked LG web apps (Settings,
  First Use, Accessibility, the wizards, Scheduler, Game Optimizer, Media
  Player, Usage Care and the rest; the exception is the sign-language avatar)
  carry hundreds of Enact Sandstone strings, so the one shared theme of PLAN
  reaches every reachable one at no cost per app; those rows read Restyle.
  Apps that load a cloud page (Apps, LG Account, LG Channels, Premium) or
  draw their own UI (Netflix, Plex) are not themable that way.
