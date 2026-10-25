import QtQuick 2.12
import WebOS.Global 1.0
import "NLocaleService"
import "Utils.js" as Utils

// LumaTV's PIN prompt, laid over LG's notification tree in place of LG's own `PincodePrompt.qml` (tools/lumatv,
// `notifications on`). PincodePromptView creates one of these per prompt and reads exactly the interface LG's file
// had: `title`, `message`, `retry`, `isFirstUse`, `aboutToClose`, `password`, the signals `postPassword()` and
// `postExitSignal()` and the prompt-title and prompt-message functions. How a PIN is taken is LG's, unchanged: a
// digit goes into the next of four boxes, Back (or the backspace key) takes the last one out and, with nothing
// entered, closes the prompt; the key that sends it is live only when all four boxes are full and takes the focus then;
// the number keys of the remote work as the pad's keys do; and the arrow-only secret mode (`secretMode`) is kept.
// The audio guidance and the voice-control items are LG's.
//
// What differs from LG's: the pad is the mock's (19-pin-prompt) in a card in the middle of the screen, three keys wide
// with Clear, 0 and Back under the nine, where LG draws one row of ten keys across a black screen; the card has the
// same lift-and-brighter-glass focus as everything else and no DropShadow. Nothing here listens to anything but
// the remote: the keys, and the pointer's clicks on the pad. The answer leaves the view only through
// `closePincodePrompt`, as in LG's.
//
// A palette name between two pairs of at-signs is a token (tools/tokens.py), filled in when the tree is staged.

