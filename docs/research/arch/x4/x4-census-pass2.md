## X4 census: pass 2: Netflix and PlxNative first screen, input picker, toast, Settings, standby and wake, 2026-10-24 20:15

15126 messages (7418 TX copies counted, 0 unparsable lines), 124.1 s, 60 messages/s; types: {'call': 1552, 'return': 4697, 'callCancel': 1169}

### Calls into the compositor's bus names (31 calls, 8 distinct caller/method pairs)

| Caller | Bus name | Method | Calls | First payload |
|---|---|---|---|---|
| `com.webos.app.home-7156` | `com.webos.service.recent` | `/listRecents` | 24 | `{}` |
| `com.palm.app.settings-6627` | `com.webos.surfacemanager` | `/getLastInputType` | 1 | `{"subscribe":true}` |
| `com.webos.app.hdmi2` | `com.webos.surfacemanager.home` | `/getCurrentHomeUIState` | 1 | `{"subscribe":true}` |
| `com.webos.app.quickinputpicker` | `com.webos.surfacemanager` | `/getLastInputType` | 1 | `{"subscribe":true}` |
| `com.webos.lunasend-2626` | `com.webos.surfacemanager` | `/showInputPicker` | 1 | `{}` |
| `com.webos.lunasend-2755` | `com.webos.service.systemUIManager` | `/closeByAppId` | 1 | `{"appId":"com.webos.app.quickinputpicker"}` |
| `com.webos.lunasend-4027` | `com.webos.surfacemanager` | `/getForegroundAppInfo` | 1 | `{}` |
| `com.webos.service.acr.client.async` | `com.webos.surfacemanager` | `/getForegroundVideoWindow` | 1 | `{"subscribe":true}` |

### Calls the compositor makes (1 calls, 1 distinct)

| Destination | Method | Calls | First payload |
|---|---|---|---|
| `com.webos.applicationManager` | `/closeByAppId` | 1 | `{"reason":"windowPolicy","id":"com.palm.app.settings"}` |

### tvpower / videooutput traffic touching the compositor

| t (s) | Type | From | To | Method | Payload |
|---|---|---|---|---|---|
| 4.02 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","state":"ACTIVE"}` |
| 4.02 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","state":"ACTIVE"}` |
| 56.91 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","sub processing":"request power off","state":"ACTIVE","lif…` |
| 56.91 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","sub processing":"request power off","state":"ACTIVE","lif…` |
| 56.97 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","sub processing":"request power off logo","state":"POWER O…` |
| 56.97 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","sub processing":"request power off logo","state":"POWER O…` |
| 56.98 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","timestamp":"544527936","…` |
| 60.24 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true}` |
| 60.24 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","sub processing":"request active standby","state":"POWER O…` |
| 60.24 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","sub processing":"request active standby","state":"POWER O…` |
| 60.30 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"sub state":"active","reason":"remoteKey","returnValue":true,"sub processing":"prepare power off","state":"POWER OFF","…` |
| 60.30 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"sub state":"active","reason":"remoteKey","returnValue":true,"sub processing":"prepare power off","state":"POWER OFF","…` |
| 60.30 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","next sub state":"active …` |
| 60.57 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true}` |
| 60.57 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active standby","lifeOnScreenMode":"alwaysReady","state":"POWER OFF"}` |
| 60.57 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active standby","lifeOnScreenMode":"alwaysReady","state":"POWER OFF"}` |
| 90.72 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"sub state":"active standby","reason":"remoteKey","returnValue":true,"sub processing":"prepare power on","state":"POWER…` |
| 90.72 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"sub state":"active standby","reason":"remoteKey","returnValue":true,"sub processing":"prepare power on","state":"POWER…` |
| 90.72 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active standby","lifeOnScreenMode":"alwaysReady","next sub state":…` |
| 90.80 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true}` |
| 90.81 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"sub state":"active standby","reason":"remoteKey","returnValue":true,"sub processing":"lastinput ready","state":"ACTIVE…` |
| 90.81 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"sub state":"active standby","reason":"remoteKey","returnValue":true,"sub processing":"lastinput ready","state":"ACTIVE…` |
| 90.81 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active standby","sub processing":"screen on","state":"ACTIVE","lif…` |
| 90.81 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"reason":"remoteKey","returnValue":true,"sub state":"active standby","sub processing":"screen on","state":"ACTIVE","lif…` |
| 90.95 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","state":"ACTIVE"}` |
| 90.95 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","state":"ACTIVE"}` |

### Subscriptions opened on the compositor during the window

- `com.webos.app.hdmi2` -> `com.webos.surfacemanager.home/getCurrentHomeUIState` x1
- `com.webos.app.quickinputpicker` -> `com.webos.surfacemanager/getLastInputType` x1
- `com.palm.app.settings-6627` -> `com.webos.surfacemanager/getLastInputType` x1
- `com.webos.service.acr.client.async` -> `com.webos.surfacemanager/getForegroundVideoWindow` x1

### Busiest sender -> destination pairs on the whole bus (context)

- `com.webos.service.audio` -> `com.webos.service.audiooutput`: 237 (1.9/s)
- `com.webos.service.audiooutput` -> `com.webos.service.audio`: 150 (1.2/s)
- `com.webos.applicationManager` -> `com.webos.service.activitymanager`: 127 (1.0/s)
- `com.webos.settingsservice` -> `com.webos.service.db`: 124 (1.0/s)
- `com.webos.service.tvpower` -> `com.webos.service.activitymanager`: 110 (0.9/s)
- `com.webos.service.preloadmanager` -> `com.webos.applicationManager`: 94 (0.8/s)
- `com.webos.settingsservice` -> `com.webos.accountdb`: 88 (0.7/s)
- `com.webos.service.oledepl` -> `com.webos.service.capture`: 78 (0.6/s)
- `amazon.alexa.adapter.plugin` -> `com.palm.db`: 71 (0.6/s)
- `com.palm.db` -> `amazon.alexa.adapter.plugin`: 71 (0.6/s)
- `com.webos.applicationManager` -> `com.webos.service.preloadmanager`: 69 (0.6/s)
- `com.webos.service.pushclient.req` -> `com.webos.service.db`: 68 (0.5/s)
- `com.palm.db` -> `com.webos.settingsservice`: 62 (0.5/s)
- `com.palm.db` -> `com.lge.settingsservice`: 60 (0.5/s)
- `com.webos.applicationManager` -> `com.webos.memorymanager`: 57 (0.5/s)
