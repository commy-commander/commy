(function (root) {
  'use strict';
  var doc = root.document;
  var FACE = 'M43.78 4.25A2.4 2.4 0 0 1 45.98 2.8L54.02 2.8A2.4 2.4 0 0 1 56.22 4.25L64.5 23.58A5 5 0 0 0 68.65 26.59L89.59 28.49A2.4 2.4 0 0 1 91.65 30.14L94.13 37.78A2.4 2.4 0 0 1 93.43 40.33L77.61 54.18A5 5 0 0 0 76.03 59.05L80.69 79.55A2.4 2.4 0 0 1 79.76 82.03L73.26 86.75A2.4 2.4 0 0 1 70.62 86.87L52.56 76.1A5 5 0 0 0 47.44 76.1L29.38 86.87A2.4 2.4 0 0 1 26.74 86.75L20.24 82.03A2.4 2.4 0 0 1 19.31 79.55L23.97 59.05A5 5 0 0 0 22.39 54.18L6.57 40.33A2.4 2.4 0 0 1 5.87 37.78L8.35 30.14A2.4 2.4 0 0 1 10.41 28.49L31.35 26.59A5 5 0 0 0 35.5 23.58Z';
  var TICK = 'M41.6 51.62l5.88 5.88 11.13 -11.55';
  var G = {"y":51.2,"r":3.9,"p":11.8,"lit":1.3,"d":10,"n":6};
  var STATES = ['idle', 'near', 'press', 'sent', 'waiting', 'quiet', 'unimpressed', 'surprised', 'working', 'off'];
  var MOODS = { level: 'idle', wink: 'sent', wide: 'surprised', side: 'unimpressed' };
  var SENT_MS = 1500;
  var SMALL = 28;
  var live = [], px = -1e5, py = -1e5, raf = 0, listening = false;

  function n2(v) { return Math.round(v * 100) / 100; }
  function art() {
    var s = '<svg class="star-art" viewBox="0 0 100 100" aria-hidden="true" focusable="false"><g class="star-hop"><g class="star-wall">', i;
    for (i = 1; i <= G.n; i++) s += '<path d="' + FACE + '" style="--k:' + n2(G.d * i / G.n) + '"/>';
    s += '</g><g class="star-cap"><path class="star-lit" d="' + FACE + '"/><path class="star-top" d="' + FACE + '" transform="translate(0 ' + G.lit + ')"/>';
    s += '<g class="star-legend" style="transform-origin:50px ' + G.y + 'px"><g class="star-dots" style="transform-origin:50px ' + G.y + 'px">';
    for (i = -1; i <= 1; i++) {
      var box = ' x="' + n2(50 + i * G.p - G.r) + '" y="' + n2(G.y - G.r) + '" width="' + n2(G.r * 2) + '" height="' + n2(G.r * 2) + '" rx="' + G.r + '"/>';
      s += '<g class="star-dot star-d' + (i + 2) + '"><rect class="star-ink"' + box + '<rect class="star-wait"' + box + '</g>';
    }
    var dw = G.p * 2 + G.r * 2 - 1.6, bw = G.r * 1.9, bh = G.r * 4.2;
    s += '</g><rect class="star-dash" x="' + n2(50 - dw / 2) + '" y="' + n2(G.y - 2) + '" width="' + n2(dw) + '" height="4" rx="2"/>';
    s += '<rect class="star-bang" x="' + n2(50 - bw / 2) + '" y="' + n2(G.y - G.r * 3.6) + '" width="' + n2(bw) + '" height="' + n2(bh) + '" rx="' + n2(bw / 2) + '"/>';
    s += '<path class="star-tick" d="' + TICK + '"/></g></g></g></svg>';
    return s;
  }

  function still() { return !!(root.matchMedia && root.matchMedia('(prefers-reduced-motion: reduce)').matches); }
  function aim() {
    raf = 0;
    for (var i = 0; i < live.length; i++) {
      var s = live[i];
      if (s.slot || s.held || s.el.getAttribute('data-follow') === 'off') continue;
      var st = s.el.getAttribute('data-state');
      if (st !== 'idle' && st !== 'waiting') { if (s.near) release(s); continue; }
      var r = s.el.getBoundingClientRect();
      if (!r.width) continue;
      var dx = px - (r.left + r.width / 2), dy = py - (r.top + r.height / 2), dist = Math.sqrt(dx * dx + dy * dy) || 1;
      if (dist < Math.max(r.width * 2.4, 96)) {
        var k = Math.min(1, dist / (r.width * .7));
        s.el.style.setProperty('--look-x', n2(dx / dist * k)); s.el.style.setProperty('--look-y', n2(dy / dist * k));
        if (!s.near) { s.near = true; s.el.setAttribute('data-near', ''); }
      } else if (s.near) release(s);
    }
  }
  function release(s) { s.near = false; s.el.removeAttribute('data-near'); s.el.style.removeProperty('--look-x'); s.el.style.removeProperty('--look-y'); }
  function listen() {
    return;
    root.addEventListener('pointermove', function (e) { px = e.clientX; py = e.clientY; if (!raf) raf = root.requestAnimationFrame(aim); }, { passive: true });
    doc.documentElement.addEventListener('pointerleave', function () { px = py = -1e5; if (!raf) raf = root.requestAnimationFrame(aim); });
  }

  function mount(target) {
    var el = typeof target === 'string' ? doc.querySelector(target) : target;
    if (!el) return null;
    if (el.__star) return el.__star;
    var slot = null, host = el;
    if (el.classList.contains('mascot-slot')) {
      slot = el; host = doc.createElement('span'); host.className = 'star mascot-art'; slot.appendChild(host);
    } else if (el.parentElement && el.parentElement.closest('.mascot-slot')) slot = el.parentElement.closest('.mascot-slot');
    host.classList.add('star');
    host.innerHTML = art();
    if (slot || host.closest('a, button, [role="button"], [aria-hidden="true"]')) host.setAttribute('aria-hidden', 'true');
    else if (!host.hasAttribute('aria-label') && !host.hasAttribute('aria-hidden')) { host.setAttribute('role', 'img'); host.setAttribute('aria-label', 'Commy'); }
    if (!host.getAttribute('data-state')) host.setAttribute('data-state', 'idle');

    var s = { el: host, slot: slot, near: false, held: false, lock: false, base: 'idle', timer: 0, obs: [] };
    var cur = host.getAttribute('data-state');
    if (cur !== 'sent' && cur !== 'press') s.base = cur;
    function back(ms, to) { root.clearTimeout(s.timer); s.timer = root.setTimeout(function () { s.timer = 0; s.lock = false; host.setAttribute('data-state', to); }, ms); }
    function changed() {
      var st = host.getAttribute('data-state') || 'idle';
      if (STATES.indexOf(st) < 0) { host.setAttribute('data-state', 'idle'); return; }
      if (s.lock) return;
      if (st === 'sent') { if (!slot && !host.hasAttribute('data-hold')) back(SENT_MS, s.base); }
      else if (st !== 'press') { s.base = st; root.clearTimeout(s.timer); }
    }
    function mood() { var m = MOODS[slot.getAttribute('data-mood')] || 'idle'; if (host.getAttribute('data-state') !== m) host.setAttribute('data-state', m); }
    if (root.MutationObserver) {
      var o = new root.MutationObserver(changed); o.observe(host, { attributes: true, attributeFilter: ['data-state'] }); s.obs.push(o);
      if (slot) { var o2 = new root.MutationObserver(mood); o2.observe(slot, { attributes: true, attributeFilter: ['data-mood'] }); s.obs.push(o2); }
    }
    if (slot) mood();
    changed();
    if (root.ResizeObserver) {
      var ro = new root.ResizeObserver(function (en) { var w = en[0].contentRect.width; if (w) host.classList.toggle('is-small', w < SMALL); });
      ro.observe(host); s.obs.push(ro);
    } else host.classList.toggle('is-small', host.getBoundingClientRect().width < SMALL);

    var api = {
      el: host,
      get state() { return host.getAttribute('data-state'); },

      set: function (state, ms) {
        if (STATES.indexOf(state) < 0) return api;
        root.clearTimeout(s.timer); s.lock = !!ms;
        if (ms) back(ms, s.base === state ? 'idle' : s.base);
        host.setAttribute('data-state', state);
        return api;
      },
      sent: function () { return api.set('sent', SENT_MS); },
      press: function (ms) { return api.set('press', ms || 180); },

      look: function (x, y) {
        if (x == null) { s.held = false; release(s); return api; }
        s.held = true; host.setAttribute('data-near', ''); host.style.setProperty('--look-x', n2(x)); host.style.setProperty('--look-y', n2(y || 0)); return api;
      },
      unmount: function () {
        root.clearTimeout(s.timer); s.obs.forEach(function (o) { o.disconnect(); });
        var i = live.indexOf(s); if (i >= 0) live.splice(i, 1);
        release(s); host.innerHTML = ''; delete el.__star; if (slot === el && host.parentNode === el) el.removeChild(host);
      }
    };
    el.__star = api; live.push(s); listen();
    return api;
  }
  function mountAll(scope) {
    var list = (scope || doc).querySelectorAll('.star, .mascot-slot'), out = [];
    for (var i = 0; i < list.length; i++) if (!list[i].__star && !list[i].classList.contains('mascot-art')) out.push(mount(list[i]));
    return out;
  }
  function set(el, state, ms) { var a = mount(el); return a && a.set(state, ms); }

  root.Star = { version: '1.0', states: STATES.slice(), mount: mount, mountAll: mountAll, set: set, svg: art };
  var me = doc.currentScript;
  if (me && me.hasAttribute('data-auto')) {
    if (doc.readyState === 'loading') doc.addEventListener('DOMContentLoaded', function () { mountAll(); }); else mountAll();
  }
})(window);
