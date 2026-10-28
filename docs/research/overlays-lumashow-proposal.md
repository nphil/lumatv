# For LumaShow's plugin API: kinds, levels and scores on notifications

A proposal, not edited in LumaShow (`docs/plugin-api.md`, `crates/plugin-api`). It is what the TV's overlay cards (`overlays-result.md`) read where a
plugin says it, so that one record draws the same moment on a Show and on the TV. Everything is additive and optional: `ChannelDecl` and
`NotifyPost` do not deny unknown fields, so a base that does not know a field ignores it, and a plugin that sends none behaves as it does today.

## The fields

| Where | Field | Values | Meaning |
|---|---|---|---|
| `register.channels[]` | `kind` | `doorbell`, `alarm`, `timer`, `score`, `alert` | What moments the channel carries. A base may style and rank by it. Absent: `alert`. |
| `notify.post` | `kind` | the same | One post of another kind than its channel's (a channel of scores that also posts an alert). Absent: the channel's. |
| `notify.post` | `level` | `info`, `warning`, `critical` | How serious. The words are those of the LumaShow integration's `notify` module (`info`, `warning`, `critical`), so a Home Assistant message keeps its level across devices. Absent: `info`. |
| `notify.post` | `score` | see below | The two sides of a game, for `kind: score`. |

`score`, all of it optional but the two sides:

```json
{"t":"notify.post","id":"nba-g1","channel":"scores","title":"Lakers 102, Celtics 99","body":"Q4 2:31","level":"info","expires_in_secs":900,
 "score":{"league":"NBA","status":"Q4 2:31","live":true,
          "away":{"name":"Lakers","abbr":"LAL","score":102},"home":{"name":"Celtics","abbr":"BOS","score":99}}}
```

- Limits: `league` and `status` at most 32 characters, `name` 24, `abbr` 4, `score` an integer or a string of at most 8; the visitors are `away`, the home side `home` (a base lists the visitors first, as scores are read).
- `title` and `body` stay what a base that ignores `score` draws, so they must read on their own ("Lakers 102, Celtics 99" and "Q4 2:31").
- A game is one notification: the same `id` posted again with new numbers is the live score, not a second notice. A base is free to show an update in place and to hold back an update that follows another too soon (the TV shows a score at most every 30 s and refreshes the card while it is up).
- Nothing is called on the plugin for a score. A plugin that wants a button on the card puts an `actions` entry on the post as for any notification.

## What a base does with them (the TV's choices; a Show is free to choose its own)

| `kind` | The TV |
|---|---|
| `doorbell` | Top priority; shares the screen with the camera pop-up, and a doorbell record and a camera's doorbell pop-up within 20 s are one card. |
| `alarm`, `timer` | Top priority; up for 60 s and 30 s. A timer that finished on another device is a notification of this kind (a card at the `ringing` priority is read the same way where a plugin holds that grant). |
| `alert` | Second; up for 10 s (20 s for `critical`); the tone follows `level`. |
| `score` | Last; up for 8 s; one score at a time. |

`importance: low` is the drawer's alone on the TV, `privacy: secret` shows the channel's name and no words, and a notification that ended (`notify.end`) takes its card away.

## What the TV reads today without these fields

A channel called `doorbell`, `alarm` or `alarms`, `timer` or `timers`, `score`, `scores` or `sports` (or whose icon is `base.icons/doorbell`, `alarm-clock`, `timer` or `trophy`) is of that kind, anything else an `alert`. The home-assistant plugin's `doorbell` channel is a doorbell and its `ha` channel an alert. The kinds and the level only make the same thing explicit.

## The TV's helper

The helper publishes `plugin.<id>.notifications` items as `{id, channel, channelTitle, channelIcon, revision, receivedAt, expiresAt, post}`; `post` carries `kind`, `level` and `score` once `NotifyPost` does. One additive field of the helper's own, not the API's: `importance`, the channel's, on each item (`session.rs` `note_json`, about ten lines), so that a `low` channel is the drawer's alone. Until it is published a channel of any importance is a card.

## For the LumaShow integration (`lumashow-ha`, the `notify` module)

`notify.post` and the snapshot reach the TV's bridge as `Notice` (id, title, message) and are not drawn today (`helper/app/src/cameras.rs` `LinkSink::notices` drops them): the `level`, `ttl_s` and `source` of the message are lost at `ha_link::Sink`, which has no field for them. Carrying them (`Notice::level`, `Notice::ttl_s`) and publishing the notices as a first-party plugin's notifications would put every `lumashow.notify` on the TV with no token and no plugin to enable.
