// The stand-in pages' icon sprite (Lucide, ISC: payload/compositor/lumatv/icons/lucide/LICENSE), injected as <symbol>s for <use href="#i-name">.
(function () {
  var paths = {
    search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
    user: '<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 4-7 8-7s8 3 8 7"/>',
    more: '<circle cx="12" cy="5" r="1.4"/><circle cx="12" cy="12" r="1.4"/><circle cx="12" cy="19" r="1.4"/>',
    x: '<path d="M6 6l12 12M18 6L6 18"/>',
    download: '<path d="M12 4v11"/><path d="m7 11 5 5 5-5"/><path d="M5 20h14"/>',
    left: '<path d="m15 5-7 7 7 7"/>',
    right: '<path d="m9 5 7 7-7 7"/>',
    back: '<path d="M19 12H5"/><path d="m11 6-6 6 6 6"/>',
    share: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><path d="m8.6 13.5 6.8 4M15.4 6.5l-6.8 4"/>'
  };
  var s = '<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0" style="position:absolute" aria-hidden="true">';
  for (var k in paths) s += '<symbol id="i-' + k + '" viewBox="0 0 24 24">' + paths[k] + "</symbol>";
  s += "</svg>";
  document.addEventListener("DOMContentLoaded", function () { document.body.insertAdjacentHTML("afterbegin", s); });
})();
