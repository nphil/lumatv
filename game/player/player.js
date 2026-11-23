/* LumaGame's player page (player.html), inside the frame app in mode "video": `?yt=<id>` plays a community video through
   YouTube's IFrame API, `?hls=<url>` a Steam trailer in an HTML5 <video>. The frame gives it keys as
   {lumaframe: "key", key: ok|left|right|play|pause} and hears its state as {lumaplayer: playing|paused|ended|refused}.
   Nothing is drawn over the player (YouTube's embed rules); the frame shows "not available here" itself. */
(function (root) {
  'use strict';
  var SEEK_S = 10;
  // YouTube's errors where the owner or the embed rules refuse this player: 101/150 not embeddable, 153 no referrer.
  var REFUSED = { 2: 1, 5: 1, 100: 1, 101: 1, 150: 1, 153: 1 };

  /** What a key does to a player that is `playing` or not: play, pause, or seek by seconds. */
  function action(key, playing) {
    if (key === 'ok') return playing ? { op: 'pause' } : { op: 'play' };
    if (key === 'play' || key === 'pause') return { op: key };
    if (key === 'left') return { op: 'seek', by: -SEEK_S };
    if (key === 'right') return { op: 'seek', by: SEEK_S };
    return null;
  }
  /** A YouTube player state (`YT.PlayerState`) as the frame hears it; null for the ones it ignores. */
  function ytState(s) {
    return s === 1 ? 'playing' : s === 2 ? 'paused' : s === 0 ? 'ended' : null;
  }
  function refused(code) { return !!REFUSED[code]; }

  if (typeof module === 'object' && module.exports) { module.exports = { action: action, ytState: ytState, refused: refused }; return; }

  var doc = root.document, q = new URLSearchParams(root.location.search);
  function tell(state) { if (root.parent !== root) root.parent.postMessage({ lumaplayer: state }, '*'); }
  var player = null;        // {playing(), play(), pause(), seek(by)}

  root.addEventListener('message', function (e) {
    var d = e.data;
    if (e.source !== root.parent || !d || d.lumaframe !== 'key' || !player) return;
    var a = action(d.key, player.playing());
    if (!a) return;
    if (a.op === 'seek') player.seek(a.by); else player[a.op]();
  });

  function youtube(id) {
    var box = doc.createElement('div'); box.id = 'player'; doc.body.appendChild(box);
    root.onYouTubeIframeAPIReady = function () {
      var yt = new root.YT.Player('player', {
        width: '100%', height: '100%', videoId: id,
        playerVars: { autoplay: 1, playsinline: 1, controls: 0, disablekb: 1, rel: 0, iv_load_policy: 3, fs: 0, origin: root.location.origin },
        events: {
          onReady: function () {
            player = {
              playing: function () { return yt.getPlayerState() === 1; },
              play: function () { yt.playVideo(); }, pause: function () { yt.pauseVideo(); },
              seek: function (by) { yt.seekTo(Math.max(0, yt.getCurrentTime() + by), true); }
            };
            yt.playVideo();
          },
          onStateChange: function (e) { var s = ytState(e.data); if (s) tell(s); },
          onError: function (e) { if (refused(e.data)) tell('refused'); }
        }
      });
    };
    var s = doc.createElement('script');
    s.src = 'https://www.youtube.com/iframe_api';
    s.onerror = function () { tell('refused'); };
    doc.head.appendChild(s);
  }

  function stream(url) {
    var v = doc.createElement('video');
    v.autoplay = true; v.playsInline = true; v.src = url;
    v.addEventListener('playing', function () { tell('playing'); });
    v.addEventListener('pause', function () { if (!v.ended) tell('paused'); });
    v.addEventListener('ended', function () { tell('ended'); });
    v.addEventListener('error', function () { tell('refused'); });
    doc.body.appendChild(v);
    player = {
      playing: function () { return !v.paused && !v.ended; },
      play: function () { v.play(); }, pause: function () { v.pause(); },
      seek: function (by) { v.currentTime = Math.max(0, v.currentTime + by); }
    };
  }

  if (q.get('yt')) youtube(q.get('yt'));
  else if (q.get('hls')) stream(q.get('hls'));
  else tell('refused');
})(this);
