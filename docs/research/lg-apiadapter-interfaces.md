# ss.apiadapter: luna interfaces it serves and routes (read from the set, 2026-11-11)

Input for a Rust replacement of `com.webos.service.apiadapter` (PLAN, After LumaTV). VIRTUAL interfaces are the public paths; the provider column is the service that answers each.

## VIRTUAL


### api (api.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getServiceList | com.webos.service.secondscreen.gateway/getServiceList | - |  |

### audio (audio.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getVolume | com.webos.service.audio/master/getVolume | - |  |
| /setVolume | com.webos.service.audio/master/setVolume | - |  |
| /volumeUp | com.webos.service.audio/master/volumeUp | - |  |
| /volumeDown | com.webos.service.audio/master/volumeDown | - |  |
| /getMute | com.webos.service.apiadapter/audio/getMute | - |  |
| /setMute | com.webos.service.apiadapter/audio/setMute | - |  |
| /getStatus | com.webos.service.apiadapter/audio/getStatus | - |  |
| /changeSoundOutput | com.webos.service.apiadapter/audio/changeSoundOutput | - |  |
| /getSoundOutput | com.webos.service.apiadapter/audio/getSoundOutput | - |  |

### config (config.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getConfigs | com.webos.service.apiadapter/config/getConfigs | - |  |

### externalpq (externalpq.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getExternalPqData | com.webos.service.apiadapter/externalpq/getExternalPqData | - |  |
| /setExternalPqData | com.webos.service.apiadapter/externalpq/setExternalPqData | - |  |

### media.controls (media.controls.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /play | com.webos.service.networkinput/controls/play | - |  |
| /pause | com.webos.service.networkinput/controls/pause | - |  |
| /stop | com.webos.service.networkinput/controls/stop | - |  |
| /fastForward | com.webos.service.networkinput/controls/fastForward | - |  |
| /rewind | com.webos.service.networkinput/controls/rewind | - |  |

### media.viewer (media.viewer.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /open | com.webos.service.apiadapter/media_viewer/open | - |  |
| /close | com.webos.service.apiadapter/media_viewer/close | - |  |

### pairing (pairing.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /setPin | com.webos.service.secondscreen.gateway/pairing/setPin | - |  |

### settings (settings.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getSystemSettings | com.webos.service.apiadapter/settings/getSystemSettings | - |  |
| /getSystemSettingsHDRInfo | com.webos.service.apiadapter/settings/getSystemSettingsHDRInfo | - |  |
| /getSystemSettingValues | com.webos.service.apiadapter/settings/getSystemSettingValues | - |  |
| /getSystemSettingDesc | com.webos.service.apiadapter/settings/getSystemSettingDesc | - |  |
| /setSystemSettings | com.webos.service.apiadapter/settings/setSystemSettings | - |  |
| /resetSystemSettings | com.webos.service.apiadapter/settings/resetSystemSettings | - |  |
| /resetSystemSettingDesc | com.webos.service.apiadapter/settings/resetSystemSettingDesc | - |  |

### system (system.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getSystemInfo | com.webos.service.apiadapter/system/getSystemInfo | - |  |
| /getCountryInfo | com.webos.service.apiadapter/system/getCountryInfo | - |  |
| /turnOff | com.webos.service.networkinput/controls/power | - |  |
| /getHostMessage | com.webos.service.secondscreen.gateway/getHostMessage | - |  |
| /getPowerState | com.webos.service.secondscreen.gateway/getPowerState | - |  |
| /doReset | com.webos.service.devicereset/doReset | STB_INTERNAL_CONNECTION |  |

### system.launcher (system.launcher.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /launch | com.webos.service.apiadapter/system_launcher/launch | - |  |
| /open | com.webos.service.apiadapter/system_launcher/open | - |  |
| /close | com.webos.service.apiadapter/system_launcher/close | - |  |
| /getAppState | com.webos.service.apiadapter/system_launcher/getAppState | - |  |

### system.notifications (system.notifications.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /createToast | com.webos.service.apiadapter/system_notifications/createToast | - |  |
| /createAlert | com.webos.service.apiadapter.pub/system_notifications/createAlert | - |  |
| /closeToast | com.webos.service.apiadapter/system_notifications/closeToast | - |  |
| /closeAlert | com.webos.service.apiadapter/system_notifications/closeAlert | - |  |

