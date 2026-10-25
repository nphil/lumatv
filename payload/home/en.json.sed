# Luma: rewrite of the stock Home i18n strings (com.webos.app.home assets/i18n/en.json).
# Applied by the CLI with:  sed -f en.json.sed < stock/en.json > assets/i18n/en.json
# Both rules match on the JSON key and replace whatever value is present, so the
# script is idempotent and also re-targets a file that already carries an edit.
#
# Hero headline -> blank. The greeting is deliberately NOT substituted here: the
# compositor QML computes it itself (homeWidgets.greet) and draws it inside the
# clock card. Putting it in LG's hero string as well paints a second, stray
# greeting in the hero's right-hand column, directly over the weather card.
s/\("Start a new experience with webOS\."\): "[^"]*"/\1: ""/
# Hero call-to-action under the headline -> blank (the shelf it pointed at is gone).
s/\("Go to Apps"\): "[^"]*"/\1: ""/
