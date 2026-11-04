/* VizardCraft published report viewer. No dependencies.
 * Data: window.VIZARDCRAFT_REPORT (single file) or report.json.
 * Engine: vizardcraft_viewer.wasm (or window.VIZARDCRAFT_WASM_B64) — the VizardCraft engine compiled to
 * WebAssembly. Without it the report shows the pre-rendered pages (static mode). */
(function () {
  'use strict';
  var $ = function (id) { return document.getElementById(id); };
  var qs = new URLSearchParams(location.search);
  var S = {
    data: null, eng: null, page: 0, layout: null, key: null, view: null,
    mode: 'page', story: null, storyIdx: 0, back: [], paneOpen: false
  };
  var ICON = {
    filter: '<svg viewBox="0 0 20 20"><path d="M3.5 4.5h13l-5 6v5l-3 1.5v-6.5z"/></svg>',
    bookmark: '<svg viewBox="0 0 20 20"><path d="M6 3.5h8v13l-4-3-4 3z"/></svg>',
    page: '<svg viewBox="0 0 20 20"><rect x="3.5" y="3.5" width="13" height="13" rx="2"/><path d="M6.5 13v-3M10 13V7M13.5 13v-5"/></svg>',
    story: '<svg viewBox="0 0 20 20"><rect x="2.5" y="5" width="15" height="10" rx="1.5"/><path d="m8.5 8 3.5 2-3.5 2z"/></svg>',
    up: '<svg viewBox="0 0 16 16"><path d="M8 13V3M4 7l4-4 4 4"/></svg>',
    down: '<svg viewBox="0 0 16 16"><path d="M8 3v10M4 9l4 4 4-4"/></svg>',
    expand: '<svg viewBox="0 0 16 16"><path d="M4 3l4 4 4-4M4 9l4 4 4-4"/></svg>',
    mode: '<svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="6"/><path d="M8 5v6M5.5 8.5 8 11l2.5-2.5"/></svg>',
    focus: '<svg viewBox="0 0 16 16"><path d="M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3"/></svg>',
    prev: '<svg viewBox="0 0 20 20"><path d="M12.5 4.5 7 10l5.5 5.5"/></svg>',
    next: '<svg viewBox="0 0 20 20"><path d="m7.5 4.5 5.5 5.5-5.5 5.5"/></svg>'
  };

  // ------------------------------------------------------------------ engine (WASM)
  function Engine(ex) {
    this.ex = ex;
    this.enc = new TextEncoder();
    this.dec = new TextDecoder();
  }
  Engine.prototype.call = function (fn, arg) {
    var ex = this.ex;
    var bytes = this.enc.encode(JSON.stringify(arg === undefined ? {} : arg));
    var p = ex.alloc(bytes.length);
    new Uint8Array(ex.memory.buffer, p, bytes.length).set(bytes);
    var rp = ex[fn](p, bytes.length);
    ex.free(p, bytes.length);
    var len = ex.result_len();
    var out = JSON.parse(this.dec.decode(new Uint8Array(ex.memory.buffer, rp, len)));
    if (out && out.error) throw new Error(out.error);
    return out;
  };

  function b64bytes(s) {
    var bin = atob(s);
    var a = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) a[i] = bin.charCodeAt(i);
    return a;
  }

  function loadData() {
    if (window.VIZARDCRAFT_REPORT) return Promise.resolve(window.VIZARDCRAFT_REPORT);
    return fetch('report.json').then(function (r) {
      if (!r.ok) throw new Error('report.json: ' + r.status);
      return r.json();
    });
  }

  function loadEngine(d) {
    if (!d.interactive || !d.project || typeof WebAssembly === 'undefined') return Promise.resolve(null);
    var src = window.VIZARDCRAFT_WASM_B64
      ? Promise.resolve(b64bytes(window.VIZARDCRAFT_WASM_B64))
      : fetch('vizardcraft_viewer.wasm').then(function (r) { if (!r.ok) throw new Error('engine: ' + r.status); return r.arrayBuffer(); });
    return src.then(function (bytes) { return WebAssembly.instantiate(bytes, {}); }).then(function (res) {
      var e = new Engine(res.instance.exports);
      e.call('load', { project: d.project, tables: d.tables, today: d.today });
      return e;
    });
  }

  // ------------------------------------------------------------------ helpers
  function esc(s) { return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); }
  function isMobile() { return S.layout === 'mobile'; }
  function tabPages() { return S.data.pages.filter(function (p) { return !p.hidden; }); }
  function pageInfo(id) { return S.data.pages.filter(function (p) { return p.id === id; })[0]; }
  function fmtDate(iso) {
    if (!iso) return '';
    var d = new Date(iso);
    if (isNaN(d)) return '';
    return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }) + ', ' + d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  }

  // ------------------------------------------------------------------ views
  function staticView(key) {
    var v = S.data.views[key];
    if (!v) return null;
    return (isMobile() && v.mobile) ? v.mobile : v.desktop;
  }

  function current() {
    if (S.eng) {
      try { return S.eng.call('render_page', { page: S.page, mobile: isMobile() }); } catch (e) { console.error(e); }
    }
    return staticView(S.key || ('page:' + S.page)) || staticView('page:' + S.page);
  }

  function refresh() {
    var v = current();
    if (!v) return;
    S.view = v;
    S.page = v.page;
    draw();
    chrome();
  }

  function outcome(o) {
    if (!o) return;
    if (o.url) { window.open(o.url, '_blank', 'noopener'); }
    if (o.page) S.page = o.page;
    if (o.message) console.warn(o.message);
    if (o.changed || o.page) refresh();
  }

  // ------------------------------------------------------------------ drawing
  function draw() {
    var v = S.view, page = $('vc-page'), stage = $('vc-stage');
    page.style.width = v.width + 'px';
    page.style.height = v.height + 'px';
    page.style.background = v.background;
    stage.style.background = v.wallpaper;
    document.getElementById('vc-app').classList.toggle('vc-mobile', !!v.mobile);
    var html = [];
    v.elements.forEach(function (e) {
      var cls = 'vc-el' + (e.action ? ' vc-clickable' : '');
      var st = 'left:' + e.x + 'px;top:' + e.y + 'px;width:' + e.w + 'px;height:' + e.h + 'px' + (e.rot ? ';transform:rotate(' + e.rot + 'deg)' : '');
      var attrs = ' data-id="' + e.id + '" data-kind="' + e.kind + '"' + (e.action ? ' data-action="' + esc(JSON.stringify(e.action)) + '"' : '');
      var head = '';
      if (e.kind === 'visual' && e.header) {
        head = '<div class="vc-vh">';
        if (e.drillable && S.eng) {
          head += '<button data-cmd="up" title="Drill up">' + ICON.up + '</button>' +
            '<button data-cmd="mode" title="Drill down on click"' + (e.drillMode ? ' class="vc-on"' : '') + '>' + ICON.mode + '</button>' +
            '<button data-cmd="down" title="Go to the next level">' + ICON.down + '</button>' +
            '<button data-cmd="expand" title="Expand all down one level">' + ICON.expand + '</button>';
        }
        head += '<button data-cmd="focus" title="Focus mode">' + ICON.focus + '</button></div>';
      }
      html.push('<div class="' + cls + '" style="' + st + '"' + attrs + '>' + e.svg + head + '</div>');
    });
    page.innerHTML = html.join('');
    fit();
  }

  function fit() {
    var v = S.view;
    if (!v) return;
    var stage = $('vc-stage'), wrap = $('vc-wrap'), page = $('vc-page');
    var story = $('vc-story');
    var storyH = story.hidden ? 0 : story.offsetHeight;
    var narrow = stage.clientWidth < 520;
    var pad = v.mobile ? (narrow ? 8 : 20) : (narrow ? 8 : 24);
    var aw = Math.max(stage.clientWidth - pad * 2, 100);
    var ah = Math.max(stage.clientHeight - pad * 2 - storyH, 100);
    var s;
    if (v.mobile) s = Math.min(aw / v.width, narrow ? 4 : 1.2);
    else if (v.viewMode === 'Fit to width') s = aw / v.width;
    else if (v.viewMode === 'Actual size') s = 1;
    else s = Math.min(aw / v.width, ah / v.height);
    s = Math.max(s, 0.15);
    page.style.transform = 'scale(' + s + ')';
    wrap.style.width = Math.floor(v.width * s) + 'px';
    wrap.style.height = Math.floor(v.height * s) + 'px';
    wrap.style.margin = pad + 'px auto';
    story.style.width = Math.max(Math.floor(v.width * s), 320) + 'px';
  }

  // ------------------------------------------------------------------ chrome: tabs, buttons, pane
  function chrome() {
    var d = S.data, v = S.view;
    // Tabs.
    var tabs = [];
    tabPages().forEach(function (p) {
      var on = S.mode === 'page' && p.id === S.page;
      tabs.push('<button class="vc-tab' + (on ? ' vc-on' : '') + '" data-page="' + p.id + '">' + ICON.page + esc(p.name) + '</button>');
    });
    if (d.stories.length) {
      tabs.push('<span class="vc-tab-sep"></span>');
      d.stories.forEach(function (s) {
        var on = S.mode === 'story' && S.story === s.id;
        tabs.push('<button class="vc-tab' + (on ? ' vc-on' : '') + '" data-story="' + s.id + '">' + ICON.story + esc(s.name || 'Story') + '</button>');
      });
    }
    $('vc-tab-list').innerHTML = tabs.join('');
    $('vc-tabs').hidden = tabs.length < 2 && !d.stories.length && !S.embed;
    // Story navigator.
    var sb = $('vc-story');
    if (S.mode === 'story') {
      var st = d.stories.filter(function (s) { return s.id === S.story; })[0];
      if (st) {
        var pts = st.points.map(function (pt, i) {
          return '<button class="vc-story-point' + (i === S.storyIdx ? ' vc-on' : '') + '" data-point="' + i + '"><b>' + (i + 1) + ' / ' + st.points.length + '</b>' + esc(pt.caption || ('Point ' + (i + 1))) + '</button>';
        }).join('');
        sb.innerHTML = (st.title ? '<h2 class="vc-story-title">' + esc(st.title) + '</h2>' : '') +
          '<div class="vc-story-nav"><button class="vc-story-arrow" data-step="-1"' + (S.storyIdx === 0 ? ' disabled' : '') + ' title="Previous">' + ICON.prev + '</button>' +
          '<div class="vc-story-points">' + pts + '</div>' +
          '<button class="vc-story-arrow" data-step="1"' + (S.storyIdx >= st.points.length - 1 ? ' disabled' : '') + ' title="Next">' + ICON.next + '</button></div>';
        sb.hidden = false;
      }
    } else {
      sb.hidden = true;
    }
    // Header.
    var pi = pageInfo(S.page);
    $('vc-sub').textContent = [S.mode === 'story' ? 'Story' : (pi ? pi.name : v.name), d.published ? 'Published ' + fmtDate(d.published) : ''].filter(Boolean).join('  ·  ');
    document.title = d.name + (pi ? ' · ' + pi.name : '');
    $('vc-back').hidden = !(S.eng ? v.canBack : S.back.length);
    var segs = $('vc-layout').querySelectorAll('button');
    for (var i = 0; i < segs.length; i++) segs[i].classList.toggle('vc-on', segs[i].getAttribute('data-layout') === (isMobile() ? 'mobile' : 'desktop'));
    $('vc-layout').hidden = !d.includeMobile;
    var n = v.filters.length;
    $('vc-filter-count').textContent = n;
    $('vc-filter-count').hidden = n === 0;
    $('vc-filters-btn').classList.toggle('vc-on', S.paneOpen);
    $('vc-bm-btn').hidden = !d.bookmarks.length;
    $('vc-mode').textContent = S.eng ? 'Interactive report' : (d.interactive ? 'Static view (the interactive engine could not load)' : 'Static view');
    pane();
    fit();
    var h = '#page=' + S.page + (S.mode === 'story' ? '&story=' + S.story + '&point=' + S.storyIdx : '');
    if (location.hash !== h) history.replaceState(null, '', h);
  }

  function pane() {
    var p = $('vc-pane');
    p.hidden = !S.paneOpen;
    if (!S.paneOpen) return;
    var v = S.view, d = S.data, out = [];
    out.push('<div class="vc-pane-section"><h3>On this page</h3>');
    if (v.filters.length) {
      v.filters.forEach(function (f) { out.push('<div class="vc-filter">' + ICON.filter + '<span>' + esc(f) + '</span></div>'); });
    } else {
      out.push('<div class="vc-empty">No filters are applied.</div>');
    }
    out.push('</div>');
    if (S.eng && d.parameters.length) {
      out.push('<div class="vc-pane-section"><h3>Parameters</h3>');
      d.parameters.forEach(function (prm, i) {
        var cur = prm._cur !== undefined ? prm._cur : prm.current;
        if (prm.kind === 'range') {
          out.push('<div class="vc-param"><label>' + esc(prm.name) + '<span id="vc-pv-' + i + '">' + esc(cur) + '</span></label>' +
            '<input type="range" data-param="' + i + '" min="' + prm.min + '" max="' + prm.max + '" step="' + (prm.step || 1) + '" value="' + cur + '"></div>');
        } else {
          out.push('<div class="vc-param"><label>' + esc(prm.name) + '</label><select data-param="' + i + '">' +
            prm.list.map(function (x, j) { var sel = (typeof cur === 'number' && prm.list.every(function (y) { return typeof y === 'string'; })) ? j === cur : x === cur; return '<option value="' + j + '"' + (sel ? ' selected' : '') + '>' + esc(x) + '</option>'; }).join('') + '</select></div>');
        }
      });
      out.push('</div>');
    }
    if (S.eng) {
      out.push('<button class="vc-pane-btn" id="vc-clear">Clear all slicers</button>');
    } else {
      out.push('<div class="vc-note">This is a static snapshot of the report: pages, bookmarks and story points work; slicers, cross-filtering and drilling need the interactive engine.</div>');
    }
    $('vc-pane-body').innerHTML = out.join('');
  }

  // ------------------------------------------------------------------ navigation
  function goPage(id, remember) {
    if (S.eng) { S.mode = 'page'; outcome(S.eng.call('navigate', { page: id, remember: !!remember })); return; }
    if (remember && S.page !== id) S.back.push({ key: S.key, page: S.page });
    S.mode = 'page';
    S.page = id;
    S.key = 'page:' + id;
    refresh();
  }

  function goBack() {
    if (S.eng) { outcome(S.eng.call('back', {})); return; }
    var b = S.back.pop();
    if (!b) return;
    S.mode = 'page';
    S.page = b.page;
    S.key = b.key;
    refresh();
  }

  function bookmark(id) {
    S.mode = 'page';
    if (S.eng) { outcome(S.eng.call('apply_bookmark', { id: id })); return; }
    var v = S.data.views['bm:' + id];
    if (!v) return;
    S.key = 'bm:' + id;
    S.page = v.desktop.page;
    refresh();
  }

  function storyPoint(story, idx) {
    var st = S.data.stories.filter(function (s) { return s.id === story; })[0];
    if (!st) return;
    idx = Math.max(0, Math.min(idx, st.points.length - 1));
    S.mode = 'story';
    S.story = story;
    S.storyIdx = idx;
    if (S.eng) { outcome(S.eng.call('story_point', { story: story, index: idx })); return; }
    S.key = 'sp:' + story + ':' + idx;
    S.page = st.points[idx].page;
    refresh();
  }

  function runAction(id, a, ctrl) {
    var obj = null;
    if (a.charAt(0) === '{') { try { obj = JSON.parse(a); } catch (e) { return; } }
    if (obj && !S.eng) {
      switch (obj.type) {
        case 'pageNavigation': case 'drillthrough': goPage(obj.page, true); return;
        case 'bookmark': bookmark(obj.id); return;
        case 'back': goBack(); return;
        case 'url': if (/^(https?:|mailto:)/i.test(obj.url)) window.open(obj.url, '_blank', 'noopener'); return;
        default: return;
      }
    }
    if (!S.eng) return;
    if (obj && obj.type === 'bookmark') { bookmark(obj.id); return; }
    if (obj) S.mode = 'page';
    outcome(S.eng.call('action', { element: id, action: a, ctrl: !!ctrl }));
  }

  // ------------------------------------------------------------------ focus mode
  function focusMode(id) {
    var e = S.view.elements.filter(function (x) { return x.id === id; })[0];
    if (!e) return;
    $('vc-focus-title').textContent = e.title || '';
    $('vc-focus-body').innerHTML = e.svg;
    $('vc-focus').hidden = false;
  }

  // ------------------------------------------------------------------ events
  function bind() {
    var page = $('vc-page');
    page.addEventListener('click', function (ev) {
      var el = ev.target.closest('.vc-el');
      if (!el) return;
      var id = Number(el.getAttribute('data-id'));
      var ctrl = ev.ctrlKey || ev.metaKey || ev.shiftKey;
      var cmd = ev.target.closest('.vc-vh button');
      if (cmd) {
        ev.stopPropagation();
        var c = cmd.getAttribute('data-cmd');
        if (c === 'focus') focusMode(id);
        else if (S.eng) outcome(S.eng.call('drill', { element: id, dir: c }));
        return;
      }
      var act = ev.target.closest('[data-action]');
      if (act && act !== el) { runAction(id, act.getAttribute('data-action'), ctrl); return; }
      var hit = ev.target.closest('.vc-hit');
      if (hit && S.eng) {
        var pt = null;
        try { pt = JSON.parse(hit.getAttribute('data-hit')); } catch (e) { pt = null; }
        outcome(S.eng.call('click', { element: id, hit: pt, ctrl: ctrl }));
        return;
      }
      if (el.hasAttribute('data-action')) { runAction(id, el.getAttribute('data-action'), ctrl); return; }
      if (S.eng && S.view.hasSelection && el.getAttribute('data-kind') === 'visual') outcome(S.eng.call('click', { element: id, hit: null }));
    });
    // Tooltips.
    var tip = $('vc-tip');
    page.addEventListener('mousemove', function (ev) {
      var t = ev.target.closest && ev.target.closest('[data-tip]');
      if (!t) { tip.hidden = true; return; }
      var lines = t.getAttribute('data-tip').split('\n').map(function (l) {
        var i = l.indexOf(': ');
        return i < 0 ? '<div><span>' + esc(l) + '</span></div>' : '<div><span>' + esc(l.slice(0, i)) + '</span><span>' + esc(l.slice(i + 2)) + '</span></div>';
      });
      tip.innerHTML = lines.join('');
      tip.hidden = false;
      var x = ev.clientX + 14, y = ev.clientY + 14;
      var r = tip.getBoundingClientRect();
      if (x + r.width > window.innerWidth - 8) x = ev.clientX - r.width - 14;
      if (y + r.height > window.innerHeight - 8) y = ev.clientY - r.height - 14;
      tip.style.left = x + 'px';
      tip.style.top = y + 'px';
    });
    page.addEventListener('mouseleave', function () { tip.hidden = true; });
    $('vc-tabs').addEventListener('click', function (ev) {
      var b = ev.target.closest('button');
      if (!b) return;
      if (b.hasAttribute('data-page')) goPage(Number(b.getAttribute('data-page')), false);
      else if (b.hasAttribute('data-story')) storyPoint(Number(b.getAttribute('data-story')), 0);
    });
    $('vc-story').addEventListener('click', function (ev) {
      var b = ev.target.closest('button');
      if (!b || b.disabled) return;
      if (b.hasAttribute('data-point')) storyPoint(S.story, Number(b.getAttribute('data-point')));
      else if (b.hasAttribute('data-step')) storyPoint(S.story, S.storyIdx + Number(b.getAttribute('data-step')));
    });
    $('vc-layout').addEventListener('click', function (ev) {
      var b = ev.target.closest('button');
      if (!b) return;
      S.layout = b.getAttribute('data-layout');
      S.userLayout = true;
      refresh();
    });
    $('vc-back').addEventListener('click', goBack);
    $('vc-reset').addEventListener('click', function () {
      S.back = [];
      if (S.eng) S.eng.call('reset', {});
      if (S.mode === 'story') { storyPoint(S.story, S.storyIdx); return; }
      S.key = 'page:' + S.page;
      d_params_reset();
      refresh();
    });
    $('vc-filters-btn').addEventListener('click', function () { S.paneOpen = !S.paneOpen; chrome(); });
    $('vc-pane-close').addEventListener('click', function () { S.paneOpen = false; chrome(); });
    $('vc-pane-body').addEventListener('input', function (ev) {
      var t = ev.target;
      if (!t.hasAttribute('data-param')) return;
      var prm = S.data.parameters[Number(t.getAttribute('data-param'))];
      if (t.type === 'range') {
        var val = Number(t.value);
        prm._cur = val;
        var lab = $('vc-pv-' + t.getAttribute('data-param'));
        if (lab) lab.textContent = val;
        clearTimeout(S.paramTimer);
        S.paramTimer = setTimeout(function () { outcome(S.eng.call('set_parameter', { name: prm.name, value: val })); }, 120);
      } else {
        var x = prm.list[Number(t.value)];
        prm._cur = x;
        outcome(S.eng.call('set_parameter', { name: prm.name, value: x }));
      }
    });
    $('vc-pane-body').addEventListener('click', function (ev) {
      if (ev.target.id === 'vc-clear' && S.eng) { outcome(S.eng.call('action', { element: 0, action: '{"type":"clearAllSlicers"}', ctrl: false })); }
    });
    var bmBtn = $('vc-bm-btn'), bmMenu = $('vc-bm-menu');
    bmBtn.addEventListener('click', function (ev) {
      ev.stopPropagation();
      if (!bmMenu.hidden) { bmMenu.hidden = true; return; }
      var groups = {};
      S.data.bookmarks.forEach(function (b) { (groups[b.group || ''] = groups[b.group || ''] || []).push(b); });
      var html = '';
      Object.keys(groups).forEach(function (g) {
        html += g ? '<h4>' + esc(g) + '</h4>' : '<h4>Bookmarks</h4>';
        groups[g].forEach(function (b) { html += '<button data-bm="' + b.id + '">' + ICON.bookmark + esc(b.name) + '</button>'; });
      });
      bmMenu.innerHTML = html;
      bmMenu.hidden = false;
    });
    bmMenu.addEventListener('click', function (ev) {
      var b = ev.target.closest('button[data-bm]');
      if (!b) return;
      bmMenu.hidden = true;
      bookmark(Number(b.getAttribute('data-bm')));
    });
    document.addEventListener('click', function () { bmMenu.hidden = true; });
    $('vc-full').addEventListener('click', function () {
      if (document.fullscreenElement) document.exitFullscreen();
      else if (document.documentElement.requestFullscreen) document.documentElement.requestFullscreen();
    });
    $('vc-focus-close').addEventListener('click', function () { $('vc-focus').hidden = true; });
    $('vc-focus').addEventListener('click', function (ev) { if (ev.target.id === 'vc-focus') $('vc-focus').hidden = true; });
    document.addEventListener('keydown', function (ev) {
      if (ev.key === 'Escape') { $('vc-focus').hidden = true; bmMenu.hidden = true; }
      if (S.mode === 'story' && !/INPUT|SELECT|TEXTAREA/.test(document.activeElement && document.activeElement.tagName)) {
        if (ev.key === 'ArrowRight') storyPoint(S.story, S.storyIdx + 1);
        if (ev.key === 'ArrowLeft') storyPoint(S.story, S.storyIdx - 1);
      }
    });
    var lastNarrow = null;
    window.addEventListener('resize', function () {
      var narrow = window.innerWidth < 640;
      if (!S.userLayout && S.data.includeMobile && narrow !== lastNarrow && lastNarrow !== null) {
        S.layout = narrow ? 'mobile' : 'desktop';
        lastNarrow = narrow;
        refresh();
        return;
      }
      lastNarrow = narrow;
      fit();
    });
  }

  function d_params_reset() {
    (S.data.parameters || []).forEach(function (p) { delete p._cur; });
  }

  // ------------------------------------------------------------------ start
  function start(d) {
    S.data = d;
    d.pages = d.pages || [];
    d.bookmarks = d.bookmarks || [];
    d.stories = d.stories || [];
    d.parameters = d.parameters || [];
    if (qs.get('embed') === '1') {
      // Embedded (the VizardCraft Service portal): the actions move into the page tab bar.
      S.embed = true;
      $('vc-app').classList.add('vc-embed');
      $('vc-tabs').appendChild(document.querySelector('.vc-actions'));
    }
    var want = qs.get('layout');
    S.layout = (want === 'mobile' || want === 'desktop') ? want : (d.includeMobile && window.innerWidth < 640 ? 'mobile' : 'desktop');
    if (want) S.userLayout = true;
    if (want && S.embed) $('vc-app').classList.add('vc-mobile-only');
    if (!d.includeMobile) S.layout = 'desktop';
    var h = new URLSearchParams(location.hash.slice(1));
    var first = tabPages()[0] || d.pages[0];
    S.page = Number(h.get('page')) || Number(qs.get('page')) || (first ? first.id : 0);
    if (!pageInfo(S.page) && first) S.page = first.id;
    S.key = 'page:' + S.page;
    bind();
    $('vc-loading').hidden = true;
    refresh();
    if (h.get('story')) storyPoint(Number(h.get('story')), Number(h.get('point')) || 0);
    $('vc-app').classList.add('vc-static');
    loadEngine(d).then(function (e) {
      if (!e) return;
      S.eng = e;
      $('vc-app').classList.remove('vc-static');
      if (S.mode === 'story') storyPoint(S.story, S.storyIdx);
      else { S.eng.call('navigate', { page: S.page }); refresh(); }
    }).catch(function (err) {
      console.warn('VizardCraft: the interactive engine did not load; showing the static report.', err);
      chrome();
    });
  }

  loadData().then(start).catch(function (err) {
    $('vc-loading').hidden = true;
    var file = location.protocol === 'file:';
    $('vc-wrap').innerHTML = '<div class="vc-error"><b>The report could not be loaded.</b><br>' +
      (file ? 'Browsers block reading report files from a folder opened directly. Serve the folder over HTTP (for example <code>python3 -m http.server</code>) or publish it as a single HTML file.' : esc(err && err.message || err)) + '</div>';
  });
})();
