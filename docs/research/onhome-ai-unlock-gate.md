# Unlocking by an AI: the HomeLabber gate (design, 2026-10-30)

Nitin's rule: a person at the TV may unlock with the remote (a question first); no AI or automation path may unlock a lock unless Nitin approves that exact unlock on his phone with Face ID (Principle 0, `/data/home/Homelabber/design-notes/human-control-gate.md`, "unlocking doors: every time, once or plan, never leased"). Until the gate has the tool below, the AI paths simply refuse.

## What refuses today

- **TV helper:** `POST /v1/onhome/act` never unlocks or unlatches (`needs_shell`); `POST /v1/onhome/unlock` answers only a loopback peer that shows the token in `onhome/shell.json` (readable by the shell only). The AI access module (MCP), luna, voice and the plugins' lease have no call to a lock, and the lease refuses the `lock` domain whatever a manifest says (`plugins/ha.rs` `NEVER_CALLED`).
- **Home Assistant:** the LumaShow integration treats unlock and open as sensitive services (a PIN, `origin: touch`, a confirmation, and the entity's sensitive switch); any other origin is refused (`needs_touch`).
- **Still to close in Phase 1 of the gate note:** the ha-mcp server and Cody's Home Assistant token can call `lock.unlock` directly (step 5: a non-admin user still can; the allow-list in phase 3 removes locks and alarms). Until then this is the open door; it is not the TV's.

## The hook: one gated tool in HomeLabber

HomeLabber holds the Home Assistant admin token (gate note 5.11) and offers `ha_unlock(entity_id, reason)`:

1. An AI calls it. HomeLabber stores a pending request with the canonical action `{"type": "ha.lock.unlock", "entity_id": "lock.front_door"}`, the AI's stated reason (labelled as the AI's) and a 5-minute expiry, and answers "waiting for Nitin" with the id.
2. The HomeLabber app shows HomeLabber's own words ("Unlock Front door", risk: high, every time) and the reason. Face ID signs hash(action, id, nonce, expiry). Single use.
3. HomeLabber verifies, then itself calls `lock.unlock` on that entity through Home Assistant's API, journals it, and tells the AI the result. The TV is not in this path at all, so the TV's refusals stay absolute and there is no bypass to add there.
4. The rule row is "every time (once or plan; never leased)"; a plan may include an unlock as one shown step. A denied or expired request pauses that token as for any other.

What HomeLabber needs that it may not have yet: the gated tool above; an allow-list of lock entities it may unlock (Nitin's, in HomeLabber, never set by an AI); the Home Assistant admin token held by HomeLabber only. The TV needs nothing from it.

## The notification (Home Assistant, Nitin's to add)

The integration fires `lumashow_action` for every action of the TV, and Home Assistant's lock changes state for every unlock, from any source. The second catches HomeLabber's, a key and the lock's keypad too:

```yaml
alias: Door unlocked
trigger:
  - platform: state
    entity_id: [lock.front_door, lock.back_door, lock.downstairs_door_lock]
    to: unlocked
action:
  - service: notify.mobile_app_nitin_iphone
    data:
      title: A door was unlocked
      message: "{{ trigger.to_state.name }} was unlocked."
```

## The change `lumashow-ha` needs for a person at the TV

Today the TV's unlock ends with `pin_required`. For "a person pressing OK in the TV's question unlocks" the integration needs one option per device entry, off by default: *Unlock from this device with a confirmation only*. With it on, for a lock the person has allowed as sensitive, `_precheck_pin` accepts `origin: touch` with `confirmed: true` and no PIN, `ctl_pin` offers `unlock` without a PIN being set, and the existing `lumashow_action` event and audit line carry `lock.unlock` and `touch`. Every other origin stays refused, and so does every other device. The TV already sends exactly that call (`origin: touch`, `confirmed: true`, no `pin`).