### timer (timer.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /setSettings | com.webos.service.apiadapter/timer/setSettings | - |  |
| /getSettings | com.webos.service.apiadapter/timer/getSettings | - |  |

### tv (tv.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getChannelList | com.webos.service.apiadapter/tv/getChannelList | - |  |
| /getTunerSettings | com.webos.service.apiadapter/tv/getTunerSettings | - |  |
| /getSimilarChannelList | com.webos.service.apiadapter/tv/getSimilarChannelList | - |  |
| /getCurrentChannel | com.webos.service.apiadapter/tv/getCurrentChannel | - |  |
| /getChannelProgramInfo | com.webos.service.apiadapter/tv/getChannelProgramInfo | - |  |
| /getChannelCurrentProgramInfo | com.webos.service.apiadapter/tv/getChannelCurrentProgramInfo | - |  |
| /getACRAuthToken | com.webos.service.apiadapter/tv/getACRAuthToken | - |  |
| /getContentState | com.webos.service.apiadapter/tv/getContentState | - |  |
| /openChannel | com.webos.service.apiadapter/tv/openChannel | - |  |
| /channelUp | com.webos.service.networkinput/controls/channelUp | - |  |
| /channelDown | com.webos.service.networkinput/controls/channelDown | - |  |
| /getExternalInputList | com.webos.service.apiadapter/tv/getExternalInputList | - |  |
| /switchInput | com.webos.service.apiadapter/tv/switchInput | - |  |
| /executeOneShot | com.webos.service.apiadapter/tv/executeOneShot | - |  |

### user (user.interface) {"version": 1}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /setUserInfo | com.webos.service.apiadapter/user/setUserInfo | - |  |
| /resetUserInfo | com.webos.service.apiadapter/user/resetUserInfo | - |  |
| /setUserData | com.webos.service.apiadapter/user/setUserData | - |  |
| /resetUserData | com.webos.service.apiadapter/user/resetUserData | - |  |
| /getUserData | com.webos.service.apiadapter/user/getUserData | - |  |
| /setUserSchedule | com.webos.service.apiadapter/user/setUserSchedule | - |  |
| /resetUserSchedule | com.webos.service.apiadapter/user/resetUserSchedule | - |  |

### webapp (webapp.interface) {"version": 2}

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /launchWebApp | com.webos.service.apiadapter/webapp/launchWebApp | - |  |
| /isWebAppPinned | com.webos.service.apiadapter/webapp/isWebAppPinned | - |  |
| /pinWebApp | com.webos.service.apiadapter/webapp/pinWebApp | - |  |
| /removePinnedWebApp | com.webos.service.apiadapter/webapp/removePinnedWebApp | - |  |
| /connectToApp | com.webos.service.secondscreen.gateway/app2app/connectToApp | - |  |
| /closeWebApp | com.webos.service.apiadapter/webapp/closeWebApp | - |  |

## CONCRETE


### com.palm.systemservice (com.palm.systemservice.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /time/getEffectiveBroadcastTime |  | CONTROL_INPUT_MEDIA_RECORDING |  |
| /time/getSystemTime |  | READ_SETTINGS |  |
| /getPreferences |  | READ_SETTINGS |  |
| /getPreferenceValues |  | READ_SETTINGS |  |
| /setPreferences |  | WRITE_SETTINGS |  |

### com.webos.applicationManager (com.webos.applicationManager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /launch |  | SECURE_LAUNCH,LAUNCH | M |
| /launchDefaultApp |  | LAUNCH | M |
| /closeByAppId |  | SECURE_LAUNCH,LAUNCH |  |
| /open |  | SECURE_LAUNCH,LAUNCH |  |
| /listApps |  | SECURE_READ_INSTALLED_APPS,READ_INSTALLED_APPS | RES:{"media":{"responseProperties":["apps.icon","apps.splashicon","apps.miniicon"]}} |
| /searchApps |  | SECURE_READ_INSTALLED_APPS,READ_INSTALLED_APPS | RES:{"media":{"responseProperties":["apps.icon"]}} |
| /getAppInfo |  | SECURE_READ_INSTALLED_APPS,READ_INSTALLED_APPS | RES:{"media":{"responseProperties":["appInfo.icon","appInfo.miniicon","appInfo.splashicon"]}} |
| /listLaunchPoints |  | READ_INSTALLED_APPS | RES:{"media":{"responseProperties":["launchPoints.icon","launchPoints.largeIcon","icon","largeIcon"]}} |
| /getForegroundAppInfo |  | READ_RUNNING_APPS |  |
| /getForegroundApps |  | READ_RUNNING_APPS |  |

