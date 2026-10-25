import QtQuick 2.12
import WebOS.Global 1.0
import "NLocaleService"
import "Utils.js" as Utils

// LumaTV's alert, laid over LG's notification tree in place of LG's own `Alert.qml` (tools/lumatv,
// `notifications on`). AlertView creates one of these per alert and reads exactly the interface LG's
// file had: the properties below, `buttonClicked(index)`, `closeClicked()`, `discard()`, the timer
// intervals and the progress update. The focus, text-to-speech and voice-control code, the discard timer and the
// way a button or the check box takes the keys are LG's behaviour kept as it was; only the drawing is new.
//
// What differs from LG's: a card in the middle of the screen (the mock, 06-confirm-dialog) with the app's icon
// and the title above the message, where LG draws the message only (LG shows a title for a full alert only), and a
// row of buttons under it, where LG stacks them in a column at the right. A button that asks for `danger: true`
// is drawn in the danger colour. No `layer.enabled`, no DropShadow and no ConicalGradient: a live blurred copy per
// item is what the frame gate forbids, and the card's own edge is enough against Home or video.
//
// A palette name between two pairs of at-signs is a token (tools/tokens.py), filled in when the tree is staged.

FocusScope {
    id: root

    property string sourceId: ""
    property url iconUrl
    property url thumbnailPath
    property string title
    property string message
    property string readable
    property string checkBoxMessage

    property bool modal: false
    property var closeAction
    property variant style

    property alias buttonModel: buttonRepeater.model
    property bool isFullAlert: false
    property bool isCheckBoxAlert: false
    property alias checkValue: checkBox.checked
    property bool isProgressAlert: false
    property bool isSpinnerAlert: false
    property bool showTimeoutValue: false
    property int progressValue
    property bool isPartialScreen: root.style.partialScreenView

    onIsPartialScreenChanged: {
        if (!isPartialScreen) {
            discard();
        }
    }

    signal buttonClicked(int btnIndex)
    signal closeClicked()
    signal discard()
    property alias discardTimerInterval: discardTimer.interval
    property string autoTimeoutIntervalValue: ""
    property int shownProgress: 0

    readonly property bool highContrast: notificationRoot.services.isHighContrast
    readonly property color dangerColor: "@@danger@@"
    readonly property bool hasIcon: iconImage.status === Image.Ready
    readonly property bool hasThumb: thumbImage.status === Image.Ready
    readonly property int pad: style.getRValue(40)
    readonly property int cardWidth: isFullAlert ? width : Math.min(width, style.getRValue(hasThumb ? 980 : 820))

    width: services.isSeroMode ? notificationRoot.height : notificationRoot.width
    height: services.isSeroMode ? notificationRoot.width : notificationRoot.height

    Connections {
        target: services
        onPerformActionReceived: {
            if (viewType == "ALERT" && itemType == "SELECT") {
                notificationRoot.services.voiceControlService.reportVoiceControlActionResult(true);
                root.buttonClicked(Number(itemIdx));
            }
        }

        onUpdatePrgoressAlert: {
            if (progressValue >= 0 && progressValue <= 100)
                root.shownProgress = progressValue;
        }
    }

    onProgressValueChanged: shownProgress = progressValue

    onFocusChanged: {
        if (focus) {
            var ttsText = (readable) ? (title || "") + "\n" + (readable || "") + "\n" : (title || "") + "\n" + (message || "") + "\n";
            var i;
            notificationRoot.services.voiceControlService.dropVoiceControlItems();

            for (i = 0; i < buttonModel.length; i++) {
                var itemId = "ALERT_SELECT_" + i;
                var values = [buttonModel[i].label];
                notificationRoot.services.voiceControlService.addVoiceControlItem("Select", itemId, values);
                ttsText += (buttonModel[i].label || "") + "\n";
            }

            notificationRoot.services.putTtsChain(Utils.replaceNewLineTags(ttsText, ", "), "title")
        }

        if (!focus || buttonRepeater.count < 1 || root.buttonModel.length !== buttonRepeater.count)
            return;

        buttonRepeater.itemAt(0).focus = true;
        for (var i = 0; i < root.buttonModel.length; i++) {
            if (root.buttonModel[i].focus) {
                buttonRepeater.itemAt(i).focus = focus;
                break;
            }
        }
    }

    Rectangle {
        id: card
        width: root.cardWidth
        height: root.isFullAlert ? parent.height : content.implicitHeight + root.pad * 2
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.verticalCenter: parent.verticalCenter
        radius: root.isFullAlert ? 0 : root.style.getRValue(@@radius@@)
        color: root.highContrast ? "#000000" : "@@sheetBottom@@"
        gradient: root.highContrast ? null : sheetGradient
        border.width: root.isFullAlert ? 0 : root.style.getRValue(1)
        border.color: root.highContrast ? "@@ink@@" : "@@hairline@@"

        Gradient {
            id: sheetGradient
            GradientStop { position: 0; color: "@@sheetTop@@" }
            GradientStop { position: 1; color: "@@sheetBottom@@" }
        }

        MouseArea {
            id: mouseBlocker
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: {}
            onEntered: {}
            onDoubleClicked: {}
            onWheel: {}
            onPressed: {}
            onPressAndHold: {}
            hoverEnabled: true
        }

        Column {
            id: content
            x: root.pad
            y: root.isFullAlert ? Math.max(root.pad, (card.height - implicitHeight) / 2) : root.pad
            width: card.width - root.pad * 2
            spacing: root.style.getRValue(root.isFullAlert ? 36 : 26)

            Image {
                id: thumbImage
                visible: status === Image.Ready
                source: root.thumbnailPath ? root.thumbnailPath : ""
                width: root.style.getRValue(root.isFullAlert ? 320 : 240)
                height: width * 9 / 16
                sourceSize.width: width
                sourceSize.height: height
                fillMode: Image.PreserveAspectFit
                anchors.horizontalCenter: root.isFullAlert ? parent.horizontalCenter : undefined
                smooth: true
            }

            Row {
                id: header
                visible: root.title !== "" || root.hasIcon
                spacing: root.style.getRValue(22)
                anchors.horizontalCenter: root.isFullAlert ? parent.horizontalCenter : undefined

                Rectangle {
                    id: iconTile
                    visible: root.hasIcon
                    width: root.style.getRValue(root.isFullAlert ? 120 : 72)
                    height: width
                    radius: root.style.getRValue(root.isFullAlert ? 28 : 18)
                    color: "@@row@@"
                    anchors.verticalCenter: parent.verticalCenter
                    Image {
                        id: iconImage
                        anchors.fill: parent
                        anchors.margins: root.style.getRValue(8)
                        source: root.iconUrl ? root.iconUrl : ""
                        sourceSize.width: width
                        sourceSize.height: height
                        fillMode: Image.PreserveAspectFit
                        smooth: true
                    }
                }

                Text {
                    id: titleText
                    objectName: "alertTitle"
                    visible: root.title !== ""
                    width: Math.min(implicitWidth, content.width - (iconTile.visible ? iconTile.width + header.spacing : 0))
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.title
                    color: "@@ink@@"
                    font.family: "Manrope"
                    font.pixelSize: root.style.getRValue(root.isFullAlert ? 64 : 38)
                    font.weight: Font.DemiBold
                    renderType: Text.NativeRendering
                    wrapMode: Text.Wrap
                    elide: Text.ElideRight
                    maximumLineCount: 2
                }
            }

            Text {
                id: alertText
                objectName: "alertText"
                visible: text !== ""
                width: parent.width
                text: root.message
                color: root.title !== "" ? "@@ink@@" : "@@ink@@"
                opacity: root.title !== "" ? 0.92 : 1
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(root.isFullAlert ? 39 : (root.title !== "" ? 28 : 32))
                font.weight: Font.Normal
                lineHeightMode: Text.FixedHeight
                lineHeight: root.style.getRValue(root.isFullAlert ? 54 : (root.title !== "" ? 38 : 42)) * (NLocaleService.isTallglyph ? 1.34 : 1.0)
                horizontalAlignment: root.isFullAlert ? Text.AlignHCenter : Text.AlignLeft
                renderType: Text.NativeRendering
                wrapMode: Text.Wrap
                elide: Text.ElideRight
                maximumLineCount: root.style.alert.maxLineCount
            }

            Item {
                id: checkBox
                property bool checked: false
                visible: root.isCheckBoxAlert
                focus: false
                width: parent.width
                height: visible ? root.style.getRValue(44) : 0
                activeFocusOnTab: false

                function changeValue() { checked = !checked }

                Rectangle {
                    id: box
                    width: root.style.getRValue(36)
                    height: width
                    radius: root.style.getRValue(10)
                    anchors.verticalCenter: parent.verticalCenter
                    color: checkBox.checked ? "@@accent@@" : "transparent"
                    border.width: checkBox.activeFocus ? 3 : 2
                    border.color: checkBox.activeFocus ? "@@focusEdge@@" : (checkBox.checked ? "@@accent@@" : "@@inkDim@@")
                    Rectangle {
                        anchors.centerIn: parent
                        visible: checkBox.checked
                        width: root.style.getRValue(14)
                        height: width
                        radius: width / 2
                        color: "@@accentInk@@"
                    }
                }
                Text {
                    anchors.left: box.right
                    anchors.leftMargin: root.style.getRValue(18)
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.checkBoxMessage
                    color: "@@ink@@"
                    font.family: "Manrope"
                    font.pixelSize: root.style.getRValue(26)
                    renderType: Text.NativeRendering
                    elide: Text.ElideRight
                }
                WebOSMouseArea {
                    cursorVisible: notificationRoot.cursorVisible
                    z: 10
                    anchors.fill: parent
                    hoverEnabled: true
                    propagateComposedEvents: true
                    onClicked: checkBox.changeValue()
                }

                Keys.onReleased: {
                    switch (event.key) {
                    case Qt.Key_Return:
                    case Qt.Key_Enter:
                        changeValue();
                        break;
                    case Qt.Key_Left:
                        if (NLocaleService.isRTLLocale && buttonRepeater.count > 0) buttonRepeater.itemAt(0).focus = true;
                        break;
                    case Qt.Key_Right:
                        if (!NLocaleService.isRTLLocale && buttonRepeater.count > 0) buttonRepeater.itemAt(0).focus = true;
                        break;
                    }
                }
            }

            Item {
                id: progressBox
                visible: root.isProgressAlert
                width: parent.width
                height: visible ? root.style.getRValue(10) : 0
                Rectangle {
                    anchors.fill: parent
                    radius: height / 2
                    color: "@@row@@"
                }
                Rectangle {
                    width: parent.width * root.shownProgress / 100
                    height: parent.height
                    radius: height / 2
                    color: "@@accent@@"
                    Behavior on width { NumberAnimation { duration: 200 } }
                }
            }

            Row {
                id: waitRow
                visible: (root.isSpinnerAlert || root.showTimeoutValue)
                spacing: root.style.getRValue(18)
                height: visible ? root.style.getRValue(44) : 0

                // A ring with one lit arc end, turned on the render thread: nothing is redrawn per frame.
                Item {
                    id: spinner
                    visible: root.isSpinnerAlert && !root.showTimeoutValue
                    width: root.style.getRValue(40)
                    height: width
                    Rectangle {
                        anchors.fill: parent
                        radius: width / 2
                        color: "transparent"
                        border.width: root.style.getRValue(5)
                        border.color: "@@row@@"
                    }
                    Item {
                        anchors.fill: parent
                        RotationAnimator on rotation { from: 0; to: 360; duration: 1000; loops: Animation.Infinite; running: spinner.visible && root.visible }
                        Rectangle {
                            width: root.style.getRValue(10)
                            height: width
                            radius: width / 2
                            color: "@@accent@@"
                            anchors.horizontalCenter: parent.horizontalCenter
                        }
                    }
                }
                Text {
                    visible: root.showTimeoutValue
                    anchors.verticalCenter: parent.verticalCenter
                    text: root.autoTimeoutIntervalValue
                    color: "@@inkDim@@"
                    font.family: "Manrope"
                    font.pixelSize: root.style.getRValue(26)
                    font.weight: Font.DemiBold
                    renderType: Text.NativeRendering
                }
            }

            Row {
                id: buttonRow
                visible: buttonRepeater.count > 0
                spacing: root.style.getRValue(18)
                layoutDirection: Qt.LeftToRight
                x: root.isFullAlert ? (parent.width - width) / 2 : parent.width - width

                Repeater {
                    id: buttonRepeater

                    SimpleButton {
                        id: alertButton

                        readonly property bool danger: modelData.danger === true

                        text: modelData.label
                        maxWidth: root.style.getRValue(420)
                        width: Math.max(root.style.getRValue(root.isFullAlert ? 200 : 160), label.actualWidth + 10 + buttonMargin * 2)
                        height: root.style.getRValue(root.isFullAlert ? 76 : 64)
                        fontSize: root.style.getRValue(root.isFullAlert ? 32 : 28)
                        bgColor: hasFocus ? (danger ? Qt.rgba(root.dangerColor.r, root.dangerColor.g, root.dangerColor.b, 0.34) : Qt.tint("@@row@@", "@@focus@@"))
                                          : (danger ? Qt.rgba(root.dangerColor.r, root.dangerColor.g, root.dangerColor.b, 0.16) : "@@row@@")
                        textColor: danger ? root.dangerColor : "@@ink@@"
                        onSelected: root.buttonClicked(index);
                        style: root.style

                        onHasFocusChanged: {
                            if (discardTimer.running)
                                discardTimer.restart();
                            if (hasFocus) {
                                alertButton.z = alertButton.z + 1;
                                notificationRoot.services.putTtsChain(alertButton.text + " " + qsTr("button") + NLocaleService.emptyString, "focused")
                            } else {
                                alertButton.z = alertButton.z - 1;
                            }
                        }

                        Keys.onReleased: {
                            switch (event.key) {
                            case Qt.Key_Left:
                                if (isCheckBoxAlert && index === 0 && !NLocaleService.isRTLLocale) checkBox.focus = true;
                                break;
                            case Qt.Key_Right:
                                if (isCheckBoxAlert && index === 0 && NLocaleService.isRTLLocale) checkBox.focus = true;
                                break;
                            }
                        }
                    }

                    onItemAdded: {
                        if (index > 0) {
                            var prev = buttonRepeater.itemAt(index - 1);
                            prev.KeyNavigation.right = item;
                            prev.KeyNavigation.down = item;
                            item.KeyNavigation.left = prev;
                            item.KeyNavigation.up = prev;
                        }

                        if (index === 0 || root.buttonModel[index].focus === true)
                            item.focus = true;
                    }
                }
            }
        }
    }

    Timer {
        id: discardTimer

        interval: 5 * 1000
        running: (interval > 0)
        onTriggered: {
            getTextDecrement.start()
            discard();
        }
    }

    Timer {
        id: getTextDecrement
        interval: 1000
        running: root.showTimeoutValue && discardTimer.running
        repeat: true
        onTriggered: {
            if (Number(autoTimeoutIntervalValue) > 0) {
                autoTimeoutIntervalValue = String(Number(autoTimeoutIntervalValue) - 1);
            } else {
                discardTimer.stop()
            }
        }
    }
}
