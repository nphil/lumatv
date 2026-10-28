// Stand-in pictures for the store pages: abstract gradients as SVG data URIs, so no third-party art is kept in the repository.
// <img data-art="banner:262:Label"> -> kind (banner | card | thumb | icon), hue, the word drawn on it.
(function () {
  function svg(kind, hue, label) {
    var w = kind === "banner" ? 960 : kind === "thumb" ? 640 : kind === "card" ? 400 : 256;
    var h = kind === "banner" ? 360 : kind === "thumb" ? 360 : kind === "card" ? 300 : 256;
    var h2 = (Number(hue) + 48) % 360;
    var big = kind === "icon" ? 120 : kind === "banner" ? 76 : kind === "card" ? 44 : 52;
    var text = kind === "icon" ? label.charAt(0) : label;
    var s = '<svg xmlns="http://www.w3.org/2000/svg" width="' + w + '" height="' + h + '" viewBox="0 0 ' + w + " " + h + '">' +
      '<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(' + hue + ',62%,46%)"/>' +
      '<stop offset="1" stop-color="hsl(' + h2 + ',70%,26%)"/></linearGradient></defs>' +
      '<rect width="100%" height="100%" fill="url(#g)"/>' +
      '<circle cx="' + w * 0.82 + '" cy="' + h * 0.2 + '" r="' + h * 0.34 + '" fill="rgba(255,255,255,.13)"/>' +
      '<text x="' + (kind === "icon" ? w / 2 : 36) + '" y="' + (kind === "icon" ? h * 0.66 : h * 0.62) + '" font-family="sans-serif" font-weight="700" font-size="' + big + '" fill="#fff"' +
      (kind === "icon" ? ' text-anchor="middle"' : "") + ">" + text.replace(/&/g, "&amp;") + "</text></svg>";
    return "data:image/svg+xml;charset=utf-8," + encodeURIComponent(s);
  }
  document.addEventListener("DOMContentLoaded", function () {
    var list = document.querySelectorAll("img[data-art]");
    for (var i = 0; i < list.length; i++) {
      var p = list[i].getAttribute("data-art").split(":");
      list[i].src = svg(p[0], p[1], p.slice(2).join(":"));
    }
  });
})();