### com.webos.launcher (com.webos.launcher.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /createGroup |  | ADD_LAUNCHER_CHANNEL |  |
| /addItemToGroup |  | ADD_LAUNCHER_CHANNEL |  |

### com.webos.media (com.webos.media.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getForegroundAppInfo |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |

### com.webos.notification (com.webos.notification.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /createPincodePrompt |  | WRITE_NOTIFICATION_TOAST |  |

### com.webos.service.apiadapter (com.webos.service.apiadapter.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /audio/getMute |  | CONTROL_AUDIO |  |
| /audio/setMute |  | CONTROL_AUDIO |  |
| /audio/getStatus |  | CONTROL_AUDIO |  |
| /audio/changeSoundOutput |  | CONTROL_AUDIO |  |
| /audio/getSoundOutput |  | CONTROL_AUDIO |  |
| /media_viewer/open |  | LAUNCH | CI M |
| /media_viewer/close |  | LAUNCH | CI M |
| /system_launcher/launch |  | LAUNCH | CI M |
| /system_launcher/open |  | LAUNCH | CI M |
| /system_launcher/close |  | LAUNCH | CI M |
| /system_launcher/getAppState |  | LAUNCH | CI |
| /system_notifications/createToast |  | WRITE_NOTIFICATION_TOAST | M |
| /system_notifications/createAlert |  | WRITE_NOTIFICATION_TOAST | M |
| /system_notifications/closeToast |  | WRITE_NOTIFICATION_TOAST | M |
| /system_notifications/closeAlert |  | WRITE_NOTIFICATION_TOAST | M |
| /system/getSystemInfo |  | - |  |
| /system/getCountryInfo |  | READ_COUNTRY_INFO |  |
| /system/isValidPin |  | READ_SETTINGS |  |
| /tv/getChannelList |  | READ_TV_CHANNEL_LIST |  |
| /tv/getSimilarChannelList |  | READ_TV_CHANNEL_LIST |  |
| /tv/getCurrentChannel |  | READ_CURRENT_CHANNEL |  |
| /tv/getChannelProgramInfo |  | READ_CURRENT_CHANNEL |  |
| /tv/getChannelCurrentProgramInfo |  | READ_CURRENT_CHANNEL |  |
| /tv/getACRAuthToken |  | READ_TV_ACR_AUTH_TOKEN | M |
| /tv/getContentState |  | READ_TV_CONTENT_STATE | M |
| /tv/openChannel |  | LAUNCH | M |
| /tv/getExternalInputList |  | READ_INPUT_DEVICE_LIST | RES:{"media":{"responseProperties":["devices.icon"]}} |
| /tv/switchInput |  | LAUNCH | M |
| /tv/executeOneShot |  | CONTROL_DISPLAY | RES:{"media":{"responseProperties":["imageUri"]}} |
| /webapp/launchWebApp |  | LAUNCH_WEBAPP | CI M |
| /webapp/isWebAppPinned |  | LAUNCH_WEBAPP | CI |
| /webapp/pinWebApp |  | LAUNCH_WEBAPP | CI M |
| /webapp/removePinnedWebApp |  | LAUNCH_WEBAPP | CI |
| /webapp/closeWebApp |  | LAUNCH_WEBAPP | CI M |
| /user/setUserInfo |  | CONTROL_USER_INFO | CI M |
| /user/resetUserInfo |  | CONTROL_USER_INFO | CI M |
| /user/setUserData |  | CONTROL_USER_INFO | CI M |
| /user/getUserData |  | CONTROL_USER_INFO | CI M |
| /user/resetUserData |  | CONTROL_USER_INFO | CI M |
| /user/setUserSchedule |  | CONTROL_USER_INFO | CI M |
| /user/resetUserSchedule |  | CONTROL_USER_INFO | CI M |
| /timer/setSettings |  | CONTROL_TIMER_INFO | CI |
| /timer/getSettings |  | CONTROL_TIMER_INFO | CI |
| /config/getConfigs |  | READ_SETTINGS | CI |
| /settings/getSystemSettings |  | READ_SETTINGS | CI |
| /settings/getSystemSettingsHDRInfo |  | READ_SETTINGS | CI |
| /settings/getSystemSettingDesc |  | READ_SETTINGS | CI |
| /settings/getSystemSettingValues |  | READ_SETTINGS | CI |
| /settings/setSystemSettings |  | WRITE_SETTINGS | CI |
| /settings/resetSystemSettings |  | WRITE_SETTINGS | CI |
| /settings/resetSystemSettingDesc |  | WRITE_SETTINGS | CI |
| /externalpq/getExternalPqData |  | READ_SETTINGS |  |
| /externalpq/setExternalPqData |  | WRITE_SETTINGS |  |
| /systemprofile/getProfiles |  | READ_SETTINGS | CI |

