// Statistics: daily energy (kWh) from the server's per-packet integration —
// bars per day (or per month for a year), toggles remembered, table with totals.

(() => {
  const { $, api, page } = SB;
  const SER = [
    { k: 'pv', label: 'Солнце', color: '#f5b041' },
    { k: 'load', label: 'Дом', color: '#ff9eb5' },
    { k: 'grid', label: 'Сеть ≈', color: '#4ea3ff' },
    { k: 'chg', label: 'В АКБ', color: '#3fb950' },
    { k: 'dis', label: 'Из АКБ', color: '#56d4dd' },
  ];
  const RANGES = [['7d', '7 дней', 7], ['30d', '30 дней', 30], ['12m', '12 месяцев', 366]];
  const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch (e) { return d; } };
  const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} };
  let range = load('sbStatsRange', '30d'), hidden = new Set(load('sbStatsHidden', ['chg', 'dis'])), data = null;
  const MONTHS = ['янв', 'фев', 'мар', 'апр', 'май', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];

  function rows() {
    if (!data) return [];
    const r = RANGES.find((x) => x[0] === range) || RANGES[1];
    const days = data.days;
    if (r[0] !== '12m') {
      // fill missing days so the axis is continuous
      const out = [], end = new Date(data.today + 'T12:00:00');
      const by = Object.fromEntries(days.map((d) => [d.day, d]));
      for (let i = r[2] - 1; i >= 0; i--) {
        const d = new Date(end); d.setDate(d.getDate() - i);
        const key = d.toISOString().slice(0, 10);
        out.push({ ...(by[key] || {}), day: key, label: `${d.getDate()}.${String(d.getMonth() + 1).padStart(2, '0')}` });
      }
      return out;
    }
    const by = {};
    for (const d of days) {
      const m = d.day.slice(0, 7);
      const o = (by[m] ||= { day: m, label: MONTHS[+m.slice(5) - 1] + ' ' + m.slice(2, 4) });
      for (const s of SER) o[s.k] = (o[s.k] || 0) + (d[s.k] || 0);
    }
    return Object.values(by);
  }

  function draw() {
    const canvas = $('#st-chart'), R = rows(), shown = SER.filter((s) => !hidden.has(s.k));
    const dpr = window.devicePixelRatio || 1, W = canvas.clientWidth, H = canvas.clientHeight;
    canvas.width = W * dpr; canvas.height = H * dpr;
    const ctx = canvas.getContext('2d'); ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, W, H);
    const css = getComputedStyle(document.body), line = css.getPropertyValue('--line').trim(), muted = css.getPropertyValue('--muted').trim();
    const L = 40, Rp = 8, T = 10, B = 22, pw = W - L - Rp, ph = H - T - B;
    let max = 0; for (const r of R) for (const s of shown) max = Math.max(max, r[s.k] || 0);
    if (!max) max = 1;
    const step = [0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100, 200, 500].find((x) => max / x <= 5) || 1000;
    max = Math.ceil(max / step) * step;
    ctx.font = '10px sans-serif'; ctx.textAlign = 'right'; ctx.textBaseline = 'middle';
    for (let v = 0; v <= max + 1e-9; v += step) {
      const y = T + ph - (v / max) * ph;
      ctx.strokeStyle = line; ctx.beginPath(); ctx.moveTo(L, y); ctx.lineTo(W - Rp, y); ctx.stroke();
      ctx.fillStyle = muted; ctx.fillText(v.toFixed(step < 1 ? 1 : 0), L - 5, y);
    }
    const gw = pw / Math.max(R.length, 1), bw = Math.max(1.5, Math.min(18, (gw * 0.8) / Math.max(shown.length, 1)));
    const every = Math.ceil(R.length / Math.max(1, Math.floor(pw / 42)));
    R.forEach((r, i) => {
      const x0 = L + i * gw + (gw - bw * shown.length) / 2;
      shown.forEach((s, j) => {
        const v = r[s.k] || 0, h = (v / max) * ph;
        ctx.fillStyle = s.color; ctx.fillRect(x0 + j * bw, T + ph - h, Math.max(bw - 1, 1), h);
      });
      if (i % every === 0) { ctx.fillStyle = muted; ctx.textAlign = 'center'; ctx.textBaseline = 'top'; ctx.fillText(r.label, L + i * gw + gw / 2, H - B + 5); }
    });
    canvas.onclick = (e) => {
      const r = R[Math.floor((e.clientX - canvas.getBoundingClientRect().left - L) / gw)];
      if (r && r.day.length === 10) openDay(r.day);
    };
    canvas.style.cursor = range === '12m' ? '' : 'pointer';
    canvas.onmousemove = (e) => {
      const i = Math.floor((e.clientX - canvas.getBoundingClientRect().left - L) / gw), r = R[i];
      $('#st-tip').innerHTML = r ? `<b>${r.label}</b> · ` + shown.map((s) => `<span style="color:${s.color}">${s.label} ${(r[s.k] || 0).toFixed(1)}</span>`).join(' · ') + ' кВт·ч' : '';
    };
  }

  function table() {
    const R = rows().filter((r) => SER.some((s) => r[s.k])).reverse();
    const tot = {}; for (const r of R) for (const s of SER) tot[s.k] = (tot[s.k] || 0) + (r[s.k] || 0);
    const f = (v) => (v ? v.toFixed(1) : '—');
    $('#st-table').innerHTML = R.length ? `<table class="stab"><tr><th></th>${SER.map((s) => `<th style="color:${s.color}">${s.label}</th>`).join('')}</tr>
      <tr class="tot"><td>Итого</td>${SER.map((s) => `<td>${f(tot[s.k])}</td>`).join('')}</tr>
      ${R.map((r) => `<tr${r.day.length === 10 ? ` class="link" data-day="${r.day}"` : ''}><td>${r.label}</td>${SER.map((s) => `<td>${f(r[s.k])}</td>`).join('')}</tr>`).join('')}</table>
      <p class="muted small">кВт·ч. Считается на сервере по каждому пакету от плат (раз в 5 с). «Сеть ≈» — оценка по балансу: инвертор не измеряет мощность сети.</p>`
      : '<div class="empty">Пока нет данных за этот период.</div>';
    document.querySelectorAll('#st-table [data-day]').forEach((tr) => (tr.onclick = () => openDay(tr.dataset.day)));
  }

  // ---- one day: totals, peaks, power chart with outages, outage list ----
  const DAYSER = [
    { k: 'pv', d: 'inverter', m: 'pv_w', label: 'Солнце', unit: 'Вт', color: '#f5b041' },
    { k: 'load', d: 'inverter', m: 'ac_out_w', label: 'Дом', unit: 'Вт', color: '#ff9eb5' },
    { k: 'batt', d: 'bms', m: 'bms_w', label: 'АКБ ±', unit: 'Вт', color: '#3fb950' },
    { k: 'soc', d: 'bms', m: 'soc', label: 'Заряд АКБ', unit: '%', color: '#56d4dd', axis: 'right', fixed: [0, 100] },
  ];
  const OUT_BG = 'rgba(248,81,73,0.22)';
  const dayHidden = new Set(load('sbDayHidden', []));
  let day = null, dayData = null;
  const p2 = (n) => String(n).padStart(2, '0');
  const iso = (d) => `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())}`;
  const shift = (key, n) => { const d = new Date(key + 'T12:00:00'); d.setDate(d.getDate() + n); return iso(d); };
  const hm = (t) => { const d = new Date(t * 1000); return `${p2(d.getHours())}:${p2(d.getMinutes())}`; };
  const span = (s) => s >= 3600 ? `${Math.floor(s / 3600)} ч ${Math.round((s % 3600) / 60)} мин` : `${Math.max(1, Math.round(s / 60))} мин`;

  function openDay(key) {
    day = key; $('#sd-date').value = key;
    $('#st-day').scrollIntoView({ behavior: 'smooth', block: 'start' });
    loadDay();
  }

  async function loadDay() {
    const from = Math.floor(new Date(day + 'T00:00:00') / 1000), to = Math.floor(new Date(shift(day, 1) + 'T00:00:00') / 1000);
    const now = Math.floor(Date.now() / 1000);
    $('#sd-next').disabled = day >= iso(new Date());
    const [series, out, energy] = await Promise.all([
      Promise.all(DAYSER.map((c) => api(`/api/series?device=${c.d}&metric=${c.m}&from=${from}&to=${Math.min(to, now)}&points=600`).catch(() => ({ ts: [], v: [] })))),
      api(`/api/outages?from=${from}&to=${to}`).catch(() => ({ items: [] })),
      api('/api/energy?days=800').catch(() => ({ days: [] })),
    ]);
    const outs = (out.items || []).filter((o) => o.start < to && (o.end ?? now) > from)
      .map((o) => ({ ...o, from: Math.max(o.start, from), to: Math.min(o.end ?? now, to) }));
    dayData = { from, to, series, outs, kwh: (energy.days || []).find((d) => d.day === day) || {} };
    renderDay();
  }

  function renderDay() {
    if (!dayData) return;
    const { from, to, series, outs, kwh } = dayData;
    const s = Object.fromEntries(DAYSER.map((c, i) => [c.k, series[i]]));
    const peak = (k) => (s[k].v.length ? Math.max(...s[k].v) : undefined);
    const low = (k) => (s[k].v.length ? Math.min(...s[k].v) : undefined);
    const f = (v, d = 1) => (v === undefined || v === null ? '—' : (+v).toFixed(d));
    const offSecs = outs.reduce((a, o) => a + (o.to - o.from), 0);
    $('#sd-facts').innerHTML = [
      ['Солнце', `${f(kwh.pv)} кВт·ч`, `пик ${f(peak('pv'), 0)} Вт`],
      ['Дом', `${f(kwh.load)} кВт·ч`, `пик ${f(peak('load'), 0)} Вт`],
      ['Сеть ≈', `${f(kwh.grid)} кВт·ч`, ''],
      ['АКБ', `+${f(kwh.chg)} / −${f(kwh.dis)} кВт·ч`, `заряд ${f(low('soc'), 0)}–${f(peak('soc'), 0)} %`],
      ['Без света', outs.length ? span(offSecs) : 'не было', outs.length ? `${outs.length} раз` : ''],
    ].map(([l, v, sub]) => `<div><span>${l}</span><b>${v}</b>${sub ? `<small class="muted">${sub}</small>` : ''}</div>`).join('');
    $('#sd-legend').innerHTML = DAYSER.map((c) => `<label><input type="checkbox" data-dk="${c.k}"${dayHidden.has(c.k) ? '' : ' checked'}>` +
      `<i style="background:${c.color}"></i>${c.label}${c.axis === 'right' ? ' (правая шкала)' : ''}</label>`).join('') +
      `<label><i style="background:${OUT_BG};width:18px"></i>фон — без света</label>`;
    document.querySelectorAll('#sd-legend input').forEach((i) => (i.onchange = () => {
      i.checked ? dayHidden.delete(i.dataset.dk) : dayHidden.add(i.dataset.dk); save('sbDayHidden', [...dayHidden]); renderDay();
    }));
    Plot($('#sd-chart'), {
      from, to,
      bands: outs.map((o) => ({ from: o.from, to: o.to, color: OUT_BG })),
      series: DAYSER.filter((c) => !dayHidden.has(c.k)).map((c) => ({ ...s[c.k], color: c.color, label: c.label, unit: c.unit, axis: c.axis, fixed: c.fixed })),
    });
    $('#sd-out').innerHTML = outs.length ? '<h3>Отключения</h3><table class="olist">' + outs.map((o) =>
      `<tr><td>${hm(o.start)} – ${o.end ? hm(o.end) : 'сейчас'}</td><td>${span((o.end ?? Math.floor(Date.now() / 1000)) - o.start)}</td>` +
      `<td>${o.soc_start != null ? `АКБ ${Math.round(o.soc_start)}${o.soc_end != null ? ' → ' + Math.round(o.soc_end) : ''} %` : ''}</td></tr>`).join('') + '</table>' : '';
  }

  function dayControls() {
    $('#sd-date').max = iso(new Date());
    $('#sd-date').onchange = () => { if ($('#sd-date').value) { day = $('#sd-date').value; loadDay(); } };
    $('#sd-prev').onclick = () => { day = shift(day, -1); $('#sd-date').value = day; loadDay(); };
    $('#sd-next').onclick = () => { if (day < iso(new Date())) { day = shift(day, 1); $('#sd-date').value = day; loadDay(); } };
    $('#sd-today').onclick = () => { day = iso(new Date()); $('#sd-date').value = day; loadDay(); };
  }

  function controls() {
    $('#st-ranges').innerHTML = RANGES.map(([k, l]) => `<button class="btn${k === range ? ' on' : ''}" data-r="${k}">${l}</button>`).join('');
    document.querySelectorAll('#st-ranges button').forEach((b) => (b.onclick = () => { range = b.dataset.r; save('sbStatsRange', range); controls(); fetchData(); }));
    $('#st-legend').innerHTML = SER.map((s) => `<label><input type="checkbox" data-k="${s.k}"${hidden.has(s.k) ? '' : ' checked'}><i style="background:${s.color}"></i>${s.label}</label>`).join('');
    document.querySelectorAll('#st-legend input').forEach((i) => (i.onchange = () => { i.checked ? hidden.delete(i.dataset.k) : hidden.add(i.dataset.k); save('sbStatsHidden', [...hidden]); draw(); }));
  }

  async function fetchData() {
    const r = RANGES.find((x) => x[0] === range) || RANGES[1];
    try { data = await api('/api/energy?days=' + r[2]); } catch (e) { data = { days: [], today: '' }; }
    draw(); table();
  }

  page('stats', { show() {
    controls(); fetchData(); dayControls();
    if (!day) { day = iso(new Date()); $('#sd-date').value = day; }
    loadDay();
  } });
  window.addEventListener('resize', () => { if (!$('#page-stats').hidden) { draw(); renderDay(); } });
})();