FocusScope {
    id: root

    property string title
    property string message
    property string parental_title: qsTr("Enter Password") + NLocaleService.emptyString  // i18n mode description : the title of password prompt
    property string parental_msg: qsTr("Please enter password.") + NLocaleService.emptyString  // i18n mode description : the message of password prompt
    property string retry_title: qsTr("Check Password") + NLocaleService.emptyString  // i18n mode description : the title of password prompt to check
    property string retry_msg: qsTr("Incorrect password. Please enter correct password.") + NLocaleService.emptyString  // i18n mode description : the message of password prompt for the incorrect password
    property string setmatch_title: qsTr("Enter Password") + NLocaleService.emptyString  // i18n mode description : the title of password prompt
    property string setmatch_msg: qsTr("Enter the current password to reset password.") + NLocaleService.emptyString  // i18n mode description : the title of password prompt to set at the first time
    property string defaultPassword_msg: qsTr("(Default Password : “%1”)") + NLocaleService.emptyString  // i18n mode description : default password text

    property string setnewpin_title: qsTr("Set Password") + NLocaleService.emptyString  // i18n mode description : the title of password prompt for new one
    property string setnewpin_msg: qsTr("Please enter new password.") + NLocaleService.emptyString  // i18n mode description : the message of password prompt for new one
    property string setnewpin_msg_firstuse: qsTr("You can lock or unlock channels and inputs.") + NLocaleService.emptyString  // i18n mode description : the message of password prompt for new one on first use
    property string notpermitted_title: qsTr("Check Password") + NLocaleService.emptyString  // i18n mode description : the title of password prompt to check
    property string notpermitted_msg: qsTr("Entered number is not permitted.") + " " + qsTr("Please enter a different password.") + NLocaleService.emptyString  // i18n mode description : the message of password prompt for the unpermitted password
    property string verify_title: qsTr("Confirm Password") + NLocaleService.emptyString  // i18n mode description : the title of password prompt to check
    property string verify_msg: qsTr("Please re-enter the password for confirmation.") + NLocaleService.emptyString  // i18n mode description : the message of password prompt to confirm
    property string retry_verify_title: qsTr("Check Password") + NLocaleService.emptyString  // i18n mode description : the title of password prompt to check
    property string retry_verify_msg: qsTr("Incorrect password. Please enter correct password.") + NLocaleService.emptyString  // i18n mode description : the message of password prompt for the incorrect password
    property string enterButtonText: qsTr("OK") + NLocaleService.emptyString
    property string clearButtonText: qsTr("Clear") + NLocaleService.emptyString
    property string password
    property string hintText: qsTr("Number keys on the remote work too") + NLocaleService.emptyString
    property string enterpincode_guide_msg: qsTr("Use the pointer or number buttons on your remote.<br>Press the BACK button to erase the number.") + NLocaleService.emptyString //i18n the message at the right of password box items
    property string tts_hidden_msg: qsTr("hidden") + NLocaleService.emptyString  // i18n audio guidance, when PIN code number is inputted
    property string deactivated_msg: qsTr("Deactivated") + NLocaleService.emptyString  // i18n audio guidance, when focusing deactivated button
    property string backspace_msg: qsTr("Back Space") + NLocaleService.emptyString  // i18n audio guidance, when focusing back space button

    property var retry
    property variant style
    property bool modal: true
    property bool isRTL: LayoutMirroring.enabled
    property bool partialRTL
    property bool isFirstUse: false
    property bool messageToBody: false
    property bool isFullType: true
    property bool isPartialScreen: root.style.partialScreenView

    property string defaultPassword: notificationRoot.services.defaultPin

    width: services.isSeroMode ? notificationRoot.height : notificationRoot.width
    height: services.isSeroMode ? notificationRoot.width : notificationRoot.height

    property bool containsMouse: mouseArea.containsMouse
    property bool aboutToClose: false

    readonly property int digits: root.style.pincodePrompt.numOfPad
    readonly property int boxes: root.style.pincodePrompt.numOfPWPad
    readonly property bool full: inputFieldIndex > root.boxes - 1
    readonly property int pad: root.style.getRValue(40)
    readonly property int keyWidth: root.style.getRValue(124)
    readonly property int keyHeight: root.style.getRValue(68)
    readonly property int keyGap: root.style.getRValue(12)

    signal postPassword()
    signal postExitSignal()

    Connections {
        target: services
        onPerformActionReceived: {
            if (viewType == "PINCODE" && itemType == "SELECT") {
                switch (itemIdx) {
                    case "BACK":
                        notificationRoot.services.voiceControlService.reportVoiceControlActionResult(true);
                        root.handleBackKey();

                        break;
                    case "CONFIRM":
                        if (root.inputFieldIndex === root.boxes) {
                            notificationRoot.services.voiceControlService.reportVoiceControlActionResult(true);
                            root.confirmClicked();
                        } else {
                            notificationRoot.services.voiceControlService.reportVoiceControlActionResult(false);
                        }
                        break;
                    default:
                        notificationRoot.services.voiceControlService.reportVoiceControlActionResult(true);
                        root.numpadClicked(itemIdx);
                        break;
                }
            }
        }
    }

    onContainsMouseChanged: {
        if (containsMouse)
            notificationRoot.clearMouseFocus();
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        propagateComposedEvents: true
    }

    Rectangle {
        id: card
        width: root.style.getRValue(640)
        height: content.implicitHeight + root.pad * 2
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.verticalCenter: parent.verticalCenter
        radius: root.style.getRValue(@@radius@@)
        color: notificationRoot.services.isHighContrast ? "#000000" : "@@sheetBottom@@"
        gradient: notificationRoot.services.isHighContrast ? null : sheetGradient
        border.width: root.style.getRValue(1)
        border.color: notificationRoot.services.isHighContrast ? "@@ink@@" : "@@hairline@@"

        Gradient {
            id: sheetGradient
            GradientStop { position: 0; color: "@@sheetTop@@" }
            GradientStop { position: 1; color: "@@sheetBottom@@" }
        }

        Column {
            id: content
            x: root.pad
            y: root.pad
            width: card.width - root.pad * 2
            spacing: root.style.getRValue(18)

            // A padlock from two shapes: no icon font on the set is guaranteed to have one.
            Rectangle {
                id: lockTile
                width: root.style.getRValue(64)
                height: width
                radius: root.style.getRValue(18)
                color: "@@row@@"
                border.width: 1
                border.color: "@@hairline@@"
                anchors.horizontalCenter: parent.horizontalCenter
                Rectangle {
                    width: root.style.getRValue(18)
                    height: root.style.getRValue(22)
                    radius: width / 2
                    color: "transparent"
                    border.width: root.style.getRValue(3)
                    border.color: "@@ink@@"
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: parent.height / 2 - root.style.getRValue(20)
                }
                Rectangle {
                    width: root.style.getRValue(28)
                    height: root.style.getRValue(20)
                    radius: root.style.getRValue(5)
                    color: "@@ink@@"
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: parent.height / 2 - root.style.getRValue(4)
                }
            }

            Text {
                id: pincodePromptText
                objectName: "pincodePromptText"
                width: parent.width
                text: root.title
                color: "@@ink@@"
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(40)
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignHCenter
                renderType: Text.NativeRendering
                wrapMode: Text.Wrap
                elide: Text.ElideRight
                maximumLineCount: 2
            }

            Text {
                id: pinCodeNormalModeDescription
                objectName: "pinCodeNormalModeDescription"
                visible: text !== ""
                width: parent.width
                text: root.message
                color: root.retry ? "@@danger@@" : "@@inkDim@@"
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(26)
                horizontalAlignment: Text.AlignHCenter
                renderType: Text.NativeRendering
                wrapMode: Text.Wrap
                elide: Text.ElideRight
                maximumLineCount: 3
            }

            Row {
                id: inputBoxRow
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: root.style.getRValue(16)
                LayoutMirroring.enabled: false
                LayoutMirroring.childrenInherit: true

                Repeater {
                    id: repeaterPasswordBox
                    model: root.boxes
                    PincodePromptPasswordButton {
                        style: root.style
                        objectName: "pincodePromptPasswordButton"
                        current: index === inputFieldIndex
                        inprogress: arrowInput.inprogress
                        pressed: arrowInput.pressed
                    }
                }
            }

            /*
            From UX_2014_W1.5_Settings(Product)_v1.6.3_131217:
            - Clicking on the arrow key once enters the first number, clicking on the
              arrow key twice enters the second number, and so on.
            - Clicking the back button deletes the last entered field. If all fields are
              empty, the back key does nothing.
            - Clicking the ok button moves focus to next input field. If focus is on 1st
              input field, the ok key does nothing. If focus is on 4th input box, the ok
              key enters the code and closes pop-up.
            - After N second from entering the number, focus is automatically moved to
              next input field.
            - After N second from entering four numbers, the code is automatically entered
              and pop-up is closed.
            */
            PincodePromptArrowpad {
                id: arrowpad
                anchors.horizontalCenter: parent.horizontalCenter
                style: root.style
                visible: false
                ArrowLockInput {
                    id: arrowInput
                    focus: true
                    onSelected: numpadClicked(digit);
                }
            }

            Text {
                id: secureModeText
                visible: false
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(24)
                color: "@@inkDim@@"
                text: root.messageToBody ? root.message : ""
            }

            FocusScope {
                id: buttonsScope
                width: parent.width
                height: padArea.height + root.style.getRValue(18) + numPadButtonConfirm.height

                Item {
                    id: padArea
                    width: 3 * root.keyWidth + 2 * root.keyGap
                    height: 4 * root.keyHeight + 3 * root.keyGap
                    anchors.horizontalCenter: parent.horizontalCenter
                    LayoutMirroring.enabled: false
                    LayoutMirroring.childrenInherit: true

                    property PincodePromptNumButton lastFocusItem

                    Repeater {
                        id: repeaterNumpad
                        model: root.digits
                        PincodePromptNumButton {
                            id: numpadKey
                            objectName: "pinNumpadKey"
                            // 1-9 in three rows, 0 under the 8
                            readonly property int col: index < 9 ? index % 3 : 1
                            readonly property int row: index < 9 ? Math.floor(index / 3) : 3
                            x: col * (root.keyWidth + root.keyGap)
                            y: row * (root.keyHeight + root.keyGap)
                            width: root.keyWidth
                            height: root.keyHeight
                            text: (index == (root.digits - 1)) ? 0 : (index + 1)
                            style: root.style
                            focus: index === 4
                            enabled: true
                            activated: inputFieldIndex < root.boxes
                            noColorButton: true
                            onActiveFocusChanged: {
                                if (activeFocus) {
                                    padArea.lastFocusItem = numpadKey;
                                    notificationRoot.services.putTtsChain(text + " " + qsTr("button") + NLocaleService.emptyString + (!numpadKey.activeFocus ? (", " + deactivated_msg) : ""), "focused")
                                }
                            }
                        }
                    }

                    PincodePromptNumButton {
                        id: numPadButtonClear
                        objectName: "pinClearButton"
                        text: root.clearButtonText
                        x: 0
                        y: 3 * (root.keyHeight + root.keyGap)
                        width: root.keyWidth
                        height: root.keyHeight
                        textSize: root.style.getRValue(26)
                        style: root.style
                        enabled: true
                        activated: inputFieldIndex > 0
                        noColorButton: true
                        onActivatedChanged: {
                            if (!activated && activeFocus)
                                root.focusLastDigit()
                        }
                        onActiveFocusChanged: if (activeFocus) notificationRoot.services.putTtsChain(text + " " + qsTr("button") + NLocaleService.emptyString + (!numPadButtonClear.activated ? (", " + deactivated_msg) : ""), "focused")
                    }

                    PincodePromptNumButton {
                        id: numPadButtonBack
                        objectName: "pinBackButton"
                        text: "\u2190"
                        x: 2 * (root.keyWidth + root.keyGap)
                        y: 3 * (root.keyHeight + root.keyGap)
                        width: root.keyWidth
                        height: root.keyHeight
                        style: root.style
                        textSize: root.style.getRValue(34)
                        enabled: true
                        activated: inputFieldIndex > 0
                        noColorButton: true
                        onActivatedChanged: {
                            if (!activated && activeFocus)
                                root.focusLastDigit()
                        }
                        onActiveFocusChanged: if (activeFocus)  notificationRoot.services.putTtsChain(backspace_msg + " " + qsTr("button") + NLocaleService.emptyString + (!numPadButtonBack.activated ? (", " + deactivated_msg) : ""), "focused")

                        Component.onCompleted: {
                            var itemId = "PINCODE_SELECT_BACK";
                            var values = ["Back Space"];
                            notificationRoot.services.voiceControlService.addVoiceControlItem("Select", itemId, values);
                        }
                    }
                }

                PincodePromptNumButton {
                    id: numPadButtonConfirm
                    objectName: "pinConfirmButton"
                    text: enterButtonText
                    anchors.top: padArea.bottom
                    anchors.topMargin: root.style.getRValue(18)
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: padArea.width
                    height: root.keyHeight
                    textSize: root.style.getRValue(30)
                    accentFill: true
                    style: root.style
                    enabled: true
                    activated: root.full

                    onActivatedChanged: {
                        if (root.state === "secure")
                            return

                        if (activated)
                            numPadButtonConfirm.focus = true
                        else
                            root.focusLastDigit()
                    }
                    onActiveFocusChanged: {
                        if (activeFocus) {
                            notificationRoot.services.putTtsChain(text + " " + qsTr("button") + (!numPadButtonConfirm.activated ? (", " + deactivated_msg) : ""), "focused")
                        }
                    }

                    Component.onCompleted: {
                        var itemId = "PINCODE_SELECT_CONFIRM";
                        var values = [enterButtonText];
                        notificationRoot.services.voiceControlService.addVoiceControlItem("Select", itemId, values);
                    }
                }
            }

            Text {
                id: hint
                objectName: "pinHint"
                width: parent.width
                text: root.hintText
                color: "@@inkDim@@"
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(22)
                horizontalAlignment: Text.AlignHCenter
                renderType: Text.NativeRendering
                elide: Text.ElideRight
            }
        }
    }

    // The d-pad moves over the pad as it is drawn: three keys wide, 1-9, then Clear, 0 and Back, then OK.
    function wireKeys() {
        var k = function (i) { return repeaterNumpad.itemAt(i) };
        for (var i = 0; i < root.digits; i++) {
            var key = k(i);
            if (i < 9) {
                key.KeyNavigation.left = (i % 3 > 0) ? k(i - 1) : null;
                key.KeyNavigation.right = (i % 3 < 2) ? k(i + 1) : null;
                key.KeyNavigation.up = (i >= 3) ? k(i - 3) : null;
                key.KeyNavigation.down = (i < 6) ? k(i + 3) : (i === 8 ? numPadButtonBack : k(9));
            }
        }
        var zero = k(9);
        zero.KeyNavigation.left = numPadButtonClear;
        zero.KeyNavigation.right = numPadButtonBack;
        zero.KeyNavigation.up = k(7);
        zero.KeyNavigation.down = numPadButtonConfirm;
        numPadButtonClear.KeyNavigation.right = zero;
        numPadButtonClear.KeyNavigation.up = k(6);
        numPadButtonClear.KeyNavigation.down = numPadButtonConfirm;
        numPadButtonBack.KeyNavigation.left = zero;
        numPadButtonBack.KeyNavigation.up = k(8);
        numPadButtonBack.KeyNavigation.down = numPadButtonConfirm;
        numPadButtonConfirm.KeyNavigation.up = zero;
    }

    function focusLastDigit() {
        var key = padArea.lastFocusItem ? padArea.lastFocusItem : repeaterNumpad.itemAt(4);
        if (key)
            key.forceActiveFocus();
    }

    Component.onCompleted: {
        notificationRoot.services.voiceControlService.dropVoiceControlItems();
        for (var i = 0; i < root.digits; ++i) {
            var itemId = "PINCODE_SELECT_" + i;
            var values = [String(i)];
            notificationRoot.services.voiceControlService.addVoiceControlItem("Select", itemId, values);
            repeaterNumpad.itemAt(i).clicked.connect(numpadClicked)
        }
        numPadButtonClear.clicked.connect(clearAll)
        numPadButtonBack.clicked.connect(backClicked)
        numPadButtonConfirm.clicked.connect(confirmClicked)
        wireKeys()

        if (!aboutToClose) {
            const before = defaultPassword_msg.arg(notificationRoot.services.defaultPin)
            const after = defaultPassword_msg.arg(notificationRoot.services.defaultPin.replace(/(.{1})/g,"$1 "))
            const replacedMsg = message ? message.replace(before, after) : ""
            notificationRoot.services.putTtsChain((title || "") + ", " + replacedMsg + " " + (root.state === "secure" ? secureModeText.text : (Utils.replaceNewLineTags(enterpincode_guide_msg, " "))), "title")
        }
    }

    property int inputFieldIndex: 0

    onInputFieldIndexChanged: {
        if (state === "secure" && inputFieldIndex === root.boxes)
            confirmClicked()
    }

    function getCurrentPasswordBox() {
        if (inputFieldIndex >= 0 && inputFieldIndex <= root.boxes - 1)
            return repeaterPasswordBox.itemAt(inputFieldIndex)
        else
            return null;
    }

    //return true : successfully inserted and return
    //return false : inputField is FULL
    function pushPasswordToInputField(strNumber) {
        var currentPasswordBox;

        currentPasswordBox = getCurrentPasswordBox();
        if (currentPasswordBox === null)
            return false;

        currentPasswordBox.text = strNumber;
        speakClickedPinCode(strNumber);

        ++inputFieldIndex;
        return true;
    }

    function numpadClicked(text) {
        pushPasswordToInputField(text);
    }

    function backClicked() {
        var currentPasswordBox;

        if (inputFieldIndex <= 0) {
            return
        }

        inputFieldIndex--;
        currentPasswordBox = getCurrentPasswordBox();
        if (currentPasswordBox) {
            currentPasswordBox.text = ""
        }

        notificationRoot.services.putTtsChain(backspace_msg + NLocaleService.emptyString, "focused");
    }

    function clearAll() {
        while (inputFieldIndex > 0)
            backClicked();
    }

    function handleBackKey() {
        //cancel secure mode arrow input
        if (arrowInput.inprogress) {
            arrowInput.cancel();
            return;
        }

        //boundery check
        if (inputFieldIndex <= 0)
        {
            inputFieldIndex = 0;
            if (state === "secure") {
                state = "normal";
                saveLockState();
            }
            else if (!isFirstUse) {
                exitClicked();
            }
            return
        }
        //backspace func
        backClicked();
    }

    function confirmClicked()
    {
        if (inputFieldIndex <= root.boxes - 1)
            return;

        // A fresh PIN each time: LG's file appended to whatever was left in `password`.
        password = "";
        for (var i = 0; i < root.boxes; ++i) {
            password += repeaterPasswordBox.itemAt(i).text;
        }

        postPassword();
    }

    function exitClicked()
    {
        postExitSignal();
    }

    function getPromptTitle(retry, promptType, rev_title)
    {

        var title = parental_title; // default

        if (rev_title !== undefined && !retry
             && (promptType === "parental" || promptType === "set_match"))
            return rev_title;

        if (promptType === "parental")
            title = retry ? retry_title : parental_title;
        else if (promptType === "set_match")
            title = retry ? retry_title : setmatch_title;
        else if (promptType === "set_newpin")
            title = retry ? notpermitted_title : setnewpin_title;
        else if (promptType === "set_verify")
            title = retry ? retry_verify_title : verify_title;

        return title;
    }

    function getPromptMessage(retry, promptType, rev_msg)
    {

        var msg = parental_msg; // default

        if (rev_msg !== undefined && !retry
             && (promptType === "parental" || promptType === "set_match"))
            return rev_msg;

        if (promptType === "parental")
            msg = retry ? retry_msg : parental_msg;
        else if (promptType == "set_match")
            msg = retry ? retry_msg : setmatch_msg + (!notificationRoot.services.supportPasswordSetting ? " " + defaultPassword_msg.arg(notificationRoot.services.defaultPin) : "")
        else if (promptType == "set_newpin")
            msg = retry ? notpermitted_msg : isFirstUse ? setnewpin_msg_firstuse : setnewpin_msg;
        else if (promptType == "set_verify")
            msg = retry ? retry_verify_msg : verify_msg;

        return msg;
    }

    function saveLockState() {
        notificationRoot.services.saveLockState(root.state === "secure");
    }

    function speakClickedPinCode(text) {
        /*
        According to UX_2016_webOS_Initial_Settings_Audio guidance_v1.0.1_160414.pdf,
        the audio guidance speaks "hidden" message for the number keys, and don't speak anything for others.
         */
        var hiddenPinCode = "1234567890";
        if (hiddenPinCode.indexOf(text) > -1)
            notificationRoot.services.speak(tts_hidden_msg);
    }

    Keys.onReleased: {
        switch(event.key)
        {
            case Qt.Key_0:
              numpadClicked("0");
              break;
            case Qt.Key_1:
              numpadClicked("1");
              break;
            case Qt.Key_2:
              numpadClicked("2");
              break;
            case Qt.Key_3:
              numpadClicked("3");
              break;
            case Qt.Key_4:
              numpadClicked("4");
              break;
            case Qt.Key_5:
              numpadClicked("5");
              break;
            case Qt.Key_6:
              numpadClicked("6");
              break;
            case Qt.Key_7:
              numpadClicked("7");
              break;
            case Qt.Key_8:
              numpadClicked("8");
              break;
            case Qt.Key_9:
              numpadClicked("9");
              break;
            case Qt.Key_Backspace:
            case WebOS.Key_webOS_Back:
              handleBackKey();
              break;
        }
    }

    state: (systemSettings.disableSecureMode || !systemSettings.secretMode) ? "normal" : "secure"
    states: [
        State {
            name: "normal"
            PropertyChanges { target: arrowpad; visible: false }
            PropertyChanges { target: secureModeText; visible: false }
            PropertyChanges { target: buttonsScope; visible: true; focus: true }
            PropertyChanges { target: hint; visible: true }
        },
        State {
            name: "secure"
            PropertyChanges { target: arrowpad; visible: true; focus: true }
            PropertyChanges { target: secureModeText; visible: true }
            PropertyChanges { target: buttonsScope; visible: false }
            PropertyChanges { target: hint; visible: false }
        }
    ]
}