### com.webos.service.apiadapter.pub (com.webos.service.apiadapter.pub.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /system_notifications/createAlert |  | WRITE_NOTIFICATION_TOAST | M |
| /application_manager/launch |  | SECURE_LAUNCH,LAUNCH | M |

### com.webos.service.apluspolicymanager (com.webos.service.apluspolicymanager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /launchPartialViewApp |  | SECURE_CONTROL_TV_SCREEN,CONTROL_TV_SCREEN |  |

### com.webos.service.appcasting (com.webos.service.appcasting.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getCommand |  | READ_NETWORK_STATE |  |
| /sendCommand |  | READ_NETWORK_STATE |  |
| /receiveUIBCEvent |  | READ_NETWORK_STATE |  |

### com.webos.service.asc (com.webos.service.asc.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /registerDevice |  | ACCESS_STORAGE |  |

### com.webos.service.attachedstoragemanager (com.webos.service.attachedstoragemanager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /listDevices |  | SECURE_READ_STORAGE_DEVICE_LIST,READ_STORAGE_DEVICE_LIST |  |
| /changeDeviceStatus |  | SECURE_READ_STORAGE_DEVICE_LIST,READ_STORAGE_DEVICE_LIST |  |
| /getProperties |  | SECURE_READ_STORAGE_DEVICE_LIST,READ_STORAGE_DEVICE_LIST |  |

### com.webos.service.audio (com.webos.service.audio.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /tv/setOffsetDelay |  | CONTROL_AUDIO |  |
| /master/getVolume |  | CONTROL_AUDIO |  |
| /master/setVolume |  | CONTROL_AUDIO |  |
| /master/volumeUp |  | CONTROL_AUDIO |  |
| /master/volumeDown |  | CONTROL_AUDIO |  |

### com.webos.service.bluetooth (com.webos.service.bluetooth.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /gap/findDevices |  | CONTROL_BLUETOOTH |  |
| /gap/removeTrustedDevice |  | CONTROL_BLUETOOTH |  |
| /service/connect |  | CONTROL_BLUETOOTH |  |
| /service/disconnect |  | CONTROL_BLUETOOTH |  |
| /gap/getTrustedDevices |  | CONTROL_BLUETOOTH |  |
| /gap/isRadioOn |  | CHECK_BLUETOOTH_DEVICE |  |
| /service/init |  | CONTROL_BLUETOOTH |  |
| /service/release |  | CONTROL_BLUETOOTH |  |
| /service/subscribeNotifications |  | CONTROL_BLUETOOTH |  |
| /gap/isWiFiOnly |  | CHECK_BLUETOOTH_DEVICE |  |
| /audio/enableSoundOutRedirect |  | CONTROL_BLUETOOTH |  |
| /audio/getSoundOutRedirectState |  | CHECK_BLUETOOTH_DEVICE |  |

### com.webos.service.bluetooth2 (com.webos.service.bluetooth2.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /adapter/queryAvailable |  | CONTROL_BLUETOOTH |  |

### com.webos.service.bosp (com.webos.service.bosp.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getCurrentPictureMode |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /getCurrentAspectRatio |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /getPictureModeList |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /getAspectRatioList |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /setPictureMode |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /setAspectRatio |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /getUITransparency |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /getLGUIState |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /power |  | SECURE_CONTROL_DISPLAY,CONTROL_POWER |  |
| /scb/getRequestedAPI |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /scb/responseHDMILabel |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /scb/responseFirstApp |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /scb/responseNetworkStatus |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /scb/responseLuminence |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /scb/requestBacklight |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /scb/removeSplashScreen |  | SECURE_APP_TO_APP,APP_TO_APP |  |

