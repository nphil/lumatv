/* @@@LICENSE
 *
 * Copyright (c) 2021-2025 LG Electronics, Inc.
 *
 * Confidential computer software. Valid license from LG required for
 * possession, use or copying. Consistent with FAR 12.211 and 12.212,
 * Commercial Computer Software, Computer Software Documentation, and
 * Technical Data for Commercial Items are licensed to the U.S. Government
 * under vendor's standard commercial license.
 *
 * LICENSE@@@ */

import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSCoreCompositor 1.0
import WebOSCompositor 1.0
import WebOS.Global 1.0

import "../../services/base"

FocusScope {
    id: root
    z:-1

    signal focused()
    signal unfocused()

    property var windowPosition: (root && root.__surfaceItem && root.__surfaceItem.windowProperties.windowPosition) ? JSON.parse(root.__surfaceItem.windowProperties.windowPosition) : []
    property var containerInfo : getContainerInfo()
    property bool active: root.__surfaceItem ? root.__surfaceItem.activeFocus : false
    property Item __surfaceItem: null
    property var __displayWindowNotified: []
    property var controlMode: false
    property int cursorFps: (root && root.__surfaceItem && root.__surfaceItem.windowProperties.cursor_fps) ? Number(root.__surfaceItem.windowProperties.cursor_fps) : 0
    property bool cursorDisplay: (root && root.__surfaceItem && root.__surfaceItem.windowProperties.cursor_display) ? (root.__surfaceItem.windowProperties.cursor_display === "true" ? true : false) : false
    property bool cloudgameActivefromWinProp: (root && root.__surfaceItem && root.__surfaceItem.windowProperties.cloudgame_active) ? (root.__surfaceItem.windowProperties.cloudgame_active === "true" ? true : false) : false
    property bool cloudgameActivefromAppInfo: root && root.__surfaceItem && LS.applicationManager.appInfoList[root.__surfaceItem.appId] && LS.applicationManager.appInfoList[root.__surfaceItem.appId].cloudgame_active ? LS.applicationManager.appInfoList[root.__surfaceItem.appId].cloudgame_active : false
    property string __restoreCursorPositionSetting: (root && root.__surfaceItem && root.__surfaceItem.windowProperties.restore_cursor_position) ? root.__surfaceItem.windowProperties.restore_cursor_position : "false"
    property alias decoration: ssd
    property var __surfaceItemType: (root && root.__surfaceItem) ? root.__surfaceItem.type : null
    property var __surfaceItemScale: 1
    property bool __beforeAnimation: true
    property bool __showToolbarStatus: false
    property var multiviewControllerService: null
    property bool needItemFocus: false
    property bool isGeometryChanged: false
    property bool visibleState: (root.width > 0) && (root.height > 0)
    property string currentMode: ""
    property int currentOrder: -1  // default value?
    property var coverState: SurfaceItem.CoverStateNormal
    property bool fitOnParent: (root.width === root.parent.width) && (root.height === root.parent.height)
    property bool hasFocusedSurfaceItem: root.active || hasActiveGroupedItem()
    property bool standbyVisible: fullscreenStandby.visible
    property bool emptySurfaceItem : (isMultiViewMode() == true) && emptyCurrentItem()
    property bool forceAnimation: false
    property bool directDestroy: false
    property bool connected: false

    property string controllerModeGuideStr: qsTr("Please press and hold the back button on the remote control to return to the previous screen.") + Settings.l10n.tr
    property var languageStyle: StarfishUtils.languageStyle
    property bool surfaceItemContainsMouse: root.__surfaceItem && root.__surfaceItem.containsMouse && decoration.inputMode && ssd.__config.buttons
    property bool dragCover: ssd.dragging
    property string animationState: "ready"

    property bool grouped: false
    property var __groupedItems: []
    property var __groupLowerChildren: []
    property var __groupUpperChildren: []

    property QtObject __layoutInfo: QtObject {
        property string appId: ""
        property string mode: ""
        property string usageType: ""
        property int order: -1
        property int x: 0
        property int y: 0
        property int z: 0
        property int width: 0
        property int height: 0
        property bool visibleState: false
        property string multiviewOrientation: "landscape"
        property int rotation: 0
        property bool isPipSub: false
        property var decorationConfig: ({})
        function reset()
        {
            console.info("[FULLSCREEN:Container][" + root.objectName + "] reset layoutInfo")
            appId = ""; mode = ""; usageType = ""; order = -1;
            x = 0; y = 0; z = 0; width = 0; height = 0;
            visibleState = false; multiviewOrientation = "landscape"; rotation = 0;
            isPipSub = false; decorationConfig = ({});
        }
    }

    property QtObject __pendingLayoutInfo: QtObject {
        property bool isPending: false
        property bool needReset: false
        property bool changedExceptGeometry: true
        property int x: 0
        property int y: 0
        property int z: 0
        property int width: 0
        property int height: 0
        property string multiviewOrientation: "landscape"
        property bool isPipSub: false
        property var decorationConfig: ({})
        function reset() {
            isPending = false; needReset = false; changedExceptGeometry = true;
            x = 0; y = 0; z = 0; width = 0; height = 0;
            multiviewOrientation = "landscape";
            isPipSub = false; decorationConfig = ({});
        }
    }

    signal groupUpdated(var groupLowerChildren, var groupUpperChildren, var change)
    signal layoutInfoUpdated()
    signal appStateChanged()
    signal surfaceItemChanged()
    signal groupedStatusUpdated()
    signal focusedSurfaceItem(int order, bool focus)
    signal focusedDecoration(int order, string direction)
    signal requestShowToolbar(int order, bool delayed, string mode)
    signal applyLayoutDone(var obj)

    Component.onCompleted: {
        console.info("[FULLSCREEN:Container][MVN] " + root.objectName  + " completed")
    }

    Component.onDestruction: {
        console.info("[FULLSCREEN:Container][MVN] " + root.objectName  + " destruction")
    }

    property Item __livedmostItem: null

    property StarfishForegroundVideoWindowMgr foregroundVideoWindowMgr: StarfishForegroundVideoWindowMgr {
        onForegroundVideoWindowChanged: {
            if (root.__layoutInfo.isPipSub === true) {
                // bgBlack unvisible only when layout is not pip or sub
                return
            }
            if (bgBlack.visible == false) {
                return
            }

            var videoList = getForegroundVideoWindow()
            Utils.pmLog.info("SVU_LSM_BG_BLACK", {}, "[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] check bgBlack. video list : " + JSON.stringify(videoList) + ", grouped : " + root.grouped + ", groupedItems : " + root.__groupedItems);

            for (var i = 0; i < videoList.length; i++) {
                if (videoList[i] !== null && videoList[i] !== undefined) {
                    if (root.grouped && 0 < root.__groupedItems.length) {
                        for (var j = 0; j < root.__groupedItems.length; j++) {
                            if (videoList[i].appId === root.__groupedItems[j].appId) {
                                Utils.pmLog.info("SVU_LSM_BG_BLACK", {}, "[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] set bgBlack false by video is inserted to group." + JSON.stringify(videoList[i]));
                                __livedmostItem = root.__groupedItems[j]
                                bgBlack.visible = false
                                break
                            }
                        }
                    }
                    else if (root.__layoutInfo.appId === videoList[i].appId) {
                        Utils.pmLog.info("SVU_LSM_BG_BLACK", {}, "[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] set bgBlack false by video is inserted. " + JSON.stringify(videoList[i]) + root.__layoutInfo.appId);
                        bgBlack.visible = false
                        break
                    }
                }
            }
        }
    }

    Rectangle {
        id: bgBlack
        anchors.fill: parent
        color: "black"
        visible: false
        z: -999
        onVisibleChanged: {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] bgBlack changed : " + visible);
        }
    }

    StarfishFullscreenStandby {
        id : fullscreenStandby
        anchors.fill: parent
        border.color : (root.__layoutInfo.mode === "pip" && !root.__layoutInfo.isPipSub) ? "transparent" : Settings.local.standbyIcon.borderColor
        visible : isMultiViewMode() && emptyCurrentItem() && ssd.__config.standbyIcon

        onVisibleChanged : console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "]  isMultiViewMode = " + isMultiViewMode() + " , emptyCurrentItem = " + emptyCurrentItem() + " Standby icon visible is changed = " + visible)
    }

    Item {
        id: surfaceItemContainer
        anchors.fill: parent
        clip: true
    }

    // Luma Home: a compositor-drawn layer that owns Home's chrome (wallpaper, glass,
    // status bar, widgets, dock) while the stock Flutter app underneath stays resident as an
    // invisible placeholder. The file lives two levels up from this view
    // (WebOSCompositor/{{qmlModule}}/LumaHome.qml). Remote keys reach it through the
    // configd-registered key filter (payload/keyfilter/lumatv.js), never through QtQuick
    // focus: the compositor's KeyFilter sends keys straight to the focused Wayland window.
    //
    // Built the first time Home is this container's surface and kept from then on, shown only
    // while Home is in front. Rebuilding it on every return to Home froze the panel for one
    // frame of 187-320 ms and then 3-6 more of 43-116 ms while widgets reloaded. While hidden
    // it is idle: hostActive is false and its timers and fetches stop.
    property bool lumaHomeCurrent: !!root.__surfaceItem && root.__surfaceItem.appId === "com.webos.app.home"
    property bool lumaHomeBuilt: false
    onLumaHomeCurrentChanged: if (lumaHomeCurrent) lumaHomeBuilt = true
    Loader {
        id: lumaHome
        anchors.fill: parent
        z: 2
        active: root.lumaHomeCurrent || root.lumaHomeBuilt
        visible: root.lumaHomeCurrent && status === Loader.Ready
        source: "../../{{qmlModule}}/LumaHome.qml"
        onStatusChanged: {
            if (status === Loader.Error)
                console.warn("[FULLSCREEN:Container][" + root.objectName + "] LumaHome failed to load from " + source)
        }
    }
    Binding {
        target: lumaHome.item
        property: "hostActive"
        value: root.lumaHomeCurrent && root.visibleState && lumaHome.status === Loader.Ready
        when: lumaHome.status === Loader.Ready
    }
    // The stock Home surface is fully covered while LumaHome draws; at opacity 0 the scene
    // graph skips its full-screen blit (a 1080p texture read and write per frame). Wayland
    // focus and key delivery do not depend on the item's opacity, and the Binding restores
    // the value the moment the layer goes away.
    Binding {
        target: root.__surfaceItem
        property: "opacity"
        value: 0
        when: lumaHome.visible && root.__surfaceItem
    }

    StarfishFullscreenStandby {
        id : fullscreenCover
        anchors.fill: parent
        visible: false

        onVisibleChanged : {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Cover UI visible is changed = " + fullscreenCover.visible )
        }
    }

    StarfishServerDecoration {
        id: ssd
        enabled: false
        z: root.z + 1
        isMultiViewMode: root.isMultiViewMode()
        isMultiViewLandscapeMode: root.isMultiViewLandscapeMode()
        fullscreenCoverVisible: fullscreenCover.visible
        containerVisible: root.visibleState
        controlMode: root.controlMode
        itemContainsMouse: root.surfaceItemContainsMouse
        onRequestGoNextDecoration: {
            if (!root.__layoutInfo.isPipSub) {
                root.focusedDecoration(order, direction)
            }
        }
        onRequestShowToolbar: root.requestShowToolbar(order, delayed, root.mode())
        onRequestHideDecorations: root.setShowToolbarStatus(false) // by backkey
        expireTimer.onTriggered: root.setShowToolbarStatus(false) //by timer
    }

    Rectangle {
        id: controllerModeGuide
        visible: root.controlMode && root.fitOnParent && controlModeGuideTimer.running && !emptyCurrentItem() && ssd.__config.controlModeGuideUI
        anchors.bottom: root.bottom
        width: root.width
        height: Settings.local.multiviewControlMode.bgHeight
        color: "transparent"
        onVisibleChanged: {
            if (visible) {
                views.fullscreen.requestTts(root.controllerModeGuideStr, 1, true)
            }
        }

        Rectangle {
            id: bgImage
            anchors.fill: parent
            color: Settings.local.multiviewControlMode.bgColor
            opacity: Settings.local.multiviewControlMode.opacity
        }

        Text {
            id: guideText
            anchors.verticalCenter: parent.verticalCenter
            anchors.horizontalCenter: parent.horizontalCenter
            font.family: root.languageStyle.normal400.fontFamily[0]
            font.pixelSize: Settings.local.multiviewControlMode.fontSize
            color: Settings.local.multiviewControlMode.fontColor
            text: root.controllerModeGuideStr
        }
    }

    function __debugState() {
        return {
            "surfaceItem": (root.__surfaceItem ? root.__surfaceItem.appId : "empty"),
            "layoutInfo": {
                "appId": root.__layoutInfo.appId,
                "mode": root.__layoutInfo.mode,
                "order": root.__layoutInfo.order,
                "x": root.__layoutInfo.x,
                "y": root.__layoutInfo.y,
                "z": root.__layoutInfo.z,
                "width": root.__layoutInfo.width,
                "height": root.__layoutInfo.height,
                "multiviewOrientation": root.__layoutInfo.multiviewOrientation,
                "controlMode": root.controlMode
            }
        }
    }

    function __debugSSD() {
        return ssd
    }

    function __debugAnimator() {
        return containerAnimator
    }

    function nextAppId() {
        return root.__layoutInfo.appId
    }

    function resetNextLayoutInfo() {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] reset next layout info")
        if (fullscreenCover.visible) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Delay to reset layout info")
            root.__pendingLayoutInfo.isPending = true
            root.__pendingLayoutInfo.needReset = true
        } else {
            root.__layoutInfo.reset()
            layoutInfoUpdated()
        }
    }

    function setNextLayout(mode, usageType, requestedLayoutInfo, isPipSub = false, changedExceptGeometry) {

        console.info("[FULLSCREEN][" + root.objectName + "][" + requestedLayoutInfo.order + "] setNextLayout", requestedLayoutInfo.appId, mode, usageType,
                    requestedLayoutInfo.x, requestedLayoutInfo.y, requestedLayoutInfo.width, requestedLayoutInfo.height,
                    requestedLayoutInfo.orientation, isPipSub, changedExceptGeometry)

        root.setShowToolbarStatus(false)

        if (bgBlack.visible === false) {
            if (root.__layoutInfo.appId !== "" && requestedLayoutInfo.appId !== root.__layoutInfo.appId) {
                Utils.pmLog.info("SVU_LSM_BG_BLACK", {}, "[FULLSCREEN][" + root.objectName + "][" + requestedLayoutInfo.order + "] set bgBlack true by layout appid is changed. " + root.__layoutInfo.appId + " -> " + requestedLayoutInfo.appId);
                bgBlack.visible = true
            }
            if (isPipSub === true) {
                Utils.pmLog.info("SVU_LSM_BG_BLACK", {}, "[FULLSCREEN][" + root.objectName + "][" + requestedLayoutInfo.order + "] set bgBlack true by layout is pip & sub");
                bgBlack.visible = true
            }
        }

        if (changedExceptGeometry === undefined) {
            if (root.__layoutInfo.appId === requestedLayoutInfo.appId && root.__layoutInfo.mode === mode && root.__layoutInfo.order === requestedLayoutInfo.order) {
                changedExceptGeometry = false
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + requestedLayoutInfo.order + "] layout info except geometry is not changed")
            } else {
                changedExceptGeometry = true
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + requestedLayoutInfo.order + "] layout info except geometry is changed")
            }
        }

        root.__layoutInfo.appId = requestedLayoutInfo.appId
        root.__layoutInfo.mode = mode
        root.__layoutInfo.usageType = usageType
        root.__layoutInfo.order = requestedLayoutInfo.order
        root.__layoutInfo.isPipSub = isPipSub

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] fullscreenCover.visible " + fullscreenCover.visible)

        if (fullscreenCover.visible) {
            __storePendingLayoutInfo(requestedLayoutInfo.x, requestedLayoutInfo.y, requestedLayoutInfo.width, requestedLayoutInfo.height, requestedLayoutInfo.orientation, isPipSub, requestedLayoutInfo.decorationConfig, changedExceptGeometry)
            if (root.coverState === SurfaceItem.CoverStateNormal) {
                console.info("[FULLSCREEN::Container][" + root.objectName + "][" + root.__layoutInfo.order + "] set cover state to changing because store pending layout")
                __updateCoverState(SurfaceItem.CoverStateChanging)
            }
            layoutInfoUpdated()
            return
        }

        root.isGeometryChanged = false
        if (changedExceptGeometry === false) {
            if ((root.__layoutInfo.x !== requestedLayoutInfo.x) || (root.__layoutInfo.y !== requestedLayoutInfo.y) ||
                (root.__layoutInfo.width !== requestedLayoutInfo.width) || (root.__layoutInfo.height !== requestedLayoutInfo.height))
                root.isGeometryChanged = true
        }

        root.__layoutInfo.x = requestedLayoutInfo.x ? requestedLayoutInfo.x : 0
        root.__layoutInfo.y = requestedLayoutInfo.y ? requestedLayoutInfo.y : 0
        root.__layoutInfo.z = (isPipSub) ? 2 : 0
        root.__layoutInfo.width = requestedLayoutInfo.width ? requestedLayoutInfo.width : 0
        root.__layoutInfo.height = requestedLayoutInfo.height ? requestedLayoutInfo.height : 0
        root.__layoutInfo.visibleState = (root.__layoutInfo.width > 0) && (root.__layoutInfo.height > 0)
        root.__layoutInfo.multiviewOrientation = (requestedLayoutInfo.orientation !== undefined) ? requestedLayoutInfo.orientation : "landscape"
        root.__layoutInfo.decorationConfig = (requestedLayoutInfo.decorationConfig !== undefined) ? requestedLayoutInfo.decorationConfig : ({})
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] container layoutInfo's multiviewOrientation is  " + root.__layoutInfo.multiviewOrientation)


        __setAniDuration()
        layoutInfoUpdated()

        ssd.updateDecorationLayout(root.__layoutInfo)
        if(changedExceptGeometry){
            ssd.startAni()
        }
    }

    function setControlMode(isControlMode) {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] control mode is updated to " + isControlMode + " for " + root.objectName)
        if ((root.__surfaceItem === null && isControlMode) || root.controlMode === isControlMode)
           return

        root.controlMode = isControlMode
        if (root.__layoutInfo.mode === "sxs" ||  (root.__layoutInfo.mode === "pip" && !root.__layoutInfo.isPipSub)) {
            root.z = root.controlMode ? 2 : 0
        }
    }

    function applyNewLayout() {

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] applyNewLayout")

        if (fullscreenCover.visible) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Cover status. Wait to apply layout")
            return
        }

        if (__checkAnimationExceptionCase() && !root.forceAnimation) {
            __applyNewLayoutWithoutAnimation()
            root.applyLayoutDone(root)
            return
        }

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] previous layout info - ", root.x, root.y, root.z, root.width, root.height)
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] changed layout info - ", root.__layoutInfo.x, root.__layoutInfo.y, root.__layoutInfo.z, root.__layoutInfo.width, root.__layoutInfo.height)

        // Start animation at the center position of container
        if (root.visibleState === false && root.__layoutInfo.visibleState === true && isMultiViewMode())
        {
            root.x = root.__layoutInfo.x + root.__layoutInfo.width * 0.5
            root.y = root.__layoutInfo.y + root.__layoutInfo.height * 0.5
        }

        // Finish animation at the center position of container
        if (root.visibleState === true && root.__layoutInfo.visibleState === false && (root.currentMode === "pip" || root.currentMode === "sxs"))
        {
            root.__layoutInfo.x = root.x + root.width * 0.5
            root.__layoutInfo.y = root.y + root.height * 0.5
        }

        root.currentMode = root.__layoutInfo.mode
        root.currentOrder = root.__layoutInfo.order

        /*
            --- cover state ---
            Normal : show video. can call setDisplayWindow anytime
            Hidden : hide video. call setDisplayWindow 0x0
            Changing : keep current video. Do not call setDisplayWindow
            -------------------

            1. show cover UI (fullscreenCover)
            2. set cover state - changing
                -> keep current video to prevent video transient
            3. delay timer 150ms - to wait show cover
            4. (timer triggered) set cover state - hidden
                -> hide video through video window
            5. delay timer 150ms - to wait hide video
            6. (timer triggered) container animation start
            7. after animation, delay timer - to wait graphic rendering
            8. (timer triggered) reset cover state
                8-1. set cover state - changing, when pending layout is exist
                    -> keep current video(0x0)
                8-2. set cover state - normal, when other case
                    -> show video
            9. delay timer - to wait show video
                -> set cover state - chaning, if pending layout is stored while wait delay
            10. (timer triggered) hide cover UI
        */

        fullscreenCover.visible = true

        root.animationState = "running"
        __updateCoverState(SurfaceItem.CoverStateChanging)
        console.info("[FULLSCREEN:Container][" + root.__layoutInfo.order + "] start delay timer to show cover ")
        __beforeAnimation = true
        delayAnimationTimer.delayInterval = 150
        delayAnimationTimer.restart()
    }

    function order() {
        return root.__layoutInfo.order
    }

    function mode() {
        return root.__layoutInfo.mode
    }

    function usageType() {
        return root.__layoutInfo.usageType
    }

    function currentAppId() {
        return (root.__surfaceItem && root.__surfaceItem.appId) ? root.__surfaceItem.appId : ""
    }

    function isMultiViewMode() {
        return (root.__layoutInfo.mode === "pip" || root.__layoutInfo.mode === "sxs")
    }

    function isMultiViewLandscapeMode() {
        return (root.__layoutInfo.multiviewOrientation === "landscape")
    }

    function isPipSubContainer() {
        return (root.__layoutInfo.isPipSub === true)
    }

    function getSurfaceItem() {
        return root.__surfaceItem;
    }

    function setSurfaceItem(item) {

        if (root.__surfaceItem === item) return;

        __displayWindowNotified = [];
        __resetGroupedItems()

        if (root.__surfaceItem) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceItem is changed from " + root.__surfaceItem + " to " + item )
            root.__surfaceItem.parent = null
            root.__surfaceItem.fullscreen = false
            if (item === null && !root.connected) {
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceItem was removed from model and container will be destroyed.")
                return
            }
            root.__surfaceItem = null
        } else {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] previous surfaceItem is empty");
        }

        root.__surfaceItem = item

        if (root.__surfaceItem) {
            __updateLayoutInfoBasedOnRotation()

            if (isMultiViewMode() && (usageType() === "appView") && root.__layoutInfo.isPipSub)
                root.__surfaceItem.enabled = false
        }

        if (item) {
            root.__surfaceItem.parent = surfaceItemContainer   // To clip surface item
            root.__surfaceItem.useTextureAlpha = true
        }

        __checkAndUpdateVideoDisplay()

        if (root.__surfaceItem && root.__surfaceItem.surfaceGroup) {
            __setGroupedOwnerItems(item)
        }
        __changeGroupedStatus()
        layoutInfoUpdated()
        surfaceItemChanged()

        __updateCoverState(root.coverState)
    }

    function getShowToolbarStatus() {
        return root.__showToolbarStatus
    }

    function setShowToolbarStatus(status) {
        root.__showToolbarStatus = status
    }

    function resetContainer() {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] reset container");
        setSurfaceItem(null)
        resetNextLayoutInfo()
        __setAniDuration()
        setControlMode(false)
        ssd.resetDecorationLayout()
    }

    function resetDecorationFocus() {
        ssd.setVisible(false)
        ssd.setFocus(false)
    }

    function emptyNextAppId() {
        return root.__layoutInfo.appId === ""
    }

    function emptyCurrentItem() {
        if (root.__surfaceItem)
            return false

        return true
    }

    function getContainerInfo() {
        var containerInfo = {};
        containerInfo["appId"] = root.__surfaceItem ? root.__surfaceItem.appId : null
        containerInfo["nextAppId"] = root.__layoutInfo.appId
        containerInfo["mode"] = root.__layoutInfo.mode
        containerInfo["order"] = root.__layoutInfo.order
        containerInfo["active"] = root.active
        containerInfo["visible"] = root.visible
        containerInfo["surfaceItem"] = root.__surfaceItem
        containerInfo["grouped"] = root.grouped
        containerInfo["groupedItems"] = root.__groupedItems

        return containerInfo;
    }

    function getGeometryInfo() {
        var geometryInfo = {}
        geometryInfo["x"] = root.__layoutInfo.x
        geometryInfo["y"] = root.__layoutInfo.y
        geometryInfo["width"] = root.__layoutInfo.width
        geometryInfo["height"] = root.__layoutInfo.height

        return geometryInfo
    }

    function getToolbarPosition() {
        return ssd.toolbarPosition
    }

    function setFocus(bFocus) {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] setFocus = " + bFocus)

        if (bFocus) {
            root.focus = true
            root.needItemFocus = true
            setItemFocus(true)
        } else {
            root.focus = false
            root.needItemFocus = false
            setItemFocus(false)
        }
    }

    function hasActiveGroupedItem() {
        var retValue = false
        if (groupModelLoader.item)
            retValue = groupModelLoader.item.hasActiveFocus();
        return retValue;
    }

    function __setAniDuration() {

        if (root.width === root.__layoutInfo.width && root.height === root.__layoutInfo.height)
        {
            containerAnimator.aniDuration = 400
        }
        else if ((Math.abs(root.x-root.__layoutInfo.x) > root.parent.width*0.3)
                    || (Math.abs(root.y-root.__layoutInfo.y) > root.parent.height*0.3)
                        || (Math.abs(root.width-root.__layoutInfo.width) > root.parent.width*0.3)
                            || (Math.abs(root.height-root.__layoutInfo.height) > root.parent.height*0.3))
        {
           containerAnimator.aniDuration = 600
        }
        else
        {
            containerAnimator.aniDuration = 300
        }
    }

    function __setPendingLayoutInfo() {

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "]")

        var needApplyLayout = false
        if (root.__pendingLayoutInfo.isPending) {
            if (root.__pendingLayoutInfo.needReset) {
                resetNextLayoutInfo()
                ssd.resetDecorationLayout()
            } else {
                var layoutInfo = {
                    "appId": root.__layoutInfo.appId,
                    "order": root.__layoutInfo.order,
                    "x": root.__pendingLayoutInfo.x,
                    "y": root.__pendingLayoutInfo.y,
                    "width": root.__pendingLayoutInfo.width,
                    "height": root.__pendingLayoutInfo.height,
                    "orientation": root.__pendingLayoutInfo.multiviewOrientation,
                    "decorationConfig": root.__pendingLayoutInfo.decorationConfig
                }
                setNextLayout(root.__layoutInfo.mode, root.__layoutInfo.usageType, layoutInfo, root.__pendingLayoutInfo.isPipSub, root.__pendingLayoutInfo.changedExceptGeometry)
            }
            needApplyLayout = true
        } else {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] There is no pending info")
        }
        root.__pendingLayoutInfo.reset()

        if (needApplyLayout)
            applyNewLayout()
    }

    function __storePendingLayoutInfo(x, y, w, h, multiviewOrientation, isPipSub, decorationConfig, changedExceptGeometry) {

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] store pending info")

        root.__pendingLayoutInfo.isPending = true
        root.__pendingLayoutInfo.needReset = false
        root.__pendingLayoutInfo.x = x
        root.__pendingLayoutInfo.y = y
        root.__pendingLayoutInfo.width = w
        root.__pendingLayoutInfo.height = h
        root.__pendingLayoutInfo.multiviewOrientation = (multiviewOrientation !== undefined) ? multiviewOrientation : "landscape"
        root.__pendingLayoutInfo.isPipSub = isPipSub
        root.__pendingLayoutInfo.decorationConfig = (decorationConfig !== undefined) ? decorationConfig : ({})
        root.__pendingLayoutInfo.changedExceptGeometry = changedExceptGeometry
    }

    function __applyNewLayoutWithoutAnimation() {

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] apply new layout without animation")

        root.rotation = root.__layoutInfo.rotation
        root.x = root.__layoutInfo.x
        root.y = root.__layoutInfo.y
        root.z = root.__layoutInfo.z
        root.height = root.__layoutInfo.height
        root.width = root.__layoutInfo.width

        root.currentMode = root.__layoutInfo.mode
        root.currentOrder = root.__layoutInfo.order

        __fitSurfaceItemToParent()
    }

    function __checkAnimationExceptionCase() {

        if (!ssd.__config.animation)
            return true

        if (root.currentMode === "" && root.__layoutInfo.mode === "")
            return true

        if (root.x ===  root.__layoutInfo.x
            && root.y === root.__layoutInfo.y
                && root.width === root.__layoutInfo.width
                    && root.height === root.__layoutInfo.height)
                    {
                        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] geometry is not changed")
                        return true
                    }

        if (root.rotation !== root.__layoutInfo.rotation ||
            (root.__layoutInfo.rotation == 90 || root.__layoutInfo.rotation == 270)) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] rotation is changed. Skip the animation")
            return true
        }

        /*Note : No animation case
            "" <-> pip + main <-> normal
        */
        if (root.currentMode !== root.__layoutInfo.mode && root.visibleState !== root.__layoutInfo.visibleState)
        {
            if ((root.width === root.parent.width && root.height === root.parent.height && root.__layoutInfo.visibleState === false)
                || (root.__layoutInfo.width === root.parent.width && root.__layoutInfo.height === root.parent.height && root.visibleState === false))
                {
                    console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] fullscreen app switch case")
                    return true
                }
        }

        if (root.currentMode === "sxs" && isMultiViewMode() === false && emptyNextAppId()) {
            return true
        }

        if (nextAppId().indexOf("empty") !== -1) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Cover app to be changed or closed.")
            return true
        }

        return false
    }

    function __updateCoverState(state) {

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] update cover state = " + state)
        if (root.__surfaceItem)
            root.__surfaceItem.coverState = state
        root.coverState = state

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] update cover state of groupModelLoader = " + groupModelLoader.item)
        if (groupModelLoader.item)
            groupModelLoader.item.updateCoverState(state)
    }

    function __setGroupedOwnerItems(item) {
        if (root.__surfaceItem && root.__surfaceItem.surfaceGroup && root.__surfaceItem == item) {
            root.__groupedItems = [item]
            root.__loadGroupModel()
        }
    }

    function __resetGroupedItems() {
        root.__groupedItems = []
        root.__groupLowerChildren = [];
        root.__groupUpperChildren = [];

        groupModelLoader.setSource("");
    }

    function __changeGroupedStatus() {
        root.grouped = (root.__surfaceItem && root.__surfaceItem.surfaceGroup && root.__groupedItems.length > 0) ? true : false
        root.groupedStatusUpdated()
    }

    function __updateLayoutInfoBasedOnRotation() {
        if (root.__surfaceItem == null) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceItem is null")
            return
        }

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "][updateLayoutInfoBasedOnRotation-START] appId:" + root.__surfaceItem.appId + " layoutInfo (rotation:" + root.__layoutInfo.rotation +
                     " width:" + root.__layoutInfo.width + " height:" + root.__layoutInfo.height + " x:" + root.__layoutInfo.x + " y:" + root.__layoutInfo.y + ")")

        switch (root.__surfaceItem.orientation) {
            case 1:  root.__layoutInfo.rotation = 270;  break;
            case 2:  root.__layoutInfo.rotation = 0;   break;
            case 4:  root.__layoutInfo.rotation = 90; break;
            case 8:  root.__layoutInfo.rotation = 180; break;
            default: root.__layoutInfo.rotation = 0;   break;
        }

        if (root.__layoutInfo.rotation % 180) {
            // This case is for portrait.
            root.__layoutInfo.width = root.parent.height // root.parent is fullscreenView
            root.__layoutInfo.height = root.parent.width

            root.__layoutInfo.x = Utils.center(root.parent.width, root.__layoutInfo.width)
            root.__layoutInfo.y = Utils.center(root.parent.height, root.__layoutInfo.height)
        } else {
            // else is for landscape. Layout for landscape shoud consider the multiview settings that is already set in setLayout() func.
            if (!isMultiViewMode()) {
                root.__layoutInfo.width = root.parent.width // root.parent is fullscreenView
                root.__layoutInfo.height = root.parent.height

                root.__layoutInfo.x = Utils.center(root.parent.width, root.__layoutInfo.width)
                root.__layoutInfo.y = Utils.center(root.parent.height, root.__layoutInfo.height)
            }
        }

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "][updateLayoutInfoBasedOnRotation-END] appId:" + root.__surfaceItem.appId + " layoutInfo (rotation:" + root.__layoutInfo.rotation +
                     " width:" + root.__layoutInfo.width + " height:" + root.__layoutInfo.height + " x:" + root.__layoutInfo.x + " y:" + root.__layoutInfo.y + ")")
    }

    onGroupUpdated: {
        var isChanged = false;
        if ((change === "both" || change === "lower") && __isChildItemsUpdated(groupLowerChildren, root.__groupLowerChildren)) {
            root.__groupLowerChildren = groupLowerChildren;
            isChanged = true;
        }

        if ((change === "both" || change === "upper") && __isChildItemsUpdated(groupUpperChildren, root.__groupUpperChildren)) {
            root.__groupUpperChildren = groupUpperChildren;
            isChanged = true;
        }

        if (isChanged) {
            if (root.grouped && root.__surfaceItem) {
                root.__groupedItems = root.__groupLowerChildren.concat(Array(root.__surfaceItem), root.__groupUpperChildren);
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceItem's group is updated, surfaceItem = " + root.__surfaceItem + ", change = " + change + ", groupedItem = " + root.__groupedItems);
            }
        }
    }

    onWindowPositionChanged: {
        __checkAndUpdateVideoDisplay();
    }

    onActiveChanged: {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] appId:" + (root.__surfaceItem ? root.__surfaceItem.appId : "") + ", activeFocus:" + root.activeFocus +
                     ", surfaceItem.activeFocus:" + root.active + ", hasActiveGroupedItem:" + root.hasActiveGroupedItem());

        layoutInfoUpdated();
    }

    onActiveFocusChanged: {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] appId:" + (root.__surfaceItem ? root.__surfaceItem.appId : "") + ", activeFocus:" + root.activeFocus +
                     ", surfaceItem.activeFocus:" + root.active + ", hasActiveGroupedItem:" + root.hasActiveGroupedItem());
    }

    onHasFocusedSurfaceItemChanged: {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] appId:" + (root.__surfaceItem ? root.__surfaceItem.appId : "") + ", activeFocus:" + root.activeFocus +
                     ", surfaceItem.activeFocus:" + root.active + ", hasActiveGroupedItem:" + root.hasActiveGroupedItem());
        focusedSurfaceItem(root.__layoutInfo.order, hasFocusedSurfaceItem); // sends a signal whenever a surfaceitem of focus changes
    }

    onDragCoverChanged: {
        // Prevent mismatch of video position when dragging sub app in pip mode
        if (root.dragCover)
            __updateCoverState(SurfaceItem.CoverStateHidden)
        else
            __updateCoverState(SurfaceItem.CoverStateNormal)
    }

    onMultiviewControllerServiceChanged: {
        ssd.ssdService = root.multiviewControllerService
    }

    function __checkAndUpdateVideoDisplay(){
        var diff = false;
        var params;

        if (windowPosition.length === 0 || __surfaceItemType !== "_WEBOS_WINDOW_TYPE_CARD") {
            __displayWindowNotified = [];
            return;
        }

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Update window position");
        if (windowPosition.length != __displayWindowNotified.length) {
            diff = true;
        } else {
            for (var i = 0; i < windowPosition.length; i++) {
                if (windowPosition[i].id != __displayWindowNotified[i].context
                    || windowPosition[i].punchX != __displayWindowNotified[i].displayOutput.x
                        || windowPosition[i].punchY != __displayWindowNotified[i].displayOutput.y
                            || windowPosition[i].punchW != __displayWindowNotified[i].displayOutput.width
                                || windowPosition[i].punchH != __displayWindowNotified[i].displayOutput.height)
                    {
                        diff = true;
                        break;
                    } else {
                        console.log("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] windowPosition info is same as notified one");
                    }
            }
        }

        if (diff)
            __setVideoDisplayWindow();
    }

    function __setVideoDisplayWindow() {
        var region, params;
        __displayWindowNotified = [];
        for (var i = 0; i < windowPosition.length; i++) {
            region = windowPosition[i];
            if (region.id === "") {
                console.log("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Pipeline id is empty. Do not call setDisplayWindow");
                continue;
            }
            params = {
                "displayOutput": {
                    "x": region.punchX,
                    "y": region.punchY,
                    "width": region.punchW,
                    "height": region.punchH
                },
                "context": region.id,
                "fullScreen": true
            };
            __displayWindowNotified.push(params);
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Calling setDisplayWindow:", JSON.stringify(params));
            LS.adhoc.call("luna://com.webos.service.videooutput", "/display/setDisplayWindow", JSON.stringify(params));
        }
    }

    function __isChildItemsUpdated(childItems, targetChildItems) {
        //To block redundant getForegroundAppInfo publishing, check real change of child items is occurred or not.
        var isChildItemsSame = (targetChildItems.length == childItems.length) && targetChildItems.every(function(element, index) {
            return element === childItems[index];
        });
        return (isChildItemsSame) ? false : true;
    }

    function __loadGroupModel() {
        if (!root.__surfaceItem) {
            console.warn("Surface item is empty. Group model can't be loaded.");
            return;
        }

        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] __loadGroupModel. groupOwner = " + root.__surfaceItem + ", groupOwnerParent = " + root);
        groupModelLoader.setSource("../../models/StarfishGroupModel.qml", { objectName: "fullscreenGroupModel", groupOwner: root.__surfaceItem, groupOwnerParent: root });
    }

    function __fitSurfaceItemToParent() {

        if (containerAnimator.running) {
            console.log("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] container animator is running")
            return
        }

        if (__surfaceItem) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] [FitSurfaceItem-START] x = " + root.x + " , y= " + root.y + ", width = " + root.width + " , height = " + root.height);
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surface item appId = " + __surfaceItem.appId+ ", width = " + __surfaceItem.width + ", height = " + __surfaceItem.height + " , previous scale factor = " + __surfaceItemScale)

            if (root.width === 0 || root.heigth === 0 || __surfaceItem.width <= 0 || __surfaceItem.height <= 0) {
                __surfaceItem.x = 0
                __surfaceItem.y = 0
                __surfaceItem.scale = 0
            } else {
                __surfaceItem.x = root.width / 2 -  __surfaceItem.width / 2;
                __surfaceItem.y = root.height / 2 - __surfaceItem.height / 2;

                if (root.__layoutInfo.multiviewOrientation === "landscape") {
                    __surfaceItem.scale = Math.min(root.width / __surfaceItem.width, root.height / __surfaceItem.height)
                    __surfaceItem.activeRegion = Qt.rect(0,0,0,0)
                } else {
                    __surfaceItem.scale = root.height / __surfaceItem.height
                    var activeWidth = root.width * __surfaceItem.height / root.height
                    var activeX = (__surfaceItem.width - activeWidth) * 0.5
                    var activeRegion =  Qt.rect(activeX, 0, activeWidth, __surfaceItem.height)
                    console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] portrait active region = " + activeRegion)
                    __surfaceItem.activeRegion = activeRegion
                }
            }

            if (root.__layoutInfo.isPipSub) {
                var surfaceToContainer = __surfaceItem.mapToItem(root, 0, 0, __surfaceItem.width, __surfaceItem.height)

                // To fix error when calculate activeRegion in LSM, add margin 1 pixel to all directions.
                var tempMargin = (root.__layoutInfo.multiviewOrientation === "landscape" ? 0 : 1)

                bgBlack.anchors.leftMargin = Math.max(surfaceToContainer.x, 0) + tempMargin
                bgBlack.anchors.topMargin = Math.max(surfaceToContainer.y, 0) + tempMargin
                bgBlack.anchors.rightMargin = Math.max(root.width - surfaceToContainer.right, 0) + tempMargin
                bgBlack.anchors.bottomMargin = Math.max(root.height - surfaceToContainer.bottom, 0) + tempMargin
            } else {
                bgBlack.anchors.leftMargin = 0
                bgBlack.anchors.topMargin = 0
                bgBlack.anchors.rightMargin = 0
                bgBlack.anchors.bottomMargin = 0
            }
            Utils.pmLog.info("SVU_LSM_BG_BLACK", {}, "[FULLSCREEN] global positions. surfaceitem : " + __surfaceItem.mapToItem(null, 0, 0, __surfaceItem.width, __surfaceItem.height) + ", bgBlack : " + bgBlack.mapToItem(null, 0, 0, bgBlack.width, bgBlack.height));

            if (__surfaceItemScale !== __surfaceItem.scale) {
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surface item scale is changed. scale = " + __surfaceItem.scale)
                __surfaceItem.scaleChanged()
                __surfaceItemScale = __surfaceItem.scale
            }

            if (__surfaceItem.pipSub !== root.__layoutInfo.isPipSub) {
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surface item pipSub is changed. pipSub = " + root.__layoutInfo.isPipSub)
                __surfaceItem.pipSub = root.__layoutInfo.isPipSub;
            }

            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] [FitSurfaceItem-END] update surfaceitem's position and scale. x = " + __surfaceItem.x +",  y = " + __surfaceItem.y + " , scale = " + __surfaceItem.scale)
        }
    }

    function setItemFocus(focus) {
        if (root.__surfaceItem) {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] setItemFocus = " + focus + " for item = " + root.__surfaceItem.appId)
        }

        if (focus) {
            if (groupModelLoader.item)
                groupModelLoader.item.updateGroup();
            else if (root.__surfaceItem)
                root.__surfaceItem.takeFocus();
            compositor.updateCursorFocus();
            compositor.updateCursorSettings(cursorFps, __restoreCursorPositionSetting, cursorDisplay);
            if(cloudgameActivefromWinProp || cloudgameActivefromAppInfo) {
                compositor.updateCloudgameActiveState(true);
            } else {
                compositor.updateCloudgameActiveState(false);
            }
        } else {
            if (groupModelLoader.item)
                groupModelLoader.item.resetFocus();
            else if (root.__surfaceItem)
                root.__surfaceItem.focus = focus;
        }
    }

    function setHiddenContainer() {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] " + "set hidden container")
        root.directDestroy = true
        root.__updateCoverState(SurfaceItem.CoverStateHidden)
        root.visible = false
    }

    function unsetHiddenContainer() {
        console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] " + "unset hidden container")
        root.directDestroy = false
        root.__updateCoverState(SurfaceItem.CoverStateNormal)
        root.visible = true
    }

    onCursorFpsChanged: {
        console.info("[FULLSCREEN] onCursorFpsChanged FPS : " + cursorFps + "restoreCursorPos : " + __restoreCursorPositionSetting + "CursorDisplay : " + cursorDisplay);
        compositor.updateCursorSettings(cursorFps, __restoreCursorPositionSetting, cursorDisplay);
    }

    onCursorDisplayChanged: {
        console.info("[FULLSCREEN] onCursorDisplayChanged FPS : " + cursorFps + "restoreCursorPos : " + __restoreCursorPositionSetting + "CursorDisplay : " + cursorDisplay);
        compositor.updateCursorSettings(cursorFps, __restoreCursorPositionSetting, cursorDisplay);
    }

    onCloudgameActivefromWinPropChanged: {
        console.info("[FULLSCREEN:onCloudgameActiveChanged] cloudgame_active : "+ cloudgameActivefromWinProp);
        compositor.updateCloudgameActiveState(cloudgameActivefromWinProp);
    }

    onHeightChanged: {
        //console.info("[FULLSCREEN] "+ root.objectName + " x = " + root.x + " y = " + root.y + " width = " + root.width + " height = " + root.height  + " rotation: " +root.rotation)
        __fitSurfaceItemToParent()
    }

    onControlModeChanged: {
        if (root.controlMode)
            controlModeGuideTimer.restart()
    }

    onAnimationStateChanged: {
        if (isMultiViewMode()) {
            views.fullscreen.multiViewLayoutAnimationStateChanged()
        }
    }

    onCurrentModeChanged: {
        ssd.inputModeRequested = false // if mode is changed, inputmode should be released
    }

    onVisibleChanged: console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] visible: " +root.visible)

    function getLayoutAnimationState(){
        var state = new Object() ;
        state.order = root.__layoutInfo.order ;
        state.state = animationState ;
        return state
    }

    Connections {
        target: views.fullscreen
        onUserInputOnControlMode: {
            if (!controlModeGuideTimer.running)
                controlModeGuideTimer.restart()
        }
        onFocusedSurfaceItemChanged: {
            if(!focus || !target.isMultiViewMode || (root.objectName === "baseContainer" && !target.baseContainerInMultiview)) return

            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] focused item's order:" + order + " order " + root.__layoutInfo.order + "/"+ order )
            if(root.__layoutInfo.order != order) {
                ssd.inputModeRequested = false  // release inputmode
            }else { //focused app
                target.__setCurrentItem(getSurfaceItem())
            }
        }

        onRequestShowVolumeUI: {
            if(!target.isMultiViewMode || (root.objectName === "baseContainer" && !target.baseContainerInMultiview)) return
            ssd.showVolumeUI(true)
        }

        onRequestShowIndexForApps: {
            if(!target.isMultiViewMode || (root.objectName === "baseContainer" && !target.baseContainerInMultiview)) return
            ssd.showIndexForApps(visible)
       }

        onShowToolbar: {
            if(!target.isMultiViewMode ||
              (root.objectName === "baseContainer" && !target.baseContainerInMultiview) ||
              (root.__layoutInfo.usageType === "appView") ||
               target.isControlMode() ||
               delayAnimationTimer.running ||
               containerAnimator.running ) {
                return
            }

            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] order:", order, "----------------------------")
            if(root.__layoutInfo.order != order) {
                root.setShowToolbarStatus(false)
                root.z = root.__layoutInfo.z
                ssd.expireTimer.stop()
                if(!target.isInputMode()) {
                    ssd.setFocus(false)
                }
                ssd.state = "active_init"
                ssd.setVisible(false)
            } else{
                root.setShowToolbarStatus(true)
                root.z = root.__layoutInfo.z + 1
                if(!target.isInputMode()){
                    ssd.setFocus(true)
                }
                ssd.state = (ssd.state !== "active_inputmode" ? "active_focused" : ssd.state)
                ssd.setVisible(true)
                ssd.requestFullScreenTimerRestart()
            }
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] order:", order, "z", root.z, "/", root.__layoutInfo.z)
        }
    }

    property Timer controlModeGuideExpireTimer: Timer {
        id: controlModeGuideTimer
        running: false
        repeat: false
        interval: Settings.local.multiviewControlMode.timeout
    }
    property Timer appViewClosingTimer: Timer {
        id: appViewClosingTimer
        running: false
        repeat: false
        interval: 300
        onTriggered: {
            console.info("[FULLSCREEN:Container][" + root.__layoutInfo.order + "] appView closing timer is done")
            __updateCoverState(SurfaceItem.CoverStateNormal)
        }
    }

    function startAppViewClosingCover() {
        console.info("[FULLSCREEN:Container][" + root.__layoutInfo.order + "] appView closing timer start")
        __updateCoverState(SurfaceItem.CoverStateHidden)
        appViewClosingTimer.restart()
    }

    property Timer delayAnimationTimer: Timer {
        id: delayAnimationTimer
        property int delayInterval: 50
        running: false
        interval: delayInterval
        onTriggered: {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] delay animation timer is triggered ")

            if (root.coverState === SurfaceItem.CoverStateNormal || root.coverState === SurfaceItem.CoverStateChanging) {
                if (__beforeAnimation) {
                    console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] start delay timer to hide video")
                    __updateCoverState(SurfaceItem.CoverStateHidden)
                    delayAnimationTimer.delayInterval = 150
                    delayAnimationTimer.restart()
                } else {
                    __fitSurfaceItemToParent()
                    console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] hide cover UI on container")
                    fullscreenCover.visible = false
                    __beforeAnimation = true
                    root.animationState = "ready"

                    if (root.__layoutInfo.visibleState === false) {
                        root.x = 0
                        root.y = 0
                        root.__layoutInfo.x = 0
                        root.__layoutInfo.y = 0
                    }
                    __setPendingLayoutInfo()
                    if (fullscreenCover.visible === false)
                        __updateCoverState(SurfaceItem.CoverStateNormal)

                    root.applyLayoutDone(root)
                }

            } else if (root.coverState === SurfaceItem.CoverStateHidden) {
                if (__beforeAnimation) {
                    console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] start animation")
                    containerAnimator.restart()
                } else {
                    if (root.__pendingLayoutInfo.isPending) {
                        __updateCoverState(SurfaceItem.CoverStateChanging)
                    } else {
                        __updateCoverState(SurfaceItem.CoverStateNormal)
                    }

                    console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] start delay timer to show video cover state = " + coverState)

                    delayAnimationTimer.delayInterval = 300
                    delayAnimationTimer.restart()
                }
            }
        }
    }

    Connections {
        target: containerAnimator
        onRunningChanged: {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] containerAnimator running = " + containerAnimator.running)
            if (!containerAnimator.running) {
                __beforeAnimation = false
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Container animation is finished")
                __fitSurfaceItemToParent()

                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] after animation, start delay timer to wait graphic rendering ")
                delayAnimationTimer.delayInterval = 50
                delayAnimationTimer.restart()
            }
        }
    }

    ParallelAnimation {
        id : containerAnimator
        property int aniDuration: 500
        PropertyAnimation  {target: root; property: "x"; to: __layoutInfo.x; easing.type: Easing.Linear; duration: containerAnimator.aniDuration}
        PropertyAnimation  {target: root; property: "y"; to: __layoutInfo.y; easing.type: Easing.Linear;  duration: containerAnimator.aniDuration}
        PropertyAnimation  {target: root; property: "z"; to: __layoutInfo.z; easing.type: Easing.Linear; duration: containerAnimator.aniDuration}
        PropertyAnimation  {target: root; property: "width"; to: __layoutInfo.width; easing.type: Easing.Linear; duration: containerAnimator.aniDuration}
        PropertyAnimation  {target: root; property: "height"; to: __layoutInfo.height; easing.type: Easing.Linear; duration: containerAnimator.aniDuration}
   }

    Connections {
        target: __surfaceItem
        onStateChanged: {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceItem state is changed to "  + root.__surfaceItem.state);

            if (groupModelLoader.item)
                groupModelLoader.item.updateGroupState(__surfaceItem.state);

            root.appStateChanged()
        }

        onSurfaceGroupChanged: {
            if (__surfaceItem)
                console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceItem's surfaceGroup changed to = " + root.__surfaceItem.surfaceGroup);

            __resetGroupedItems()
            if (__surfaceItem && __surfaceItem.surfaceGroup)
                __setGroupedOwnerItems(__surfaceItem)
            __changeGroupedStatus()
        }

        // There is a case that surface item's geometry is set after surface item is mapped.
        // If handling both onWidthChanged and onHeightChanged, fitSurfaceItemToParent is called twice.
        // Also, width and height are changed at the same time on fullscreen case. So, handle only one signal.
        onWidthChanged: {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surface item's width is updated. Check surface item's scale.")
            __fitSurfaceItemToParent()
        }

        onOrientationChanged: {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceitem's orientation is changed. orientation = " +root.__surfaceItem.orientation);
            if (root.__surfaceItem && root.__surfaceItem.orientation >= 0) {
                __updateLayoutInfoBasedOnRotation();
                applyNewLayout()
            }
        }

        onEnabledChanged : console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] surfaceitem's enabled is changed. enabled = " +root.__surfaceItem.enabled);
    }

    Loader {
        id: groupModelLoader
        asynchronous: true
        onLoaded: {
            console.info("[FULLSCREEN:Container][" + root.objectName + "][" + root.__layoutInfo.order + "] Group model is loaded. update cover state to " + root.coverState )
            groupModelLoader.item.updateCoverState(root.coverState)
        }
    }

    onLayoutInfoUpdated: {
        var region = Qt.rect(views.starfishViewAdjustments.mx, views.starfishViewAdjustments.my, views.starfishViewAdjustments.mw, views.starfishViewAdjustments.mh);
        var list = StarfishUtils.foregroundList(views.children)
        for (var i = 0; i < list.length; i++) {
            var item = list[i]
            item.zoomValue = views.starfishViewAdjustments.zoomValue
            item.zoomRegion = region
        }
    }

    function makeScreenRequest()
    {
        if( views.starfishViewAdjustments.zoomStatus )
        {
            var screenRect = Qt.rect(
                ( ( compositorWindow.outputGeometry.x - views.starfishViewAdjustments.mx ) * views.starfishViewAdjustments.zoomValue ) * compositorWindow.screenRatio,
                ( ( compositorWindow.outputGeometry.y - views.starfishViewAdjustments.my ) * views.starfishViewAdjustments.zoomValue ) * compositorWindow.screenRatio,
                ( compositorWindow.outputGeometry.width * views.starfishViewAdjustments.zoomValue ) * compositorWindow.screenRatio,
                ( compositorWindow.outputGeometry.height * views.starfishViewAdjustments.zoomValue ) * compositorWindow.screenRatio
            );

            videooutputdCommunicator.setVideoDisplayScreenRequested(
                views.starfishViewAdjustments.zoomStatus,
                views.starfishViewAdjustments.zoomStatus,
                screenRect
            );
        }
        else
        {
            var screenRect = Qt.rect(
                compositorWindow.outputGeometry.x * compositorWindow.screenRatio,
                compositorWindow.outputGeometry.y * compositorWindow.screenRatio,
                compositorWindow.outputGeometry.width * compositorWindow.screenRatio,
                compositorWindow.outputGeometry.height * compositorWindow.screenRatio
            );

            videooutputdCommunicator.setVideoDisplayScreenRequested(
                views.starfishViewAdjustments.zoomStatus,
                views.starfishViewAdjustments.zoomStatus,
                screenRect
            );
        }
    }

    function updateRegion()
    {
        var region = Qt.rect(views.starfishViewAdjustments.mx, views.starfishViewAdjustments.my, views.starfishViewAdjustments.mw, views.starfishViewAdjustments.mh);
        var list = StarfishUtils.foregroundList(views.children)
        for (var i = 0; i < list.length; i++) {
            var item = list[i]
            item.zoomValue = views.starfishViewAdjustments.zoomValue
            item.zoomRegion = region
        }
        if(views.starfishViewAdjustments.zoomStatus)
            compositor.updateZoomInfo(region, views.starfishViewAdjustments.zoomValue);
        else
            compositor.updateZoomInfo(region, 1);

        makeScreenRequest();
    }

    Connections {
        target: views.starfishViewAdjustments

        onMxChanged: {
            if(!views.starfishViewAdjustments.aniRunning)
            {
                if(views.starfishViewAdjustments.mx > (views.starfishViewAdjustments.windowWidth - views.starfishViewAdjustments.mw))
                {
                    views.starfishViewAdjustments.mx = (views.starfishViewAdjustments.windowWidth - views.starfishViewAdjustments.mw)
                    updateRegion()
                }
                else
                {
                    updateRegion()
                }
            }
        }
        onMyChanged: {
            if(!views.starfishViewAdjustments.aniRunning)
            {
                if(views.starfishViewAdjustments.my > (views.starfishViewAdjustments.windowHeight - views.starfishViewAdjustments.mh))
                {
                    views.starfishViewAdjustments.my = (views.starfishViewAdjustments.windowHeight - views.starfishViewAdjustments.mh)
                    updateRegion()
                }
                else
                {
                    updateRegion()
                }
            }
        }
        onMwChanged: {
            updateRegion()
        }
    }
}
