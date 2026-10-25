/* Luma Frame: the web half of a mini app. LumaHome draws the glass frame and launches
   this overlay-type app with {url, title, source, rect:{x,y,w,h}, light}. The interior at
   rect shows the page in an iframe when the site allows framing, otherwise a reader view
   built from the fetched HTML. Up/Down scroll, Back closes (the compositor closes overlay
   windows on Back; window.close() covers the app's own handling). */
(function () {
  'use strict';
  var pane = document.getElementById('pane');
  var spinner = document.getElementById('spinner');
  var reader = document.getElementById('reader');
  var web = document.getElementById('web');
  var errorBox = document.getElementById('error');
  var scrollY = 0, inner = null, current = null;

  function exec(cmd) {
    return new Promise(function (resolve, reject) {
      if (typeof PalmServiceBridge === 'function') {
        var bridge = new PalmServiceBridge();
        bridge.onservicecallback = function (msg) {
          try { resolve(JSON.parse(msg)); } catch (e) { reject(new Error('exec: invalid response')); }
        };
        bridge.call('palm://org.webosbrew.hbchannel.service/exec', JSON.stringify({ command: cmd }));
        return;
      }
      if (window.webOS && webOS.service && webOS.service.request) {
        webOS.service.request('luna://org.webosbrew.hbchannel.service', {
          method: 'exec', parameters: { command: cmd },
          onSuccess: function (r) { resolve(r); },
          onFailure: function (e) { reject(new Error(e && e.errorText || 'exec failed')); }
        });
        return;
      }
      reject(new Error('no service bridge'));
    });
  }
  function shq(s) { return "'" + String(s).replace(/'/g, "'\\''") + "'"; }

  function layout(p) {
    var r = p.rect || { x: 200, y: 150, w: 1520, h: 860 };
    pane.style.left = r.x + 'px'; pane.style.top = r.y + 'px'; pane.style.width = r.w + 'px'; pane.style.height = r.h + 'px';
    document.body.classList.toggle('light', !!p.light);
  }
  function show(el) {
    [spinner, reader, web, errorBox].forEach(function (e) { e.hidden = (e !== el); });
    if (el === spinner) spinner.style.display = 'flex'; else spinner.style.display = 'none';
  }
  function fail(msg) { errorBox.textContent = msg; show(errorBox); }

  // ---- reader: pick the largest text container, keep headings, paragraphs, lists, figures
  function textLen(el) { return (el.textContent || '').replace(/\s+/g, ' ').trim().length; }
  function pickImage(el) {
    // largest candidate from <picture><source srcset>, <img srcset> or <img src>
    var best = '', bw = -1;
    function consider(srcset, src) {
      if (srcset) srcset.split(',').forEach(function (s) {
        var parts = s.trim().split(/\s+/), w = parseInt((parts[1] || '0'), 10) || 0;
        if (/^https?:/.test(parts[0]) && w > bw) { bw = w; best = parts[0]; }
      });
      if (src && /^https?:/.test(src) && bw < 0) { bw = 0; best = src; }
    }
    el.querySelectorAll('source').forEach(function (so) { consider(so.getAttribute('srcset') || so.getAttribute('data-srcset'), null); });
    var img = el.tagName === 'IMG' ? el : el.querySelector('img');
    if (img) consider(img.getAttribute('srcset') || img.getAttribute('data-srcset'), img.getAttribute('src') || img.getAttribute('data-src'));
    return best;
  }
  function junk(t) {
    return /you are now following|updates from your news topics|^published|^subsection|^related topics$|^share$|^more on this story|^follow /i.test(t);
  }
  function buildReader(html, p) {
    var doc = new DOMParser().parseFromString(html, 'text/html');
    doc.querySelectorAll('script,style,noscript,nav,header,footer,aside,form,iframe,svg,button,[role=navigation],[aria-hidden=true]').forEach(function (e) { e.remove(); });
    var candidates = Array.prototype.slice.call(doc.querySelectorAll('article,main,[role=main],.article,.story,#content,body'));
    var best = candidates.sort(function (a, b) { return textLen(b) - textLen(a); })[0] || doc.body;
    var frag = document.createDocumentFragment();
    var h1 = doc.querySelector('h1');
    var title = document.createElement('h1'); title.textContent = p.title || (h1 && h1.textContent.trim()) || ''; frag.appendChild(title);
    var seenText = {}, seenImg = {}, stop = false;
    best.querySelectorAll('p,h2,h3,ul,ol,figure,picture,img').forEach(function (el) {
      if (stop) return;
      if (el.closest('figure,picture') && !/^(FIGURE|PICTURE)$/.test(el.tagName)) return;
      if (/^(FIGURE|PICTURE|IMG)$/.test(el.tagName)) {
        var src = pickImage(el);
        if (!src || seenImg[src]) return;
        seenImg[src] = true;
        var fig = document.createElement('figure'); var im = document.createElement('img'); im.src = src; fig.appendChild(im);
        var cap = el.querySelector && el.querySelector('figcaption');
        if (cap) { var c = document.createElement('figcaption'); c.textContent = cap.textContent.replace(/^image caption,?\s*/i, '').trim(); fig.appendChild(c); }
        frag.appendChild(fig); return;
      }
      var t = el.textContent.replace(/\s+/g, ' ').trim();
      if (t.length < 2 || seenText[t] || junk(t)) return;
      if (/^h[23]$/i.test(el.tagName) && /^(related|more on|explore more|latest|top stories|most (read|watched|popular))/i.test(t)) { stop = true; return; }
      seenText[t] = true;
      if (/^(ul|ol)$/i.test(el.tagName)) {
        var items = Array.prototype.slice.call(el.querySelectorAll('li')).filter(function (li) { return !li.querySelector('a') && !junk(li.textContent); });
        if (!items.length) return;
        var list = document.createElement(el.tagName.toLowerCase());
        items.forEach(function (li) { var l = document.createElement('li'); l.textContent = li.textContent.replace(/\s+/g, ' ').trim(); list.appendChild(l); });
        frag.appendChild(list); return;
      }
      if (el.tagName === 'P' && t.length < 24 && !/[.!?]$/.test(t)) return;   // labels, bylines, timestamps
      var out = document.createElement(el.tagName.toLowerCase() === 'h3' ? 'h2' : el.tagName.toLowerCase());
      out.textContent = t;
      frag.appendChild(out);
    });
    reader.innerHTML = '';
    inner = document.createElement('div'); inner.className = 'inner'; inner.appendChild(frag); reader.appendChild(inner);
    scrollY = 0; inner.style.transform = 'translateY(0)';
  }
  function scrollBy(d) {
    if (!inner) return;
    var max = Math.max(0, inner.scrollHeight - reader.clientHeight + 40);
    scrollY = Math.max(0, Math.min(max, scrollY + d));
    inner.style.transform = 'translateY(' + (-scrollY) + 'px)';
  }

  // ---- open: decide iframe vs reader from the response headers, then load
  function open(p) {
    current = p; layout(p); show(spinner);
    if (!p.url) { fail('Nothing to show'); return; }
    if (p.mode === 'web') { openWeb(p.url); return; }
    exec('curl -sSL -m 12 -A "Mozilla/5.0 (X11; Linux) AppleWebKit/537.36 Chrome/94 Safari/537.36" -D - -o /tmp/lumaframe.html ' + shq(p.url) + ' >/tmp/lumaframe.hdr 2>&1; cat /tmp/lumaframe.hdr')
      .then(function (r) {
        var hdr = (r.stdoutString || '').toLowerCase();
        var framable = !/x-frame-options:\s*(deny|sameorigin)/.test(hdr) && !/frame-ancestors\s+('none'|'self')/.test(hdr);
        if (p.mode !== 'reader' && framable) { openWeb(p.url); return; }
        return exec('cat /tmp/lumaframe.html').then(function (rr) {
          if (!rr.stdoutString) { fail('Could not load the article'); return; }
          buildReader(rr.stdoutString, p); show(reader);
        });
      })
      .catch(function (e) { fail('Could not load: ' + e.message); });
  }
  function openWeb(url) {
    web.onload = function () { show(web); };
    web.src = url;
    setTimeout(function () { if (spinner.style.display !== 'none') show(web); }, 6000);
  }

  function params() {
    try { if (window.webOSSystem && webOSSystem.launchParams) return JSON.parse(webOSSystem.launchParams); } catch (e) {}
    try { if (window.PalmSystem && PalmSystem.launchParams) return JSON.parse(PalmSystem.launchParams); } catch (e) {}
    return {};
  }
  document.addEventListener('webOSRelaunch', function (e) { open((e && e.detail) || params()); });
  window.addEventListener('keydown', function (e) {
    var k = e.keyCode;
    if (k === 40) { scrollBy(300); e.preventDefault(); }
    else if (k === 38) { scrollBy(-300); e.preventDefault(); }
    else if (k === 461 || k === 27) { e.preventDefault(); window.close(); }
  });
  document.body.tabIndex = 0; document.body.focus();
  open(params());
})();
