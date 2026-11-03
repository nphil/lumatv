"""Additional LG notification view scenes."""
from scenes import scene
# ---- the notification view: LG's standard toast and alert in the Luma look, and the input popup with its port said
from scenes_notify import render, ALERT, INPUT  # noqa: E402  (loaded before this file: scenes_*.py in name order)

# What LG's own handlers send for a key that does nothing here ("not available") and for a message of three lines: no icon, no button.
STANDARD_TOAST = {"sourceId": "com.webos.app.home", "type": "standard", "timeout": 30, "iconPath": "", "readable": "", "userData": {"id": "scene"},
                  "message": "This function is not available."}
STANDARD_TOAST_LINES = dict(STANDARD_TOAST, message="Wi-Fi connection lost<br>The TV is trying to join Nate's Home again. Check that the router is on.")
# A plain question with no title and two buttons, the first focused: LG's alert for a setting that needs a confirmation.
STANDARD_ALERT = {"alertInfo": {"sourceId": "com.webos.app.home", "title": "", "message": "Turn off the TV now?", "readable": "", "showTimeout": False, "modal": False,
                                "timeout": 0, "timestamp": 21, "buttons": [{"label": "Cancel", "focus": True, "params": {}}, {"label": "Turn off", "params": {}}]}}


@scene("07-toast-standard", runner=True)
def _(out_dir, material):
    return render("07-toast-standard", out_dir, material, [("toast", 300, STANDARD_TOAST)])


@scene("07-toast-standard-lines", runner=True)
def _(out_dir, material):
    return render("07-toast-standard-lines", out_dir, material, [("toast", 300, STANDARD_TOAST_LINES)])


@scene("06-alert-standard", runner=True)
def _(out_dir, material):
    return render("06-alert-standard", out_dir, material, [("alert", 300, STANDARD_ALERT)])


@scene("18-device-connected-port", runner=True)
def _(out_dir, material):
    return render("18-device-connected-port", out_dir, material, [("input", 300, INPUT)])