### com.webos.service.btaudiosnk (com.webos.service.btaudiosnk.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /open |  | CONTROL_BLUETOOTH |  |
| /close |  | CONTROL_BLUETOOTH |  |
| /getState |  | CONTROL_BLUETOOTH |  |
| /disconnect |  | CONTROL_BLUETOOTH |  |

### com.webos.service.btaudiosrc (com.webos.service.btaudiosrc.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /connect |  | CONTROL_BLUETOOTH |  |
| /disconnect |  | CONTROL_BLUETOOTH |  |
| /getConnectedDevice |  | CONTROL_BLUETOOTH |  |
| /getPairedDevices |  | CONTROL_BLUETOOTH |  |

### com.webos.service.capture (com.webos.service.capture.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /executeOneShot |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |

### com.webos.service.cis (com.webos.service.cis.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /sampleSubscribe |  | READ_SETTINGS |  |
| /getState |  | READ_SETTINGS |  |
| /getPairedDevices |  | READ_SETTINGS |  |
| /sendVideo |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /getVideo |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /responseConfirmSendVideo |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /cancelGetVideo |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /cancelSendVideo |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /notifyEvent |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /setSettings |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /getChannelCurrentInfo |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /seamlessPairing |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /sendR2Rkey |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /selectRoomType |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /getAvailableRoomType |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /setDeviceState |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /confirmGetVideo |  | SECURE_APP_TO_APP,APP_TO_APP |  |
| /confirmSendVideo |  | SECURE_APP_TO_APP,APP_TO_APP |  |

### com.webos.service.connectionmanager (com.webos.service.connectionmanager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getinfo |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /checkinternetstatus |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getstatus |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getUserStatus |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /tv/createVirtualLan |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /tv/setExternalWanStatus |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |

### com.webos.service.devicereset (com.webos.service.devicereset.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /doReset |  | STB_INTERNAL_CONNECTION |  |
| /getTvRegion |  | READ_SETTINGS |  |
| /requestReboot |  | SECURE_CONTROL_POWER,CONTROL_POWER |  |

### com.webos.service.downloadmanager (com.webos.service.downloadmanager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /download |  | ACCESS_STORAGE |  |
| /cancelAllDownloads |  | ACCESS_STORAGE |  |

### com.webos.service.eim (com.webos.service.eim.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getCurrentInput |  | SECURE_READ_INPUT_DEVICE_LIST,SECURE_READ_RUNNING_APPS,READ_INPUT_DEVICE_LIST,READ_RUNNING_APPS |  |
| /getLastInput |  | SECURE_READ_INPUT_DEVICE_LIST,SECURE_READ_RUNNING_APPS,READ_INPUT_DEVICE_LIST,READ_RUNNING_APPS |  |
| /getAllInputStatus |  | SECURE_READ_INPUT_DEVICE_LIST,SECURE_READ_RUNNING_APPS,READ_INPUT_DEVICE_LIST,READ_RUNNING_APPS |  |
| /getHDMISignalInfo |  | SECURE_READ_INPUT_DEVICE_LIST,READ_INPUT_DEVICE_LIST |  |
| /setDeviceInfo |  | WRITE_SETTINGS |  |

### com.webos.service.firstuse (com.webos.service.firstuse.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getSettings |  | READ_SETTINGS |  |
| /setSettings |  | WRITE_SETTINGS |  |

### com.webos.service.homelaunchpoints (com.webos.service.homelaunchpoints.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /listLaunchPoints |  | READ_INSTALLED_APPS | RES:{"media":{"responseProperties":["launchPoints.icon","launchPoints.largeIcon","icon","largeIcon"]}} |
| /listAvailableLaunchPoints |  | READ_INSTALLED_APPS | RES:{"media":{"responseProperties":["launchPoints.icon","launchPoints.largeIcon","icon","largeIcon"]}} |

### com.webos.service.ics (com.webos.service.ics.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /blaster/startDetectDevices |  | SECURE_READ_RUNNING_APPS,READ_RUNNING_APPS |  |

