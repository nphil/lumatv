# opencard.sh SECONDS_FROM_NOW : open the card in LG's browser as a data: URL whose clock starts in N s; prints T0 (epoch s)
N=$1; T0=$(( $(date +%s) + N ))
{ printf '<script>window.T0=%s000;</script>' $T0; cat /tmp/sharpness-card.html; } | base64 | tr -d '\n' > /tmp/card.b64
luna-send -n 1 -f luna://com.webos.service.applicationmanager/launch "{\"id\":\"com.webos.app.browser\",\"params\":{\"target\":\"data:text/html;base64,$(cat /tmp/card.b64)\"}}" > /dev/null
echo $T0
