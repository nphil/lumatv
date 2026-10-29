# LG's Input Manager queue: lginput2's handshake with the compositor

## The symptom

Under LumaTV's compositor every reconnect of the Magic Remote (it sleeps when laid down, and its Bluetooth link drops) spent
4.0 s in lginput2's `STAGE_ADDING` before the remote worked; the first press after waking was lost. lginput2's log at each
reconnect (round 19, 21:45 UTC):

```
T       [_uhid_set_conn_device_info (1378)] {MRCU} :: [STAGE_NO_DEVICE] ===> [STAGE_ADDING]
T+2.0 s [IM_ReceiveMsgWithMq][431] The call timed out before a message could be transferred.
        [IM_MQ_Connect][300] IM_CallMethodSyncWithMq is Error(retVal:0)
        [mrcu_state_registerCursorCallback, 1725]
T+4.0 s [IM_ReceiveMsgWithMq][431] The call timed out before a message could be transferred.
        [IM_RegisterCursorInfoCallback][572] IM_CallMethodSyncWithMq is Error(retVal:0)
        [mrcu_state_registerCursorCallback, 1792] ret : FALSE
        [_mrcu_state_update_cursorOnTimer (1678)] Retry Cursor On Timer Enter
        [_uhid_add_device (1518)] {MRCU:"LGE MR24"} :: [STAGE_ADDING] ===> [STAGE_ADDED]
```

`/dev/mqueue/IM_MQ_WEBOS` held 2520 unread bytes (30 messages of 84, two per reconnect), `IM_MQ_MRCU` and `mq_IMOpenAPI_3737`
nothing; lginput2 (pid 3737) held 60 queue descriptors (30 `/IM_MQ_WEBOS`, 15 `/IM_MQ_MRCU`, 15 `/mq_IMOpenAPI_3737`), four more
per reconnect; nobody else held any. LG's `surface-manager` ran beside with no `libim` mapped and no queue descriptors.

## The cause

lginput2 links `libim` (`/usr/lib/libim.so.1.0.0`, "Input Manager", built 7 Jul 2025, exports every function by name). The library
has two halves. The compositor's half ran inside LG's `surface-manager`: `IM_MAIN_StartTasks` starts the threads `tMQMsgHandler`,
`tInput`, `tCursor` and `tTIMER`, which read the evdev devices, draw the cursor through the DRM cursor plane and keep lginput2 informed.
lginput2's half makes requests to it. They talk over POSIX message queues: every message is 84 bytes (`mq_msgsize` 84, `mq_maxmsg`
128, mode 0777), little-endian 32-bit words, word 0 the type (`IM_MQMSG_*`).

| Queue | Made by | Carries |
|---|---|---|
| `/IM_MQ_WEBOS` | the compositor (`handler_input_MsgHandleTask`, `O_CREAT\|O_EXCL`, reopened when it exists) | requests to the compositor: `{type, pid, argument}` |
| `/IM_MQ_MRCU` | lginput2 (`_IM_GetDeviceMqFd`) | the answer to the cursor registration, and the compositor's cursor notices |
| `/mq_IMOpenAPI_<pid>` | the caller (`IM_MQ_Connect`) | the answers to the "open API" calls |

The compositor's `tMQMsgHandler` blocks in `mq_receive` on `/IM_MQ_WEBOS` and dispatches on the type:

| Type | Name | Request | Answer |
|---|---|---|---|
| 0 | `REGISTER_INPUTDEVICE_MQ` | `{0, pid}` | on `/IM_MQ_MRCU`: `{0, 1, visibility, support, speed, standby ms}` |
| 1 | `REQUEST_TO_CHECK_INPUT` | `{1, pid}` | none (the compositor re-reads its input devices) |
| 5 | `REGISTER_OPENAPI_MQ` | `{5, pid}` | on the caller's queue: `{5, 1}` |
| 6 | `DEREGISTER_OPENAPI_MQ` | `{6, pid}` | none (the caller's queue is forgotten) |
| 7 | `SET_INPUT_DISPATCH_TYPE` | `{7, pid, type}` | `{7, 1}` |
| 8 | `GET_INPUT_DISPATCH_TYPE` | `{8, pid}` | `{8, 1, type}` |
| 11 | `STOP_WEBOS_TASK` | `{11}` | none (the threads stop) |
| 12 | `SET_DEACTIVATED_DEVICE_TYPE` | `{12, pid, type}` | `{12, 1}` |
| 13 | `ALLOW_TO_SET_DISPATCH_TYPE` | `{13, pid, allowed}` | `{13, 1}` |

Word 1 of an answer is the success the caller checks (`recv[4] == 1`); the open API answers go to `/mq_IMOpenAPI_<pid>` by name.
The other types are the compositor's notices to lginput2 on `/IM_MQ_MRCU`, read there by lginput2's `tDevMsgHandler` thread
(`IM_MAIN_InputDeviceMsgHandleTask`, started once a registration succeeded) and handed to the callbacks it registered:
2 `SEND_CURSOR_VISIBILITY {2, state}`, 3 `SEND_CURSOR_SUPPORT {3, bool}`, 4 `SEND_CURSOR_SPEED {4, speed}`,
9 `SEND_CURSOR_POSITION {9, x, y}`, 10 `SEND_CURSOR_STANDBY_TIME {10, ms}`, 14 `SEND_MRCU_CURSOR_ON {14, status}`.

A call is one request and one `mq_timedreceive` of its answer with a 2 s timeout (`IM_CallMethodSyncWithMq`). lginput2 imports three
functions and makes them from `mrcu_state_create`, inside the add of the remote:

1. `IM_GetInputDispatchType`: `IM_MQ_Connect` opens `/IM_MQ_WEBOS` (`O_RDWR`, no creation), makes `/mq_IMOpenAPI_<pid>`, sends
   `{5, pid}` and waits for `{5, 1}`; connected, it sends `{8, pid}` and waits for `{8, 1, type}`. A connection that succeeded is kept
   for the life of the process (`imOpenAPI mq exists and is connected`), so the type 5 request is sent once.
2. `IM_RegisterCursorInfoCallback(0, callbacks, &visibility, &support, &speed, &standby)`: makes `/IM_MQ_MRCU`, sends `{0, pid}`,
   waits for the answer there, starts `tDevMsgHandler`, and sets a flag after which the function answers "already registered" at once.
3. `IM_RequestToCheckInput`: `{1, pid}`, no answer, when lginput2 makes a device.

With LumaTV's compositor in LG's place nobody read `/IM_MQ_WEBOS`: each reconnect was two timeouts of 2 s, the requests stayed in the
queue, and the error paths of `IM_MQ_Connect` and `IM_RegisterCursorInfoCallback` return without closing what they opened (the four
descriptors per reconnect). Had the requests reached 128 the send itself would have blocked 2 s more.

What lginput2 does with the answers (`mrcu_state_registerCursorCallback`, the callbacks, `mrcu_button_event_callback`,
`mrcu_motion_checkCursorReShow`, all read off `/usr/sbin/lginput2`):

- The dispatch type must be 3 (`IM_INPUT_DISPATCH_ALL`, libim's default `_gInputDispatchType`): 0, 2 and 5 put the remote's state in
  its no-pointer state.
- Visibility 0 is shown, 1 hidden, 2 hidden with the remote sent to sleep (`lginput_devmngr_command(1)`, `COMMAND_SLEEP from external`),
  3 unknown. Support 1 applies `0x6d` (pointer on), 0 applies `0x6e`, 2 is left alone. Speed 3 is left alone, 0, 1 and 2 set the motion
  library's. The standby time is logged and never read.
- Told the pointer shows, lginput2 sends the wheel's press as `BTN_LEFT` (272, entry 0 of its key table) instead of `KEY_ENTER` (entry
  185), and `CUSTOM_KEY_CURSOR_OFF` (1199) before a navigation key. Told it is hidden, it sends `CUSTOM_KEY_CURSOR_ON` (1198) when
  the remote moves more than 300 pixels from the centre or the wheel turns, asking the compositor for the pointer. Never told, it
  keeps its initial state (2) and pulses 1198 every 0.5 s while the remote is active.
- `_mrcu_state_update_cursorOnTimer` (`Retry Cursor On Timer`) runs for 5 s after a failed registration and sends 1198 every 0.5 s
  while it believes the pointer hidden.

## The fix

`compositor/crates/lumatv-compositor/src/im_queue.rs`: the compositor is the reader of `/IM_MQ_WEBOS` while `--input` is on
(`--no-im-queue` turns it off). The queue is opened non-blocking (made with LG's attributes if it is missing; 10 messages where 128
is refused), its descriptor joins the loop's epoll, and every request is answered from the loop within the pass that read it:

- `{5, pid}` gets `{5, 1}` on `/mq_IMOpenAPI_<pid>`; `{8, pid}` gets `{8, 1, 3}`; 7, 12 and 13 get `{type, 1}` (7 is remembered for 8).
- `{0, pid}` gets `{0, 1, visibility, 1, 3, 10000}` on `/IM_MQ_MRCU`: the pointer's state (0 shown, 1 hidden), support kept on as LG's
  default, speed left to lginput2's own, and the compositor's hide timeout (`seat::CURSOR_TIMEOUT`).
- From then on (and from the start when `/IM_MQ_MRCU` exists: lginput2 registers with LG's compositor at boot and never again in its
  life) each change of the pointer's visibility is sent as `{2, 0}` or `{2, 1}`. A full queue (nobody reads it) drops the notice and
  stops them until the next registration. Hidden is 1, never 2: 2 puts the remote to sleep.
- Requests found waiting at the start are read and discarded (their callers gave up 2 s after sending), and so is anything waiting on
  a queue before an answer goes on it: libim checks only word 1 of what it reads, so a stale `{5, 1}` would pass for the answer to a
  later `GET_INPUT_DISPATCH_TYPE` (type 0: no pointer), and a stale notice `{2, 1}` for a registration's answer with support 0.
- `input.rs` takes `BTN_LEFT` from a device as the wheel's press: the click while the pointer shows, `KEY_ENTER` when it hid first.

`ctl status` reports it under `im`: `registered`, `requests`, `answered`, `unanswered`, `stale`, `purged`, `notices`.

Host tests: `tests/im_queue.rs` (a stand-in lginput2 over private queues makes the three calls with libim's timeout, the
notices follow the pointer, stale requests and answers are discarded, a full queue drops without blocking, `BTN_LEFT` is the click)
and the unit tests of `im_queue.rs` (the messages, from the captured bytes).

## The TV check

After the install, with the remote laid down until it sleeps (`MRCU_STATE ... SLEEP`) and picked up:

1. lginput2's log: `STAGE_ADDING` to `STAGE_ADDED` within 100 ms, no `IM_ReceiveMsgWithMq` timeout, no `Retry Cursor On Timer`, and once
   per lginput2 life ` set cursor status [visibility : 1] [support : 1] [speed : 3] [standby time : 10000msec]` and `ret : TRUE`.
2. `cat /dev/mqueue/IM_MQ_WEBOS`: `QSIZE:0`; `ls -l /proc/$(pidof lginput2)/fd | grep -c IM_MQ` does not grow across reconnects.
3. The compositor's journal: `im: REGISTER_OPENAPI_MQ from pid N ... sent`, `im: GET_INPUT_DISPATCH_TYPE ... sent`,
   `im: REGISTER_INPUTDEVICE_MQ ... cursor info ... sent`; `ctl status` `im.answered` equal to `im.requests`, `im.unanswered` 0.
4. The first press after waking reaches the set (a key in the journal within the same second as `STAGE_ADDED`).
5. With the pointer shown by the wheel, the wheel's press arrives as `BTN_LEFT` (272) and clicks; hidden, as `KEY_ENTER` (28) and is OK.
   The 1198 pulses stop while the pointer shows.