### com.webos.service.iepg (com.webos.service.iepg.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /notifyChannelListChanged |  | SECURE_READ_TV_CHANNEL_LIST,READ_TV_CHANNEL_LIST |  |
| /getEventList |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |
| /getChannelList |  | SECURE_READ_TV_CHANNEL_LIST,READ_TV_CHANNEL_LIST |  |
| /setChannelSkip |  | SECURE_SET_CHANNEL_SKIP,SET_CHANNEL_SKIP |  |
| /releaseChannelSkip |  | SECURE_RELEASE_CHANNEL_SKIP,RELEASE_CHANNEL_SKIP |  |
| /setChannelBlock |  | SECURE_CONTROL_CHANNEL_BLOCK,CONTROL_CHANNEL_BLOCK |  |
| /releaseChannelBlock |  | SECURE_CONTROL_CHANNEL_BLOCK,CONTROL_CHANNEL_BLOCK |  |
| /setChannelFavoriteGroup |  | SECURE_CONTROL_FAVORITE_GROUP,CONTROL_FAVORITE_GROUP |  |
| /getFavoriteGroupList |  | SECURE_CONTROL_FAVORITE_GROUP,CONTROL_FAVORITE_GROUP |  |
| /releaseChannelFavoriteGroup |  | SECURE_CONTROL_FAVORITE_GROUP,CONTROL_FAVORITE_GROUP |  |
| /deleteChannel |  | SECURE_DELETE_SELECT_CHANNEL,DELETE_SELECT_CHANNEL |  |
| /getCountryPolicy |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |

### com.webos.service.ime (com.webos.service.ime.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /registerRemoteKeyboard |  | CONTROL_INPUT_TEXT |  |
| /insertText |  | CONTROL_INPUT_TEXT |  |
| /deleteCharacters |  | CONTROL_INPUT_TEXT |  |
| /sendEnterKey |  | CONTROL_INPUT_TEXT |  |

### com.webos.service.iotclient (com.webos.service.iotclient.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /registerToken |  | READ_LGE_SDX |  |
| /getAccountInfoList |  | READ_LGE_SDX |  |
| /getDeviceUuid |  | READ_LGE_SDX |  |
| /updateDeviceName |  | READ_LGE_SDX |  |
| /updateAccountInfo |  | READ_LGE_SDX |  |
| /checkSupport |  | READ_LGE_SDX |  |

### com.webos.service.irdbmanager (com.webos.service.irdbmanager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getDevices |  | SECURE_READ_INPUT_DEVICE_LIST,READ_INPUT_DEVICE_LIST |  |
| /isSupported |  | SECURE_READ_INPUT_DEVICE_LIST,READ_INPUT_DEVICE_LIST |  |

### com.webos.service.livepick (com.webos.service.livepick.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /isSupport |  | CONTROL_DISPLAY |  |
| /isPickable |  | CONTROL_DISPLAY |  |
| /isCapturable |  | CONTROL_DISPLAY |  |
| /checkInformation |  | SEARCH |  |
| /getSceneInfo |  | SEARCH |  |
| /getTotalInfo |  | SEARCH |  |
| /setWishCart |  | SEARCH |  |
| /getWishCart |  | SEARCH |  |
| /getTotalWishCount |  | SEARCH |  |

### com.webos.service.miracast (com.webos.service.miracast.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /sendRTSPRequest |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /setWFDConnectionMode |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getWFDConnectionMode |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getState |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /launchOverlayApp |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getConnectedDeviceInfo |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getDeviceInfo |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |

### com.webos.service.miracasttx (com.webos.service.miracasttx.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /connect |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /requestMsg |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getDeviceInfo |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |
| /getState |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |

### com.webos.service.mrcu (com.webos.service.mrcu.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /isPaired |  | CHECK_BLUETOOTH_DEVICE |  |

### com.webos.service.networkinput (com.webos.service.networkinput.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getPointerInputSocket |  | CONTROL_MOUSE_AND_KEYBOARD | RES:{"socket":{"responseProperties":["socketPath"],"framing":"LENGTH_PREFIX_32"}} |
| /getGamepadInputSocket |  | CONTROL_INPUT_JOYSTICK | RES:{"socket":{"responseProperties":["socketPath"],"framing":"LENGTH_PREFIX_32"}} |
| /sendKeyCodes |  | CONTROL_MOUSE_AND_KEYBOARD |  |
| /test/getTouchpanelInputSocket |  | CONTROL_MOUSE_AND_KEYBOARD | RES:{"socket":{"responseProperties":["socketPath"],"framing":"LENGTH_PREFIX_32"}} |
| /controls/play |  | CONTROL_INPUT_MEDIA_PLAYBACK |  |
| /controls/pause |  | CONTROL_INPUT_MEDIA_PLAYBACK |  |
| /controls/stop |  | CONTROL_INPUT_MEDIA_PLAYBACK |  |
| /controls/fastForward |  | CONTROL_INPUT_MEDIA_PLAYBACK |  |
| /controls/rewind |  | CONTROL_INPUT_MEDIA_PLAYBACK |  |
| /controls/channelUp |  | CONTROL_INPUT_TV |  |
| /controls/channelDown |  | CONTROL_INPUT_TV |  |
| /controls/power |  | CONTROL_POWER |  |

