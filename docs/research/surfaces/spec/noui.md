## No UI

Apps, files and services that draw nothing, or only host the surfaces in the
tables. Listed so that no app folder and no compositor view file is left out.

### Apps with no UI

- Developer samples for window groups and key masks, never launched on a
  retail set: `com.webos.exampleapp.enyoapp.epg`,
  `com.webos.exampleapp.groupowner`, `com.webos.exampleapp.nav`,
  `com.webos.exampleapp.qmlapp.client.negative.one`,
  `com.webos.exampleapp.qmlapp.client.negative.two`,
  `com.webos.exampleapp.qmlapp.client.positive.one`,
  `com.webos.exampleapp.qmlapp.client.positive.two`,
  `com.webos.exampleapp.qmlapp.discover`, `com.webos.exampleapp.qmlapp.epg`,
  `com.webos.exampleapp.qmlapp.hbbtv`, `com.webos.exampleapp.qmlapp.livetv`,
  `com.webos.exampleapp.qmlapp.search`, `com.webos.exampleapp.systemui` (a red
  rectangle fading every 5 s).
- Blank layer hosts that a broadcaster page would fill, dormant here (HbbTV
  off, US ATSC): `com.webos.app.livehbbtv`,
  `com.webos.app.liveinteractivecontent`, `com.webos.app.liveginga`.
- Transparent launchers that call a service and close:
  `com.webos.service.billing.app`, `com.webos.service.homeconnect.app`.

### Compositor files that host surfaces or help, with no pixels of their own

| File | Role |
|---|---|
| `views/StarfishViewsRoot.qml` | instantiates the 26 views; paint order is declaration order |
| `views/StarfishOverlayView.qml` | one overlay at a time: Settings, Guide, quick panel, input picker, Search |
| `views/StarfishOverlayGroupView.qml` | overlay that owns a window group; no app uses it here |
| `views/StarfishPopupView.qml`, `popup/StarfishPopupSurface.qml`, `popup/popuphandler.js` | movable popup windows (DVR popup, TV hotkey panel) |
| `views/StarfishScreenRemoteView.qml` | hosts `screenremote`, `contextmenu`, `toss` |
| `views/StarfishNudgeView.qml` | hosts nudge windows; blocked over Home, alerts, overlays, keyboard |
| `views/StarfishNotificationView.qml` | loads `SystemUI/Notification`; focus layers 7 and 8 |
| `views/StarfishScreenCoverView.qml` | hosts `bootlogo`, `masterbrand`, `woweffect` |
| `views/systemUi/StarfishSystemUIManager.qml`, `StarfishSystemUIContainer.qml`, `StarfishSystemUIComponentContainer.qml`, `StarfishSystemUIView.qml`, `StarfishSystemUIViewContainer.qml`, `StarfishSystemUIViewStateTransition.qml`, `StarfishSystemUIViewStateTransitionContainer.qml`, `StarfishSystemUIOutProcView.qml`, `StarfishSystemUIOutProcViewPlaceHolder.qml`, `StarfishSurfaceContainer.qml`, `StarfishViewContainer.qml`, `StarfishFocusableView.qml`, `StarfishOverlaySurface.qml`, `StarfishHomeSurface.qml` | load the system-UI QML named in configd `systemUIManager`; a failed load shows nothing, the out-of-process placeholder prints "Can not load <appId>" |
| `views/base/StarfishImage.qml`, `views/base/imageutils.js`, `views/containerController.js`, `views/punchThrough.js`, `views/fullscreen/fullscreencontainerhandler.js` | scaling, video punch-through and container helpers |
| `WebOSCompositorBase/views/ScreenFreezer.qml` | holds a snapshot of the UI for up to 2 s while the output geometry changes |
| `WebOSCompositorBase/views/Launcher.qml`, `NotificationView.qml`, `OverlayView.qml`, `base/BaseView.qml`, `base/FocusableView.qml`, `base/SurfaceView.qml`, `base/PopupSurface.qml`, `base/FocusHandler.js`, `base/popuphandler.js` | reference-compositor classes that `StarfishViewsRoot` overrides or never instantiates |
| `WebOSCompositorBase/views/notification/` `Alert.qml`, `AlertView.qml`, `Button.qml`, `CloseButton.qml`, `MarqueeText.qml`, `Notification.qml`, `NotificationModel.qml`, `NotificationService.qml`, `NotificationStack.qml`, `Popup.qml`, `PincodePrompt.qml`, `PincodePromptView.qml`, `PincodePromptBaseButton.qml`, `PincodePromptNumButton.qml`, `PincodePromptPasswordButton.qml`, `Toast.qml`, `ToastView.qml` | the reference notification set; the shipping one is `SystemUI/Notification` |
| `WebOSCompositorBase/views/debug/` `DebugOverlay.qml`, `DebugWindow.qml`, `FocusConsole.qml`, `FocusHighlight.qml`, `FpsGraphOverlay.qml`, `LogConsole.qml`, `ResourceMonitor.qml`, `SurfaceConsole.qml`, `TouchOverlay.qml`, `exportedItemDebug.qml` | developer overlays behind `debug.enable` (false) |

### QML modules and plumbing with no surface of their own

- `SystemUI/Notification`: `Services.qml`, `NotificationModel.qml`,
  `NotificationView.qml`, `NotificationStyle.qml`, `Notification.qml`,
  `NotificationStack.qml`, `FocusableView.qml`, `SystemUIView.qml`,
  `NLocaleService`, `Utils.js`; `Popup.qml` is the base class that owns the
  scrim, not a luna type; `Button.qml` and `PincodePromptBackButton.qml` are
  never instantiated; `MarqueeText.qml`, `ScrollText.qml`, `TextScroller.qml`,
  `ElideShader.qml` scroll long labels.
- Adaptation, interface and controller layers inside `QuickSettings`,
  `InputPicker`, `ContextMenu`, `ScreenRemote`, `BootLogo`, `MasterBrand`,
  `CoverUI`, `Nudge`, `SportsAlarm`, `StoreStreamer`, `FavoriteChannels`,
  `RemoteUserList` and `inputcommon` (113 interface files, handlers, layer
  containers, key routers, 33 shared widgets); five unreferenced Quick Settings
  components and unreferenced SportsAlarm and RemoteUserList helpers.
- Libraries: `Eos` (Moonstone controls, items, styles, three native plugins),
  `QmlAppComponents`, `QmlAppComponentsStarfish`, `WebOS` (developer
  consoles), `WebOSServices` (models), `iLib`, `Qt*`, `QtGraphicalEffects`,
  `QtMultimedia`, `AudioService`, `FpsCounter`, `KeyFilters` (JS),
  `PerformanceLog`, `PmLog`, `PmTrace`, `RdxLog`, `TextFile`, `Voice` (Opus
  player), `SystemUIComponent`, `SystemUIAssets` (volume and list icons).
- configd `systemUIManager` entries whose module is absent from the set:
  `com.webos.app.woweffect` (`ScreenCover`) and `com.webos.app.toss`
  (`Toss`); `com.webos.app.sportsalarm` and `com.webos.app.homegymnudge` have
  no `main`.
- Services that show nothing: `rdxd`, `uploadd`, `contentminer`,
  `remotelogger`, `sleepd`, `avmutemanager`, `lowlevelstorage`,
  `systemprofile`, `sysfaultdetector`, `activitymanager`, `logoutd`,
  `audiooutputd`, `devicereset`, `color-info-miner`, `ledbarcontroller`,
  `snapshot-boot-manager`, `browser-service` and the camera and microphone
  services; `flutter-wayland-client` only holds screen-reader labels.
