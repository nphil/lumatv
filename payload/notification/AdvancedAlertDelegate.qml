import QtQuick 2.12
import WebOS.Global 1.0
import "NLocaleService"

// LumaTV's input popup (what LG calls an advanced alert), laid over LG's notification tree in place of
// LG's own `AdvancedAlertDelegate.qml` (tools/lumatv, `notifications on`). AdvancedAlertView creates one
// of these per popup and reads exactly the interface LG's file had: `discardTimerInterval`, `alertCardContainer.cards`
// (set by the view: one {title, portIcon, action} per input), `deviceReady`, `portReady`, `animateButtonClose()` and the
// signals `discard()` and `discardAndActivate(cardId)`. The data, the voice-control items and the action that
// switches the input are LG's and untouched.
//
// What differs from LG's: LG's row of parallelogram cards with a skull that closes them, at the bottom of the
// screen under a black scrim, becomes a glass sheet at the top right where the toast goes (the mock,
// 18-device-connected): the input's icon, what happened and the question, a button to switch to each input and
// one to stay, and a bar that runs out with the timer. The popup does not dim the screen (the view's scrim is
// patched to nothing) and closes by itself without switching.
//
// A palette name between two pairs of at-signs is a token (tools/tokens.py), filled in when the tree is staged.

FocusScope {
    id: root
    width: parent ? parent.width : 0
    height: 1

    signal discard()
    signal discardAndActivate(int cardId)
    property alias discardTimerInterval: discardTimer.interval
    property alias alertCardContainer: cardContainer
    property bool deviceReady: false
    property bool portReady: false
    property var style

    readonly property var cards: cardContainer.cards
    readonly property int count: cards ? cards.length : 0
    readonly property string firstName: count > 0 ? String(cards[0].title) : ""
    readonly property string title: count > 1 ? qsTr("%1 inputs connected").arg(count) + NLocaleService.emptyString
                                              : (qsTr("%1 connected").arg(firstName) + NLocaleService.emptyString)
    // The port the question is about (mock: "On HDMI 3. Switch to it?"): LG launches the input's app, com.webos.app.hdmi3, so that is where it is said.
    function portOf(card) {
        var a = card && card.action && card.action.launchParams ? String(card.action.launchParams.id || "") : "";
        var m = /hdmi(\d+)$/i.exec(a);
        return m ? "HDMI " + m[1] : "";
    }
    readonly property string port: count === 1 ? portOf(cards[0]) : ""
    readonly property string subtitle: (count > 1 ? qsTr("Pick one to switch to.") : (port !== "" ? qsTr("On %1. Switch to it?").arg(port) : qsTr("Switch to it?"))) + NLocaleService.emptyString
    readonly property int pad: style.getRValue(20)
    readonly property int tile: style.getRValue(56)

    function animateButtonClose() {}

    // The timer and the bar that shows it run only while a popup is up, and both are started together.
    function rearm() {
        if (discardTimer.interval > 0) {
            discardTimer.restart();
            countdown.restart();
        }
    }

    Connections {
        target: advAlertScrimMouseEventConsumer
        onClicked: {
            discard();
        }
    }

    Connections {
        target: services
        onPerformActionReceived: {
            if (viewType == "ADVALERT" && itemType == "SELECT") {
                notificationRoot.services.voiceControlService.reportVoiceControlActionResult(true);
                root.discardAndActivate(Number(itemIdx));
            }
        }
    }

    ListView.onAdd: root.rearm()

    Timer {
        id: discardTimer
        interval: 10000
        onTriggered: {
            if (model.index !== -1) {
                discard();
            }
        }
    }

    Item {
        id: cardContainer
        property var cards: []
        visible: false
        width: 0
        height: 0
    }

    Rectangle {
        id: sheet
        width: parent.width
        height: content.implicitHeight + root.pad * 2 + bar.height
        radius: root.style.getRValue(@@rowRadius@@)
        color: "@@sheetBottom@@"
        border.width: 1
        border.color: "@@hairline@@"

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            hoverEnabled: true
            onEntered: discardTimer.stop()
            onExited: root.rearm()
        }

        Column {
            id: content
            x: root.pad
            y: root.pad
            width: sheet.width - root.pad * 2
            spacing: root.style.getRValue(14)

            Row {
                id: head
                spacing: root.style.getRValue(18)

                Rectangle {
                    id: iconTile
                    width: root.tile
                    height: width
                    radius: root.style.getRValue(14)
                    color: "@@row@@"
                    Image {
                        anchors.fill: parent
                        anchors.margins: root.style.getRValue(8)
                        source: root.count > 0 && root.cards[0].portIcon ? root.cards[0].portIcon : ""
                        sourceSize.width: width
                        sourceSize.height: height
                        fillMode: Image.PreserveAspectFit
                        smooth: true
                        asynchronous: true
                    }
                }

                Column {
                    width: content.width - iconTile.width - head.spacing
                    spacing: root.style.getRValue(3)
                    anchors.verticalCenter: parent.verticalCenter

                    Text {
                        objectName: "inputPopupTitle"
                        width: parent.width
                        text: root.title
                        color: "@@ink@@"
                        font.family: "Manrope"
                        font.pixelSize: root.style.getRValue(28)
                        font.weight: Font.DemiBold
                        renderType: Text.NativeRendering
                        elide: Text.ElideRight
                    }
                    Text {
                        width: parent.width
                        text: root.subtitle
                        color: "@@inkDim@@"
                        font.family: "Manrope"
                        font.pixelSize: root.style.getRValue(21)
                        renderType: Text.NativeRendering
                        elide: Text.ElideRight
                    }
                }
            }

            Row {
                id: buttons
                x: iconTile.width + head.spacing
                spacing: root.style.getRValue(14)

                Repeater {
                    id: switchButtons
                    model: root.count

                    SimpleButton {
                        id: switchButton
                        objectName: "inputPopupSwitch"
                        text: qsTr("Switch to %1").arg(root.port !== "" ? root.port : root.cards[index].title) + NLocaleService.emptyString
                        maxWidth: root.style.getRValue(380)
                        width: Math.max(root.style.getRValue(150), label.actualWidth + 10 + buttonMargin * 2)
                        height: root.style.getRValue(52)
                        fontSize: root.style.getRValue(24)
                        bgColor: hasFocus ? Qt.lighter("@@accent@@", 1.18) : "@@accent@@"
                        textColor: "@@accentInk@@"
                        style: root.style
                        focus: index === 0
                        onSelected: root.discardAndActivate(index)
                        onHasFocusChanged: {
                            if (hasFocus) {
                                root.rearm();
                                notificationRoot.services.speak(root.cards[index].title, notificationRoot.services.speechFCCLevel, true);
                            }
                        }
                        Component.onCompleted: {
                            if (index > 0) {
                                var prev = switchButtons.itemAt(index - 1);
                                prev.KeyNavigation.right = switchButton;
                                switchButton.KeyNavigation.left = prev;
                            }
                            if (index === root.count - 1) {
                                switchButton.KeyNavigation.right = stay;
                                stay.KeyNavigation.left = switchButton;
                            }
                        }
                    }
                }

                SimpleButton {
                    id: stay
                    objectName: "inputPopupStay"
                    text: qsTr("Stay here") + NLocaleService.emptyString
                    maxWidth: root.style.getRValue(380)
                    width: Math.max(root.style.getRValue(150), label.actualWidth + 10 + buttonMargin * 2)
                    height: root.style.getRValue(52)
                    fontSize: root.style.getRValue(24)
                    bgColor: hasFocus ? Qt.tint("@@row@@", "@@focus@@") : "@@row@@"
                    textColor: "@@ink@@"
                    style: root.style
                    onSelected: root.discard()
                    onHasFocusChanged: {
                        if (hasFocus) {
                            root.rearm();
                            notificationRoot.services.speak(text, notificationRoot.services.speechFCCLevel, true);
                        }
                    }
                    Component.onCompleted: {
                        if (root.count === 0) stay.focus = true;
                    }
                }
            }
        }

        // The time left, drawn as a bar along the foot of the sheet.
        Item {
            id: bar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.leftMargin: root.pad
            anchors.rightMargin: root.pad
            anchors.bottomMargin: root.style.getRValue(10)
            height: root.style.getRValue(4) + anchors.bottomMargin
            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: root.style.getRValue(4)
                radius: height / 2
                color: "@@row@@"
            }
            Rectangle {
                id: fill
                anchors.left: parent.left
                anchors.bottom: parent.bottom
                width: parent.width
                height: root.style.getRValue(4)
                radius: height / 2
                color: "@@inkDim@@"
            }
            NumberAnimation {
                id: countdown
                target: fill
                property: "width"
                from: bar.width
                to: 0
                duration: discardTimer.interval
            }
        }
    }
}
