# Home Assistant gateway: port 3001 in Rust

The gateway plugin (`ssgw`, off until switched on in Settings › System › Home Assistant gateway) answers Home Assistant's LG webOS TV
integration, the second-screen (SSAP) websocket on port 3001, with `ssgw` (`helper/ssgw`) in place of LG's
`com.webos.service.secondscreen.gateway`. Design: `docs/research/ss-gateway-rust-plan.md`. Code: `helper/app/src/ssgw.rs` and
`tools/ssgw.sh`; page `rustui/ui/src/settings/pages/ssgw.rs`.

- **Port 3001:** xinetd keeps the port. Switched on, `ssgw.sh on` binds LumaTV's copy of `/etc/xinetd.d/ssg_https` over LG's, with
  `ssgw-accept` as its server, and has xinetd read it. `ssgw-accept` hands each connection to the gateway over
  `/tmp/lumatv-ssgw/accept.sock`, or runs LG's `/usr/bin/nsl` with LG's arguments on it when the gateway is not running, refuses or
  does not answer within 200 ms. Falling back needs nothing from the helper. Each fall back is kept by `ssgw-accept` itself in
  `/var/lib/lumatv/ssgw/fallback/` (below). The copy is bound only over the webOS 10.2.1 entry and
  `nsl` (sha256 pinned in `ssgw.sh`). A boot puts LG's entry back; the boot hook (`ssgw.sh early`) binds the copy again while
  `/var/lib/lumatv/ssgw/enabled` is there.
- **Gateway:** the helper runs `/var/lib/lumatv/ssgw/bin/ssgw` (staged by `lumatv` from `payload/ssgw/`) as uid/gid 19998, confined as
  plugin programs are with 256 open files, its client store in `ssgw/state` (0700). When `luna-send` does not answer that user, or the
  gateway exits with 78 as it, it runs as root. Every 10 s the helper pings `/tmp/lumatv-ssgw/control.sock`; three missed answers kill
  it. The gateway's log goes to the helper's log. A newly staged `ssgw` is started in place of the running one. LG's input sockets
  (`/tmp/netinput.{pointer,gamepad,touchpanel}.sock`) are root's alone: for a gateway on its own user the helper connects them on
  request and hands over the connection (`SSGW_BROKER_FD`), and connects nothing else.
- **Crashes:** an exit the helper did not ask for, a failed start or a hang is kept in `ssgw/crash/<time>-<n>/` (`reason.txt`, the last
  300 log lines in `gateway.log.tail`, the last URIs served in `recent.txt`; the newest 8 are kept), shown on the page until Mark as
  read, and sent once to the notify service on the page (default `notify.mobile_app_nitins_iphone`; empty sends nothing) through the
  TV's Home Assistant link. The gateway starts again after 2 s, doubling to 60 s. After 3 crashes within 10 min, or an exit with 78 as
  root (the TV's files or key are not the ones it was made for), the plugin stops trying until the next boot or Try again, and says
  so once; LG's gateway serves meanwhile.
- **Fall backs:** `ssgw-accept` cannot log (its standard streams are the client's socket), so each port 3001 connection it gives to
  LG's gateway is written to `fallback/<first time>-<n>.txt` (root's, 0700) before LG's gateway is run: the reason code and words
  (`no-run-dir`, `untrusted-run-dir`, `not-running`, `untrusted-peer`, `not-ready` for no ready byte in 200 ms, `refused` for a close
  without one, `bad-ready-byte`, `handoff-failed`), the run directory's mode and owner, whether `accept.sock` and `control.sock` exist,
  how long the hand-over was tried, the boot id and xinetd's arguments. Connections falling back for the same reason within 5 min of
  the last one (Home Assistant retrying against a stopped gateway) share one file with the first and last time and a count; the
  state lines are those of the last. The newest 16 files are kept, so records survive helper restarts and gateway crashes, and are
  gone with `/var/lib/lumatv`. A connection the gateway took, or dropped after taking it (LG's gateway does not answer that one),
  leaves none. The helper reads the newest every 10 min, logs it as `ssgw: <summary>` when its count changes and shows it on the page
  as "Last fall back to LG's gateway"; it tells the phone nothing about fall backs.
- **Forget clients:** deletes the gateway's client store; each Home Assistant key is checked with LG's gateway again on its next
  connection.
- **Off:** stops the gateway and has `ssgw.sh off` give the port back to LG's entry. `lumatv ssgw off` does the same without the
  helper and switches the plugin off; the escape's stock-UI restoration and `cutover.sh revert` run it. A stopping helper stops only the
  gateway, so connections go to LG's gateway until it is back.

The page also shows whether LG's service host repair (`lumatv sysbus`) is on, which decides what the fallback costs (about 22 % of a
core without it), and whether LG's extras are kept from loading (`lumatv lgextras`). With the extras off, declutter's boot hook
restarts LG's JavaScript host once when it still holds one of them after boot.

Soak, 2026-11-14 to 2026-11-21: no crash or hang, back by itself after 22 boots, 1.3 MB resident, no measurable idle CPU (HANDOFF
"Rust gateway seven-day soak").