### com.webos.service.nlpmanager (com.webos.service.nlpmanager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /tips |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |
| /tipsCategory |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |
| /cancel |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |
| /searchMore |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |
| /listDynamicCategory |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |
| /getAllTips |  | SECURE_READ_TV_PROGRAM_INFO,READ_TV_PROGRAM_INFO |  |

### com.webos.service.panelcontroller (com.webos.service.panelcontroller.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /setScreenOnOff |  | SECURE_CONTROL_TV_SCREEN,CONTROL_TV_SCREEN |  |
| /getPanelResolution |  | READ_SETTINGS |  |

### com.webos.service.pushclient (com.webos.service.pushclient.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getStatus |  | SECURE_ACCESS_PUSH,ACCESS_PUSH |  |
| /registerService |  | SECURE_ACCESS_PUSH,ACCESS_PUSH |  |
| /subscribeToTopics |  | SECURE_ACCESS_PUSH,ACCESS_PUSH |  |

### com.webos.service.reclistmgr.list (com.webos.service.reclistmgr.list.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getRecordFileList |  | READ_RECORDING_LIST |  |
| /deleteRecordFile |  | WRITE_RECORDING_LIST |  |

### com.webos.service.rollingscreen (com.webos.service.rollingscreen.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /changeView |  | SECURE_CONTROL_TV_SCREEN,CONTROL_TV_SCREEN |  |
| /getViewState |  | SECURE_CONTROL_TV_SCREEN,CONTROL_TV_SCREEN |  |
| /pressUpDown |  | SECURE_CONTROL_TV_SCREEN,CONTROL_TV_SCREEN |  |

### com.webos.service.sdx (com.webos.service.sdx.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getHttpHeaderForServiceRequest |  | READ_LGE_SDX |  |
| /getDeviceAuthenticationStatus |  | READ_LGE_SDX |  |
| /send |  | READ_LGE_SDX |  |
| /getEulaDownloadStatus |  | READ_LGE_SDX |  |
| /requestEulaDownload |  | READ_LGE_SDX |  |

### com.webos.service.secondscreen.gateway (com.webos.service.secondscreen.gateway.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /test/getTime |  | TEST_OPEN |  |
| /test/protected |  | TEST_PROTECTED |  |
| /test/secure |  | TEST_SECURE |  |
| /getServiceList |  | - |  |
| /pairing/setPin |  | - | CI |
| /app2app/connectToApp |  | APP_TO_APP | CI |
| /getHostMessage |  | APP_TO_APP | CI |
| /getPowerState |  | READ_SETTINGS | CI |

### com.webos.service.sm (com.webos.service.sm.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /crypto/decryptFile |  | ACCESS_STORAGE |  |

### com.webos.service.soundshare (com.webos.service.soundshare.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getExecutableStatus |  | CONTROL_AUDIO |  |
| /getStreamOutStatus |  | CONTROL_AUDIO |  |
| /startStreamOut |  | CONTROL_AUDIO |  |
| /stopStreamOut |  | CONTROL_AUDIO |  |
| /setPcmBuffer |  | CONTROL_AUDIO |  |
| /getStreamInStatus |  | CONTROL_AUDIO |  |
| /startStreamIn |  | CONTROL_AUDIO |  |
| /stopStreamIn |  | CONTROL_AUDIO |  |

### com.webos.service.systemprofile (com.webos.service.systemprofile.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getProfiles |  | READ_SETTINGS |  |

### com.webos.service.tvReservationAgent (com.webos.service.tvReservationAgent.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getRepeatedList |  | READ_RECORDING_SCHEDULE |  |
| /watch/fullEpgProgram/add |  | READ_RECORDING_SCHEDULE |  |
| /watch/epgProgram/add |  | SECURE_WRITE_RECORDING_SCHEDULE,WRITE_RECORDING_SCHEDULE |  |
| /cancel |  | SECURE_WRITE_RECORDING_SCHEDULE,WRITE_RECORDING_SCHEDULE |  |
| /delete |  | SECURE_WRITE_RECORDING_SCHEDULE,WRITE_RECORDING_SCHEDULE |  |

