/* Meshtrike website behaviour. Vanilla JS, no dependencies, no network requests. */
(function () {
  'use strict';

  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var CFG = window.MESHTRIKE_CONFIG || {};
  var reduceMQ = window.matchMedia('(prefers-reduced-motion: reduce)');
  var fineMQ = window.matchMedia('(hover: hover) and (pointer: fine)');
  var motionOK = function () { return !reduceMQ.matches; };

  /* ---------- Toast + screen-reader announcements ---------- */
  var toastEl = $('.toast'), srEl = $('[data-sr]'), toastT;
  function toast(text) {
    if (!toastEl) return;
    clearTimeout(toastT);
    toastEl.textContent = text;
    toastEl.classList.add('show');
    toastT = setTimeout(function () { toastEl.classList.remove('show'); }, 2400);
  }
  function announce(text) { if (!srEl) return; srEl.textContent = ''; setTimeout(function () { srEl.textContent = text; }, 30); }

  /* ---------- Config: download links, file names, version labels ---------- */
  var apk = {
    meshtrike: { url: CFG.MESHTRIKE_APK_URL, name: CFG.MESHTRIKE_APK_NAME || 'meshtrike.apk' },
    reach: { url: CFG.REACH_APK_URL, name: CFG.REACH_APK_NAME || 'meshgrid.apk' }
  };
  // The HTML already carries the real URLs (for visitors without JavaScript); config.js wins when set.
  $$('[data-apk]').forEach(function (a) {
    var item = apk[a.getAttribute('data-apk')];
    if (!item) return;
    if (item.url) a.setAttribute('href', item.url);
    a.addEventListener('click', function () { toast('Downloading ' + item.name); });
  });
  $$('[data-apk-name]').forEach(function (el) {
    var item = apk[el.getAttribute('data-apk-name')];
    if (item) el.textContent = item.name;
  });
  $$('[data-ver]').forEach(function (el) {
    var v = el.getAttribute('data-ver') === 'reach' ? CFG.REACH_VERSION : CFG.MESHTRIKE_VERSION;
    if (v) el.textContent = v;
  });
  if (CFG.RELEASES_URL) $$('[data-releases]').forEach(function (a) { a.setAttribute('href', CFG.RELEASES_URL); });

  /* ---------- Header: "More" dropdown + mobile menu (both <details>, so they work without JS) ---------- */
  (function navMenus() {
    var dds = $$('[data-dd]').map(function (el) { return el.tagName === 'DETAILS' ? el : $('details', el); }).filter(Boolean);
    function close(d, focus) {
      if (!d.open) return;
      d.open = false;
      if (focus) $('summary', d).focus();
    }
    dds.forEach(function (d) {
      d.addEventListener('toggle', function () {
        if (d.open) dds.forEach(function (o) { if (o !== d) close(o); });
      });
      // leaving the dropdown with Tab closes it
      d.addEventListener('focusout', function (e) {
        if (d.open && e.relatedTarget && !d.contains(e.relatedTarget)) close(d);
      });
    });
    document.addEventListener('keydown', function (e) {
      if (e.key !== 'Escape') return;
      dds.forEach(function (d) { if (d.open) close(d, d.contains(document.activeElement) || document.activeElement === document.body); });
    });
    document.addEventListener('click', function (e) {
      dds.forEach(function (d) { if (d.open && !d.contains(e.target)) close(d); });
    });
    // close the mobile menu when switching to the desktop layout
    var wide = window.matchMedia('(min-width: 1001px)');
    var onWide = function () { if (wide.matches) dds.forEach(function (d) { close(d); }); };
    if (wide.addEventListener) wide.addEventListener('change', onWide);
  })();

  /* ---------- Hero chat: replays the conversation (final frame is static HTML) ---------- */
  (function heroChat() {
    var chat = $('.chat'), pill = $('[data-pill]');
    if (!chat || !pill || !motionOK()) return;
    var templates = $$('.bub', chat).map(function (n) { return n.cloneNode(true); });
    var pillMesh = pill.innerHTML;
    var pillBt = '<svg class="i" aria-hidden="true" focusable="false"><use href="#i-bt"/></svg><span>Nearby · Bluetooth</span>';
    var step = 0, pillState = '';

    function setStatus(el, label) {
      if (!el) return;
      var icon = label === 'Waiting' ? 'clock' : label === 'Sent' ? 'check' : 'checks';
      el.querySelector('use').setAttribute('href', '#i-' + icon);
      el.querySelector('span').textContent = label;
      el.classList.toggle('read', label === 'Read');
    }
    function render() {
      if (step === 0) chat.innerHTML = '';
      templates.forEach(function (t) {
        var at = +t.getAttribute('data-at');
        if (step >= at && !chat.querySelector('[data-at="' + at + '"]')) {
          var el = t.cloneNode(true);
          el.classList.add('anim');
          chat.appendChild(el);
        }
      });
      setStatus(chat.querySelector('[data-st="first"]'), step >= 3 ? 'Read' : 'Sent');
      setStatus(chat.querySelector('[data-st="last"]'), step >= 9 ? 'Delivered' : step >= 8 ? 'Sent' : 'Waiting');
      var want = step >= 6 ? 'mesh' : 'bt';
      if (want !== pillState) {
        pillState = want;
        pill.innerHTML = want === 'mesh' ? pillMesh : pillBt;
        pill.className = 'route-pill ' + (want === 'mesh' ? 'c-mesh' : 'c-bt');
        void pill.offsetWidth;
        pill.classList.add('anim');
      }
    }
    render();
    setInterval(function () {
      if (document.hidden) return;
      step = (step + 1) % 11;
      render();
    }, 1500);
  })();

  /* ---------- Scroll reveal (single elements) and sequences ([data-seq]: children one after another) ---------- */
  if (motionOK() && 'IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (!e.isIntersecting) return;
        e.target.classList.add(e.target.classList.contains('sq') ? 'sq-in' : 'rv-in');
        io.unobserve(e.target);
      });
    }, { threshold: 0.12, rootMargin: '0px 0px -40px 0px' });
    setTimeout(function () {
      $$('[data-reveal], [data-seq]').forEach(function (el) {
        if (el.getBoundingClientRect().top < window.innerHeight) return; // already on screen: leave it visible
        el.classList.add(el.hasAttribute('data-seq') ? 'sq' : 'rv');
        io.observe(el);
      });
    }, 60);
  }

  /* ---------- Roadmap timeline ----------
     Browsers with scroll-driven animations draw the line and reveal the stages in CSS alone.
     Elsewhere: an IntersectionObserver turns stages on as they arrive, and a scroll listener (attached only
     while the timeline is on screen) draws the line. Without JS or with reduced motion everything is shown. */
  (function timeline() {
    var tl = $('[data-tl]');
    if (!tl || !motionOK() || !('IntersectionObserver' in window)) return;
    if (window.CSS && CSS.supports && CSS.supports('animation-timeline: view()')) return;
    var items = $$('.tl__item', tl), ticking = false, live = false;
    tl.classList.add('tl--js');
    function draw() {
      ticking = false;
      var r = tl.getBoundingClientRect(), mid = window.innerHeight / 2;
      var p = Math.max(0, Math.min(1, (mid - r.top) / r.height));
      tl.style.setProperty('--tl-p', p.toFixed(4));
      items.forEach(function (it) {
        var n = $('.tl__node', it).getBoundingClientRect();
        it.classList.toggle('lit', n.top + n.height / 2 <= mid + 1);
      });
    }
    function onScroll() { if (!ticking) { ticking = true; requestAnimationFrame(draw); } }
    new IntersectionObserver(function (entries) {
      var on = entries[0].isIntersecting;
      if (on && !live) { addEventListener('scroll', onScroll, { passive: true }); addEventListener('resize', onScroll); live = true; draw(); }
      else if (!on && live) { removeEventListener('scroll', onScroll); removeEventListener('resize', onScroll); live = false; draw(); }
    }).observe(tl);
    var tio = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add('on'); tio.unobserve(e.target); } });
    }, { threshold: 0.15, rootMargin: '0px 0px -60px 0px' });
    items.forEach(function (it) { tio.observe(it); });
    draw();
  })();

  /* ---------- Cursor blob, tilt cards, magnetic buttons (mouse only) ---------- */
  if (motionOK() && fineMQ.matches) {
    var blob = $('.blob');
    if (blob) {
      var mx = innerWidth / 2, my = innerHeight / 3, bx = mx, by = my;
      blob.classList.add('on');
      addEventListener('pointermove', function (e) { mx = e.clientX; my = e.clientY; }, { passive: true });
      (function loop() {
        bx += (mx - bx) * 0.08; by += (my - by) * 0.08;
        blob.style.transform = 'translate(' + bx + 'px,' + by + 'px)';
        requestAnimationFrame(loop);
      })();
    }
    $$('[data-tilt]').forEach(function (el) {
      el.addEventListener('pointermove', function (e) {
        var b = el.getBoundingClientRect(), x = (e.clientX - b.left) / b.width - 0.5, y = (e.clientY - b.top) / b.height - 0.5;
        el.style.transform = 'perspective(900px) rotateX(' + (-y * 7) + 'deg) rotateY(' + (x * 9) + 'deg) translateY(-4px)';
      });
      el.addEventListener('pointerleave', function () { el.style.transform = ''; });
    });
    $$('[data-magnet]').forEach(function (el) {
      el.addEventListener('pointermove', function (e) {
        var b = el.getBoundingClientRect();
        el.style.transform = 'translate(' + ((e.clientX - b.left - b.width / 2) * 0.18) + 'px,' + ((e.clientY - b.top - b.height / 2) * 0.3) + 'px)';
      });
      el.addEventListener('pointerleave', function () { el.style.transform = ''; });
    });
  }

  /* ---------- Use-case tabs ---------- */
  (function tabs() {
    var list = $('[role="tablist"]');
    if (!list) return;
    var tabs = $$('[role="tab"]', list);
    function select(i, focus) {
      tabs.forEach(function (t, j) {
        var on = i === j;
        t.setAttribute('aria-selected', on);
        t.tabIndex = on ? 0 : -1;
        var panel = document.getElementById(t.getAttribute('aria-controls'));
        if (on) {
          panel.hidden = false;
          // restart the entrance animation like the design's keyed re-render
          panel.style.animation = 'none'; void panel.offsetWidth; panel.style.animation = '';
          $$('li', panel).forEach(function (li) { li.style.animation = 'none'; void li.offsetWidth; li.style.animation = ''; });
        } else {
          panel.hidden = true;
        }
      });
      if (focus) tabs[i].focus();
    }
    tabs.forEach(function (t, i) {
      t.addEventListener('click', function () { select(i, false); });
      t.addEventListener('keydown', function (e) {
        var n = tabs.length, k = e.key, to = null;
        if (k === 'ArrowDown' || k === 'ArrowRight') to = (i + 1) % n;
        else if (k === 'ArrowUp' || k === 'ArrowLeft') to = (i - 1 + n) % n;
        else if (k === 'Home') to = 0;
        else if (k === 'End') to = n - 1;
        if (to !== null) { e.preventDefault(); select(to, true); }
      });
    });
  })();

  /* ---------- FAQ accordion (one open at a time, like the design) ---------- */
  (function faq() {
    var items = $$('.qa');
    items.forEach(function (qa) {
      var btn = $('.qa__btn', qa);
      btn.addEventListener('click', function () {
        var willOpen = btn.getAttribute('aria-expanded') !== 'true';
        items.forEach(function (o) {
          o.classList.remove('open');
          $('.qa__btn', o).setAttribute('aria-expanded', 'false');
        });
        if (willOpen) { qa.classList.add('open'); btn.setAttribute('aria-expanded', 'true'); }
      });
    });
    // a link to one question (faq.html#q-meshgrid) opens that one
    function openHash() {
      var t = location.hash && document.getElementById(location.hash.slice(1));
      if (t && t.classList.contains('qa') && !t.classList.contains('open')) $('.qa__btn', t).click();
    }
    openHash();
    addEventListener('hashchange', openHash);
  })();

  /* ---------- Swag: preview filters (merch is not on sale yet, so there is no bag or checkout) ---------- */
  var products = $$('.product');
  $$('.cat').forEach(function (b) {
    b.addEventListener('click', function () {
      var cat = b.getAttribute('data-cat');
      $$('.cat').forEach(function (o) { o.setAttribute('aria-pressed', o === b); });
      products.forEach(function (p) {
        var show = cat === 'All' || p.getAttribute('data-cat') === cat;
        p.hidden = !show;
        if (show) p.classList.add('rv-in');
      });
      announce(cat === 'All' ? 'Showing all swag previews' : 'Showing ' + cat + ' previews');
    });
  });

  /* ---------- Header bag button (a link to swag.html): with JS it opens a small "merch is coming soon" dialog ---------- */
  var soonBtn = $('[data-open-soon]'), soon = $('#shop-soon'), lastFocus = null;
  if (soonBtn && soon) {
    var panel = $('.drawer__panel', soon);
    var focusables = function () {
      return $$('button, [href], [tabindex]:not([tabindex="-1"])', panel).filter(function (el) { return !el.disabled && el.offsetParent !== null; });
    };
    var openSoon = function () {
      lastFocus = document.activeElement;
      soon.classList.add('open');
      setTimeout(function () { $('.icon-btn', panel).focus(); }, 50);
    };
    var closeSoon = function (restore) {
      if (!soon.classList.contains('open')) return;
      soon.classList.remove('open');
      if (restore !== false && lastFocus) lastFocus.focus();
    };
    soonBtn.addEventListener('click', function (e) { e.preventDefault(); openSoon(); });
    $$('[data-close-soon]', soon).forEach(function (b) { b.addEventListener('click', closeSoon); });
    // the link goes to the swag previews; don't pull focus back up to the header
    $('[data-soon-go]', soon).addEventListener('click', function () { closeSoon(false); });
    document.addEventListener('keydown', function (e) {
      if (!soon.classList.contains('open')) return;
      if (e.key === 'Escape') { closeSoon(); return; }
      if (e.key === 'Tab') {
        var f = focusables(); if (!f.length) return;
        var first = f[0], last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    });
  }
})();
