# LG declutter: census, classification and the on-device tool

Owner: LGDeclutter2 (restart of LGDeclutter). Own paths: `tools/declutter.sh`,
`payload/declutter/**`, this note. Facts are from the set (LG OLED65C4PUA,
webOS 10.2.1) unless marked [INFERENCE].

## Resume here (2026-10-25 03:50 EDT)

- Census done and saved on the workstation (`/tmp/lt/dc/`: `census*.tgz`, `live/`
  for the live pass of 02:53, `dcl*.tgz`). Nothing was changed on the set yet.
- Tool written and dry-run checked on the workstation, not yet run on the set:
  `tools/declutter.sh`, `payload/declutter/{items.txt,groups.txt,dc.py,stub.sh}`.
- Next: ask Main for a changes slot; deploy the tool under
  `/var/lib/lumatv/dc-dev/` (mirrors the app layout: `tools/`, `payload/declutter/`,
  `brand.json`); baseline `measure`; then one group at a time (`uploaders`,
  `ads`, `push`, `settings`, `apps`, then the lean groups), each followed by a
  reboot and the verification list below. Record MemAvailable, resident size and
  CPU per group in the table under "Measured".
- Open questions the first slot answers: does `systemctl daemon-reload` after
  masking in `/run/systemd/system` behave at boot; do bind-mounted stubs hold
  under ls-hubd; does `appInstallService/remove` take a store stub app.
