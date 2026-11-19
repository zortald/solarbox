// Shared state and helpers: latest pushed values, session, navigation.
// Pages register themselves with SB.page(name, {show, hide, tick}).

const SB = (() => {
  const $ = (s, el = document) => el.querySelector(s);
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  // Boards (title, chip, restart/version entities) and UI options come from
  // the server's config.json — editable live, see /api/config.
  const BOARDS = {};
  let cfg = { ui: {} };

  const L = {};      // L[device][metric] = value   (from /api/latest)
  const T = {};      // T[device] = newest ts
  const pages = {};
  let current = null;

  const now = () => Date.now() / 1000;
  const age = (dev) => (T[dev] ? now() - T[dev] : Infinity);
  const fresh = (dev) => age(dev) < 90;
  const v = (dev, k) => { const x = L[dev]?.[k]; return typeof x === 'number' && isFinite(x) ? x : undefined; };

  function num(x, d) {
    if (x === undefined || x === null || !isFinite(x)) return '—';
    const a = Math.abs(x);
    return x.toFixed(d ?? (a >= 100 ? 0 : a >= 10 ? 1 : 2));
  }
  function ago(sec) {
    if (!isFinite(sec)) return 'никогда';
    if (sec < 90) return Math.round(sec) + ' с назад';
    if (sec < 5400) return Math.round(sec / 60) + ' мин назад';
    if (sec < 172800) return Math.round(sec / 3600) + ' ч назад';
    return Math.round(sec / 86400) + ' дн назад';
  }
  function dur(sec) {
    if (!isFinite(sec)) return '—';
    const d = Math.floor(sec / 86400), h = Math.floor(sec % 86400 / 3600), m = Math.floor(sec % 3600 / 60);
    return d ? `${d} д ${h} ч` : h ? `${h} ч ${m} мин` : `${m} мин`;
  }

  // ---- API ----
  // The session is an HttpOnly cookie; without it the server answers 401 → login page.
  async function api(path, opts = {}) {
    const headers = Object.assign({}, opts.headers || {});
    if (opts.json !== undefined) { headers['Content-Type'] = 'application/json'; opts.body = JSON.stringify(opts.json); }
    const r = await fetch(path, { method: opts.method || (opts.body ? 'POST' : 'GET'), headers, body: opts.body });
    let data = null;
    try { data = await r.json(); } catch (e) { /* non-JSON */ }
    if (r.status === 401 && data && data.login) { location.href = '/login.html'; throw new Error(data.error); }
    if (!r.ok) throw new Error((data && data.error) || 'HTTP ' + r.status);
    return data;
  }
  const entity = (board, path) => api(`/api/board/${board}/entity?path=${encodeURIComponent(path)}`);
  const act = (board, path, action, value) => api(`/api/board/${board}/action`, { json: { path, action, value } });

  // ---- dialogs (own modals instead of the browser's confirm/prompt) ----
  // Red confirm button for anything that touches real hardware or deletes something.
  const DANGER = /реальн|Прошить|Перезагруз|Удалить|Выполнить|Отключить|Убрать|установить|включить|выключить/i;
  function modal({ text, ok = 'OK', danger, input, extra }) {
    return new Promise((res) => {
      const w = document.createElement('div');
      w.className = 'modal-wrap';
      w.innerHTML = `<div class="modal" role="dialog"><div class="mtext"></div>` +
        (input === 'text' ? '<input class="minput" autocapitalize="none" spellcheck="false">' : input ? '<input class="minput" type="password" autocomplete="current-password">' : '') +
        `<div class="mbtns"><button class="btn mcancel">Отмена</button>${extra ? `<button class="btn mextra">${extra}</button>` : ''}<button class="btn mok ${danger ? 'danger' : 'on'}">${ok}</button></div></div>`;
      w.querySelector('.mtext').textContent = text;
      document.body.appendChild(w);
      const inp = w.querySelector('.minput');
      const done = (v) => { document.removeEventListener('keydown', key); w.remove(); res(v); };
      const no = () => done(input ? null : false), yes = () => done(input ? inp.value : true);
      const key = (e) => { if (e.key === 'Escape') no(); else if (e.key === 'Enter') yes(); };
      document.addEventListener('keydown', key);
      w.querySelector('.mcancel').onclick = no;
      w.querySelector('.mok').onclick = yes;
      if (extra) w.querySelector('.mextra').onclick = () => done('extra');
      w.onclick = (e) => { if (e.target === w) no(); };
      setTimeout(() => (inp || w.querySelector('.mok')).focus(), 30);
    });
  }
  const ask = (text, o = {}) => modal({ text, ok: o.ok || 'OK', danger: o.danger ?? DANGER.test(text) });
  const askText = (text, o = {}) => modal({ text, ok: o.ok || 'Войти', input: o.plain ? 'text' : true });

  // ---- toast ----
  function toast(t, bad) {
    const m = $('#toast'); m.textContent = t; m.className = bad ? 'show bad' : 'show';
    clearTimeout(toast.t); toast.t = setTimeout(() => (m.className = ''), bad ? 6000 : 3500);
  }

  // ---- session ----
  // Only logged-in users get this far (the server sends everyone else to
  // /login.html), and whoever is logged in may do everything.
  const isAdmin = true;
  async function logout() {
    const r = await modal({ text: 'Выйти из solarbox на этом устройстве? «Выйти везде» закрывает все сессии на всех устройствах.', ok: 'Выйти', extra: 'Выйти везде' });
    if (r === false) return;
    try { await api(r === 'extra' ? '/api/logout-all' : '/api/logout', { method: 'POST' }); } catch (e) { /* going to the login page anyway */ }
    location.href = '/login.html';
  }
  function renderAdmin() {
    const b = $('#admin');
    b.textContent = 'Выйти';
    document.body.classList.add('admin');
  }
  const requireAdmin = () => true;

  // ---- latest values ----
  async function refreshLatest() {
    try {
      const d = await api('/api/latest');
      for (const dev of Object.keys(d)) {
        L[dev] = {}; let t = 0;
        for (const [k, o] of Object.entries(d[dev])) { L[dev][k] = o.v; if (o.ts > t) t = o.ts; }
        T[dev] = t;
      }
      renderStatus();
    } catch (e) {
      $('#status').innerHTML = '<span class="dot bad"></span>сервер недоступен';
    }
    pages[current]?.tick?.();
  }

  function renderStatus() {
    const parts = Object.keys(BOARDS).filter((dev) => BOARDS[dev].status !== false).map((dev) => {
      const ok = fresh(dev);
      return `<span class="dot ${ok ? 'ok' : 'bad'}"></span>${esc(BOARDS[dev].title)}${ok ? '' : ' · ' + ago(age(dev))}`;
    });
    $('#status').innerHTML = parts.join(' &nbsp; ') + (live ? ' <span class="live">● live</span>' : '');
  }

  // Live push: the server re-sends every packet a board posts; merge it in and
  // redraw (coalesced). /api/latest polling stays as a slow safety net.
  let live = false, redraw = null;
  function stream() {
    const es = new EventSource('/api/stream');
    es.onopen = () => { live = true; renderStatus(); };
    es.onmessage = (ev) => {
      let p; try { p = JSON.parse(ev.data); } catch (e) { return; }
      const d = (L[p.device] ||= {});
      Object.assign(d, p.metrics);
      if (!T[p.device] || p.ts > T[p.device]) T[p.device] = p.ts;
      clearTimeout(redraw);
      redraw = setTimeout(() => { renderStatus(); pages[current]?.tick?.(); }, 150);
    };
    es.onerror = () => { live = false; renderStatus(); }; // EventSource reconnects by itself
  }

  // ---- navigation ----
  function page(name, p) { pages[name] = p; }
  function go() {
    const name = (location.hash || '#flow').slice(1);
    const target = pages[name] ? name : 'flow';
    if (current && current !== target) pages[current]?.hide?.();
    document.querySelectorAll('main > section[data-page]').forEach((s) => (s.hidden = s.dataset.page !== target));
    document.querySelectorAll('nav a').forEach((a) => a.classList.toggle('on', a.getAttribute('href') === '#' + target));
    current = target;
    const link = document.querySelector(`nav a[href="#${target}"]`);
    $('#page-title').textContent = link ? ' · ' + link.textContent : '';
    $('#nav').classList.remove('open');
    pages[target]?.show?.();
    pages[target]?.tick?.();
  }

  async function loadConfig() {
    try { cfg = await api('/api/config'); } catch (e) { cfg = { boards: [], ui: {} }; }
    cfg.ui = cfg.ui || {};
    for (const k of Object.keys(BOARDS)) delete BOARDS[k];
    for (const b of cfg.boards || []) BOARDS[b.name] = Object.assign({ title: b.name, chip: '' }, b);
    if (cfg.load_error) toast('config.json с ошибкой, работает прежний: ' + cfg.load_error, true);
  }

  async function start() {
    $('#admin').onclick = () => { $('#nav').classList.remove('open'); logout(); };
    $('#menu-btn').onclick = () => $('#nav').classList.toggle('open');
    $('#nav-shade').onclick = () => $('#nav').classList.remove('open');
    document.querySelectorAll('nav a').forEach((a) => (a.onclick = () => $('#nav').classList.remove('open')));
    window.addEventListener('hashchange', go);
    await loadConfig();
    renderAdmin();
    refreshLatest().then(go);
    stream();
    setInterval(refreshLatest, 30000);
    setInterval(renderStatus, 5000);   // keeps "молчит N с" ages moving
  }

  return { $, esc, BOARDS, L, T, v, fresh, age, ago, dur, num, api, entity, act, toast, requireAdmin, page, start, ask, askText,
    isAdmin, get cfg() { return cfg; } };
})();