### com.webos.service.tvpower (com.webos.service.tvpower.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /power/turnOnScreen |  | CONTROL_TV_SCREEN |  |
| /power/turnOffScreen |  | CONTROL_TV_SCREEN |  |
| /power/getPowerState |  | CONTROL_POWER |  |
| /power/getPowerOnReason |  | CONTROL_POWER |  |
| /power/shutDown |  | CONTROL_POWER |  |
| /power/cancelPowerOff |  | CONTROL_POWER |  |
| /power2/turnOnAoD |  | CONTROL_POWER |  |
| /power2/getPowerState |  | CONTROL_POWER |  |

### com.webos.service.update (com.webos.service.update.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getCurrentSWInformation |  | READ_UPDATE_INFO |  |
| /startUpdateByRemoteApp |  | UPDATE_FROM_REMOTE_APP |  |
| /getProgress |  | UPDATE_FROM_REMOTE_APP |  |
| /getStatus |  | UPDATE_FROM_REMOTE_APP |  |

### com.webos.service.utp.channel (com.webos.service.utp.channel.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /setChannelGroup |  | SECURE_CONTROL_CHANNEL_GROUP,CONTROL_CHANNEL_GROUP |  |
| /getChannelGroupList |  | SECURE_CONTROL_CHANNEL_GROUP,CONTROL_CHANNEL_GROUP |  |
| /getChannelModeList |  | SECURE_READ_TV_CHANNEL_LIST,READ_TV_CHANNEL_LIST |  |
| /scan/stopAutoScan |  | SECURE_SCAN_TV_CHANNELS,SCAN_TV_CHANNELS |  |
| /scan/startAutoScan |  | SECURE_SCAN_TV_CHANNELS,SCAN_TV_CHANNELS |  |
| /scan/skipAutoScan |  | SECURE_SCAN_TV_CHANNELS,SCAN_TV_CHANNELS |  |
| /scan/getAutoScanStatus |  | SECURE_SCAN_TV_CHANNELS,SCAN_TV_CHANNELS |  |
| /scan/getAutoScanProgress |  | SECURE_SCAN_TV_CHANNELS,SCAN_TV_CHANNELS |  |

### com.webos.service.utp.dvr (com.webos.service.utp.dvr.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /record/getRecordableDuration |  | READ_RECORDING_STATE |  |
| /record/getRecordingRemainDuration |  | READ_RECORDING_STATE |  |
| /record/startProgramRecord |  | SECURE_WRITE_RECORDING_SCHEDULE,WRITE_RECORDING_SCHEDULE |  |
| /record/isRecording |  | CONTROL_RECORDING |  |
| /record/stopRecord |  | CONTROL_RECORDING |  |

### com.webos.service.voiceconductor (com.webos.service.voiceconductor.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /recognizeIntentByText |  | CONTROL_VOICE |  |
| /setInputEvent |  | CONTROL_VOICE |  |

### com.webos.service.voiceinput (com.webos.service.voiceinput.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /respondEvent |  | CONTROL_VOICE |  |

### com.webos.service.wifi (com.webos.service.wifi.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /p2p/getstate |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |

### com.webos.service.wirelessavcontroller (com.webos.service.wirelessavcontroller.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getStatus |  | SECURE_READ_NETWORK_STATE,READ_NETWORK_STATE |  |

### com.webos.surfacemanager.input (com.webos.surfacemanager.input.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /getForegroundAppKeymask |  | READ_LGE_TV_INPUT_EVENTS |  |

### com.webos.surfacemanager (com.webos.surfacemanager.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /showInputPicker |  | SECURE_LAUNCH,LAUNCH | M |
| /getForegroundVideoWindow |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |
| /getRecentsAppList |  | SECURE_CONTROL_DISPLAY,CONTROL_DISPLAY |  |

### com.webos.universalsearch (com.webos.universalsearch.interface) 

| path | provider | requiredPermissions | flags |
|-|-|-|-|
| /search |  | SEARCH | RES:{"media":{"responseProperties":["localData.list.icon"]}} |
| /listPlugins |  | SEARCH |  |
