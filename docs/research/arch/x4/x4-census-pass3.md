## X4 census: pass 3: Netflix Meridian 4K Dolby Vision playback with 3 contiguous 1080p buffers held, 2026-10-24 21:40

5100 messages (2462 TX copies counted, 0 unparsable lines), 116.2 s, 21 messages/s; types: {'call': 665, 'return': 1257, 'callCancel': 540}

### Calls into the compositor's bus names (8 calls, 1 distinct caller/method pairs)

| Caller | Bus name | Method | Calls | First payload |
|---|---|---|---|---|
| `com.webos.app.home-8950` | `com.webos.service.recent` | `/listRecents` | 8 | `{}` |

### Calls the compositor makes (0 calls, 0 distinct)

| Destination | Method | Calls | First payload |
|---|---|---|---|

### tvpower / videooutput traffic touching the compositor

| t (s) | Type | From | To | Method | Payload |
|---|---|---|---|---|---|

### Busiest sender -> destination pairs on the whole bus (context)

- `com.palm.db` -> `com.lge.settingsservice`: 118 (1.0/s)
- `com.webos.service.settings` -> `com.webos.service.db`: 108 (0.9/s)
- `com.webos.settingsservice` -> `com.webos.service.db`: 98 (0.8/s)
- `com.webos.service.settings` -> `com.webos.accountdb`: 96 (0.8/s)
- `com.webos.service.audio` -> `com.webos.service.audiooutput`: 85 (0.7/s)
- `com.webos.service.oledepl` -> `com.webos.service.capture`: 76 (0.7/s)
- `com.palm.db` -> `com.webos.service.settings`: 54 (0.5/s)
- `com.palm.db` -> `com.webos.settingsservice`: 49 (0.4/s)
- `com.palm.webappmanager` -> `com.webos.memorymanager`: 48 (0.4/s)
- `com.webos.service.audiooutput` -> `com.webos.service.audio`: 47 (0.4/s)
- `com.webos.applicationManager` -> `com.webos.service.activitymanager`: 40 (0.3/s)
- `com.webos.service.capture` -> `com.webos.service.oledepl`: 38 (0.3/s)
- `com.webos.service.pqcontroller` -> `com.webos.settingsservice`: 30 (0.3/s)
- `com.webos.service.activitymanager.client` -> `com.webos.service.sleep`: 28 (0.2/s)
- `com.webos.service.pqcontroller.plugin` -> `com.webos.settingsservice`: 26 (0.2/s)
