# Do Not Disturb and snoozes

Do Not Disturb holds alerts: camera pop-ups, plugin notes and toasts from outside senders. Volume, input, receiver and LG's or LumaTV's own feedback toasts always show. A snooze holds one camera's alerts, or every camera's, for a while, with Do Not Disturb off.

## Gate

The helper owns the state and the gate (`helper/app/src/dnd.rs`). Every alert has a kind (`camera`, or `plugin:<id>`) and a source (the camera, the plugin's channel). It is held when a snooze of its source is on, or when Do Not Disturb or a snooze of its whole kind is on and it is not a doorbell let through. "Let doorbell through" (Settings, Home & look) is on unless turned off; a snooze of the doorbell's own camera holds it either way.

- A held camera pop-up is never published; its event goes to the missed list.
- A held plugin note is published with `held: true`: it is in the notification list and nothing announces it.
- The shell holds outside senders' toasts from LG's feed itself (`rustui/ui/src/system/toast.rs`) and records them with `POST /v1/dnd/missed`. Outside means any `sourceId` but LG's system (`com.webos.*`, `com.lge.*`) and LumaTV's; Home Assistant's `notify` arrives through `com.webos.service.secondscreen.gateway` and is held.

The missed list keeps the newest 20 and is emptied when Do Not Disturb is turned on again. The notification drawer shows it as its own group, "Do Not Disturb"; clearing the last of it sends `DELETE /v1/dnd/missed`.

## Times

Do Not Disturb lasts until turned off, 1 hour, 2 hours, or until tomorrow morning; a snooze 15 minutes, 1 hour, 4 hours, or until 07:00. "Morning" is the next 07:00 on the TV's clock: today's before 07:00, else tomorrow's. Times are absolute and kept with the snoozes and the missed list in `dnd.json` beside the settings, so a helper restart keeps them. One thread sleeps until the next end, and the gate ends what has run out before it answers, so a late wake never lets an alert through.

## Screens

- Quick Settings: the Do Not Disturb tile toggles it; its card picks a duration and lists the snoozes, OK on one ends it.
- Status bar: the do-not-disturb mark (a minus in a circle) with the time left while it is on, the bell-off glyph while only a snooze is on.
- Camera pop-up: OK held opens the snooze sheet (This camera or All camera alerts; 15 min, 1 hour, 4 hours, Until 07:00). OK, OK snoozes this camera for 1 hour; the pick closes the pop-up and a toast confirms it.

## API

| Request | Answer |
|---|---|
| `GET /v1/dnd` | the `dnd` topic: `{on, until, letDoorbell, snoozes: [{id, kind, source?, label, until}], missed: [{kind, source, title, body, at, by}]}` |
| `POST /v1/dnd/on` | `{until?: ms or "morning", minutes?}`; neither: until turned off |
| `POST /v1/dnd/off` | turns it off |
| `POST /v1/dnd/snooze` | `{kind, source?, label?, until? or minutes?}`; without `source` the whole kind; replaces a snooze of the same scope |
| `DELETE /v1/dnd/snooze/{id}` | ends a snooze |
| `POST /v1/dnd/settings` | `{letDoorbell}` |
| `POST /v1/dnd/missed` | `{kind, source?, title, body?}`: an alert held outside the helper |
| `DELETE /v1/dnd/missed` | empties the missed list |
