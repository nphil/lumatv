## X4 census: x4 pass (dpad, qs, Settings overlay, browser, YouTube+Home), 2026-10-24

13033 messages (6490 TX copies counted, 0 unparsable lines), 181.1 s, 36 messages/s; types: {'call': 1408, 'return': 4028, 'callCancel': 1053, 'signal': 1}

### Calls into the compositor's bus names (30 calls, 8 distinct caller/method pairs)

| Caller | Bus name | Method | Calls | First payload |
|---|---|---|---|---|
| `com.webos.app.home-6492` | `com.webos.service.recent` | `/listRecents` | 20 | `{}` |
| `com.webos.starfish.keyfilters` | `com.webos.service.systemUIManager` | `/launch` | 3 | `{"appId":"com.webos.app.quicksettings"}` |
| `youtube.leanback.v4` | `com.webos.surfacemanager` | `/getLastInputType` | 2 | `{"subscribe":true}` |
| `com.palm.app.settings-13375` | `com.webos.surfacemanager` | `/getLastInputType` | 1 | `{"subscribe":true}` |
| `com.webos.app.browser` | `com.webos.surfacemanager` | `/getFocusedWindowInfo` | 1 | `{"subscribe":true}` |
| `com.webos.app.hdmi2` | `com.webos.surfacemanager.home` | `/getCurrentHomeUIState` | 1 | `{"subscribe":true}` |
| `com.webos.app.livetv` | `com.webos.surfacemanager.home` | `/getCurrentHomeUIState` | 1 | `{"subscribe":true}` |
| `youtube.leanback.v4` | `com.webos.surfacemanager` | `/getForegroundVideoWindow` | 1 | `{"subscribe":true}` |

### Calls the compositor makes (12 calls, 4 distinct)

| Destination | Method | Calls | First payload |
|---|---|---|---|
| `com.webos.service.videooutput` | `/display/setDisplayWindowProperty` | 5 | `{"context":"_0mXDZk7eQBALU4","appRotation":"Deg0"}` |
| `com.webos.service.multiviewcontroller` | `/setInputMode` | 4 | `{"appId":"","inputMode":false}` |
| `com.webos.applicationManager` | `/launchDefaultApp` | 2 | `{"category":"home","params":{"launchType":"homeKey","activateType":"home"}}` |
| `com.webos.applicationManager` | `/closeByAppId` | 1 | `{"reason":"windowPolicy","id":"com.palm.app.settings"}` |

### tvpower / videooutput traffic touching the compositor

| t (s) | Type | From | To | Method | Payload |
|---|---|---|---|---|---|
| 11.26 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"screen saver","sub processing":"screen on","state":"ACTIVE","lifeOnScreenMode":"alwaysR…` |
| 11.26 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"screen saver","sub processing":"screen on","state":"ACTIVE","lifeOnScreenMode":"alwaysR…` |
| 11.28 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","state":"ACTIVE"}` |
| 11.28 | return | `com.webos.service.tvpower` | `com.webos.surfacemanager` | `/?` | `{"returnValue":true,"sub state":"active","lifeOnScreenMode":"alwaysReady","state":"ACTIVE"}` |
| 128.14 | call | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/display/setDisplayWindowProperty` | `{"context":"_0mXDZk7eQBALU4","appRotation":"Deg0"}` |
| 128.14 | call | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/display/setDisplayWindowProperty` | `{"context":"_0mXDZk7eQBALU4","aspectRatio":"original"}` |
| 128.14 | call | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/display/setDisplayWindowProperty` | `{"context":"_0mXDZk7eQBALU4","fastMode":"on"}` |
| 128.14 | call | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/display/setDisplayWindowProperty` | `{"context":"_0mXDZk7eQBALU4","textureRotation":"Deg0"}` |
| 128.15 | callCancel | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/com/palm/luna/private/cancel` | `{"token":6022}` |
| 128.15 | callCancel | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/com/palm/luna/private/cancel` | `{"token":6023}` |
| 128.15 | callCancel | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/com/palm/luna/private/cancel` | `{"token":6024}` |
| 128.15 | callCancel | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/com/palm/luna/private/cancel` | `{"token":6025}` |
| 128.20 | call | `youtube.leanback.v4` | `com.webos.service.videooutput` | `/display/setDisplayWindowProperty` | `{"context":"_0mXDZk7eQBALU4","rotation":"Deg0"}` |
| 128.20 | call | `youtube.leanback.v4` | `com.webos.service.videooutput` | `/display/setDisplayWindow` | `{"context":"_0mXDZk7eQBALU4","fullScreen":false,"sourceInput":{"width":1920,"y":0,"x":0,"height":1080},"originalInput":{…` |
| 128.21 | call | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/display/setDisplayWindowProperty` | `{"context":"_0mXDZk7eQBALU4","textureRotation":"Deg0"}` |
| 128.23 | callCancel | `com.webos.surfacemanager` | `com.webos.service.videooutput` | `/com/palm/luna/private/cancel` | `{"token":6036}` |

### Subscriptions opened on the compositor during the window

- `youtube.leanback.v4` -> `com.webos.surfacemanager/getLastInputType` x2
- `com.palm.app.settings-13375` -> `com.webos.surfacemanager/getLastInputType` x1
- `com.webos.app.browser` -> `com.webos.surfacemanager/getFocusedWindowInfo` x1
- `youtube.leanback.v4` -> `com.webos.surfacemanager/getForegroundVideoWindow` x1
- `com.webos.app.livetv` -> `com.webos.surfacemanager.home/getCurrentHomeUIState` x1
- `com.webos.app.hdmi2` -> `com.webos.surfacemanager.home/getCurrentHomeUIState` x1

### Busiest sender -> destination pairs on the whole bus (context)

- `com.webos.settingsservice` -> `com.webos.service.db`: 214 (1.2/s)
- `com.palm.db` -> `com.lge.settingsservice`: 136 (0.8/s)
- `com.webos.surfacemanager` -> `com.webos.service.activitymanager`: 134 (0.7/s)
- `com.webos.service.oledepl` -> `com.webos.service.capture`: 116 (0.6/s)
- `com.webos.service.audio` -> `com.webos.service.audiooutput`: 113 (0.6/s)
- `com.webos.applicationManager` -> `com.webos.service.activitymanager`: 110 (0.6/s)
- `com.palm.db` -> `com.webos.settingsservice`: 107 (0.6/s)
- `com.webos.settingsservice` -> `com.webos.accountdb`: 98 (0.5/s)
- `com.webos.surfacemanager` -> `com.webos.service.multiviewcontroller`: 96 (0.5/s)
- `com.palm.db` -> `com.webos.service.settings`: 95 (0.5/s)
- `com.webos.applicationManager` -> `com.webos.memorymanager`: 90 (0.5/s)
- `com.webos.service.oledepl` -> `com.webos.platformstarfish`: 86 (0.5/s)
- `com.webos.accountdb` -> `com.webos.service.settings`: 84 (0.5/s)
- `com.webos.platformstarfish` -> `com.webos.service.oledepl`: 82 (0.5/s)
- `com.palm.webappmanager` -> `com.webos.memorymanager`: 72 (0.4/s)
