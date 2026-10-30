// Statistics: daily energy (kWh) from the server's per-packet integration.
// One chart, two modes: bars per day / per month, or «День» — the power curves
// of one day with outages as background. Clicking a bar or a table row opens
// that day. Range, hidden series and the day are remembered.

(() => {
  const { $, api, page } = SB;
  const SER = [
    { k: 'pv', label: 'Солнце', color: '#f5b041' },
    { k: 'load', label: 'Дом', color: '#ff9eb5' },
    { k: 'grid', label: 'Сеть ≈', color: '#4ea3ff' },
    { k: 'chg', label: 'В АКБ', color: '#3fb950' },
    { k: 'dis', label: 'Из АКБ', color: '#56d4dd' },
  ];
  const DAYSER = [
    { k: 'pv', d: 'inverter', m: 'pv_w', label: 'Солнце', unit: 'Вт', color: '#f5b041' },
    { k: 'load', d: 'inverter', m: 'ac_out_w', label: 'Дом', unit: 'Вт', color: '#ff9eb5' },
    { k: 'batt', d: 'bms', m: 'bms_w', label: 'АКБ ±', unit: 'Вт', color: '#3fb950' },
    { k: 'soc', d: 'bms', m: 'soc', label: 'Заряд АКБ', unit: '%', color: '#56d4dd', axis: 'right', fixed: [0, 100] },
  ];
  const OUT_BG = 'rgba(248,81,73,0.22)';
  // [key, label, days of energy data to fetch]; «day» also fetches 30 for the table
  // «month» fetches from the month's start up to today (computed in fetchData)
  const RANGES = [['day', 'День', 30], ['month', 'Месяц', 0], ['7d', '7 дней', 7], ['30d', '30 дней', 30], ['12m', '12 месяцев', 366]];
  const rng = (k) => RANGES.find((x) => x[0] === k) || RANGES.find((x) => x[0] === '30d');
  const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch (e) { return d; } };
  const save = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} };
  let range = load('sbStatsRange', '30d');
  if (!RANGES.some((r) => r[0] === range)) range = '30d';
  const hidden = new Set(load('sbStatsHidden', ['chg', 'dis'])), dayHidden = new Set(load('sbDayHidden', []));
  let data = null, day = null, dayData = null, month = null, cur = 'грн', hasTariff = false;
  const money = (v) => (v === undefined || !isFinite(v) ? '—' : `${v.toFixed(Math.abs(v) < 100 ? 2 : 0)} ${cur}`);
  const MONTHS = ['янв', 'фев', 'мар', 'апр', 'май', 'июн', 'июл', 'авг', 'сен', 'окт', 'ноя', 'дек'];
  const p2 = (n) => String(n).padStart(2, '0');
  const iso = (d) => `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())}`;
  const shift = (key, n) => { const d = new Date(key + 'T12:00:00'); d.setDate(d.getDate() + n); return iso(d); };
  const hm = (t) => { const d = new Date(t * 1000); return `${p2(d.getHours())}:${p2(d.getMinutes())}`; };
  const span = (s) => s >= 3600 ? `${Math.floor(s / 3600)} ч ${Math.round((s % 3600) / 60)} мин` : `${Math.max(1, Math.round(s / 60))} мин`;
  const MONTHS_FULL = ['Январь', 'Февраль', 'Март', 'Апрель', 'Май', 'Июнь', 'Июль', 'Август', 'Сентябрь', 'Октябрь', 'Ноябрь', 'Декабрь'];
  const thisMonth = () => iso(new Date()).slice(0, 7);
  const nextMonth = (m) => { let [y, mo] = m.split('-').map(Number); mo++; if (mo > 12) { mo = 1; y++; } return `${y}-${p2(mo)}`; };
  const prevMonth = (m) => { let [y, mo] = m.split('-').map(Number); mo--; if (mo < 1) { mo = 12; y--; } return `${y}-${p2(mo)}`; };
  const monthLabel = (m) => `${MONTHS_FULL[+m.slice(5) - 1]} ${m.slice(0, 4)}`;
  const dayLabel = (key) => { const d = new Date(key + 'T12:00:00'); return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`; };

  // ---- bars: days or months ----
  function rows() {
    if (!data) return [];
    const r = rng(range);
    const days = data.days;
    if (r[0] === 'month') {
      const by = Object.fromEntries(days.map((d) => [d.day, d]));
      const out = [];
      for (let key = month + '-01'; key.slice(0, 7) === month; key = shift(key, 1)) out.push({ ...(by[key] || {}), day: key, label: String(+key.slice(8)) });
      return out;
    }
    if (r[0] !== '12m') {
      // fill missing days so the axis is continuous
      const out = [], end = new Date(data.today + 'T12:00:00');
      const by = Object.fromEntries(days.map((d) => [d.day, d]));
      for (let i = r[2] - 1; i >= 0; i--) {
        const d = new Date(end); d.setDate(d.getDate() - i);
        const key = iso(d);
        out.push({ ...(by[key] || {}), day: key, label: `${d.getDate()}.${p2(d.getMonth() + 1)}` });
      }
      return out;
    }
    const by = {};
    for (const d of days) {
      const m = d.day.slice(0, 7);
      const o = (by[m] ||= { day: m, label: MONTHS[+m.slice(5) - 1] + ' ' + m.slice(2, 4) });
      for (const k of [...SER.map((s) => s.k), 'cost', 'saved']) o[k] = (o[k] || 0) + (d[k] || 0);
      // night split only exists for two-zone days; keep it undefined otherwise
      for (const k of ['cost_night', 'grid_night', 'grid_day']) if (k in d) o[k] = (o[k] || 0) + d[k];
    }
    return Object.values(by);
  }

  function bars() {
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
    const at = (e) => R[Math.floor((e.clientX - canvas.getBoundingClientRect().left - L) / gw)];
    canvas.onclick = (e) => { const r = at(e); if (r) r.day.length === 10 ? openDay(r.day) : openMonth(r.day); };
    canvas.style.cursor = 'pointer';
    canvas.onmousemove = (e) => {
      const r = at(e);
      $('#st-tip').innerHTML = r ? `<b>${r.label}</b> · ` + shown.map((s) => `<span style="color:${s.color}">${s.label} ${(r[s.k] || 0).toFixed(1)}</span>`).join(' · ') +
        ' кВт·ч <span class="muted">— нажми, чтобы открыть ' + (r.day.length === 10 ? 'день' : 'месяц') + '</span>' : '&nbsp;';
    };
    canvas.ontouchmove = canvas.onmouseleave = null;
  }

  // ---- summary for any period: totals, peaks, SOC range, outages ----
  const fmt = (v, d = 1) => (v === undefined || v === null || !isFinite(v) ? '—' : (+v).toFixed(d));
  function summary(days, outs, nDays) {
    const sum = (k) => days.reduce((a, d) => a + (d[k] || 0), 0);
    const ext = (k, fn) => { const v = days.map((d) => d[k]).filter((x) => x !== undefined); return v.length ? fn(...v) : undefined; };
    const offSecs = outs.reduce((a, o) => a + (o.to - o.from), 0);
    const avg = (k) => (nDays > 1 && days.length ? ` · ≈${fmt(sum(k) / days.length)} в день` : '');
    $('#sd-facts').innerHTML = [
      ['Солнце', `${fmt(sum('pv'))} кВт·ч`, `пик ${fmt(ext('pv_max', Math.max), 0)} Вт${avg('pv')}`],
      ['Дом', `${fmt(sum('load'))} кВт·ч`, `пик ${fmt(ext('load_max', Math.max), 0)} Вт${avg('load')}`],
      ['Сеть ≈', `${fmt(sum('grid'))} кВт·ч`, avg('grid').replace(' · ', '')],
      ['АКБ', `+${fmt(sum('chg'))} / −${fmt(sum('dis'))} кВт·ч`, `заряд ${fmt(ext('soc_min', Math.min), 0)}–${fmt(ext('soc_max', Math.max), 0)} %`],
      ...(hasTariff ? [['Деньги за сеть', `≈ ${money(sum('cost'))}`, `сэкономлено ≈ ${money(sum('saved'))}${nDays > 1 && days.length ? ` · ≈${money(sum('cost') / days.length)} в день` : ''}`]] : []),
      ...(hasTariff && days.some((d) => d.cost_night !== undefined) ? [
        ['Днём', `≈ ${money(sum('cost') - sum('cost_night'))}`, `${fmt(sum('grid_day'))} кВт·ч из сети`],
        ['Ночью', `≈ ${money(sum('cost_night'))}`, `${fmt(sum('grid_night'))} кВт·ч из сети`],
      ] : []),
      ['Без света', outs.length ? span(offSecs) : 'не было', outs.length ? `${outs.length} раз` + (outs.length > 1 ? ` · самое долгое ${span(Math.max(...outs.map((o) => o.to - o.from)))}` : '') : ''],
    ].map(([l, v, sub]) => `<div><span>${l}</span><b>${v}</b>${sub ? `<small>${sub}</small>` : ''}</div>`).join('');
    const long = nDays > 1, MAXROWS = 30;
    const when = (t) => { const d = new Date(t * 1000); return (long ? `${d.getDate()}.${p2(d.getMonth() + 1)} ` : '') + hm(t); };
    $('#sd-out').innerHTML = outs.length ? '<h3>Отключения</h3><table class="olist">' + outs.slice(0, MAXROWS).map((o) =>
      `<tr><td>${when(o.start)} – ${o.end ? hm(o.end) : 'сейчас'}</td><td>${span((o.end ?? Math.floor(Date.now() / 1000)) - o.start)}</td>` +
      `<td>${o.soc_start != null ? `АКБ ${Math.round(o.soc_start)}${o.soc_end != null ? ' → ' + Math.round(o.soc_end) : ''} %` : ''}</td></tr>`).join('') + '</table>' +
      (outs.length > MAXROWS ? `<p class="muted small">и ещё ${outs.length - MAXROWS} — все на странице «Отключения»</p>` : '') : '';
  }
  const clipOuts = (items, from, to) => {
    const now = Math.floor(Date.now() / 1000);
    return (items || []).filter((o) => o.start < to && (o.end ?? now) > from).map((o) => ({ ...o, from: Math.max(o.start, from), to: Math.min(o.end ?? now, to) }));
  };

  function openMonth(m) {
    month = m; save('sbStatsMonth', month);
    if (range !== 'month') { range = 'month'; save('sbStatsRange', range); controls(); }
    $('#page-stats').scrollIntoView({ behavior: 'smooth', block: 'start' });
    fetchData();
  }

  // ---- one day: totals, peaks, power chart with outages, outage list ----
  function openDay(key) {
    day = key; save('sbStatsDay', day);
    if (range !== 'day') { range = 'day'; save('sbStatsRange', range); controls(); }
    $('#page-stats').scrollIntoView({ behavior: 'smooth', block: 'start' });
    loadDay();
  }

  async function loadDay() {
    $('#sd-date').value = day;
    $('#sd-next').disabled = day >= iso(new Date());
    const from = Math.floor(new Date(day + 'T00:00:00') / 1000), to = Math.floor(new Date(shift(day, 1) + 'T00:00:00') / 1000);
    const now = Math.floor(Date.now() / 1000);
    const [series, out, energy] = await Promise.all([
      Promise.all(DAYSER.map((c) => api(`/api/series?device=${c.d}&metric=${c.m}&from=${from}&to=${Math.min(to, now)}&points=600`).catch(() => ({ ts: [], v: [] })))),
      api(`/api/outages?from=${from}&to=${to}`).catch(() => ({ items: [] })),
      api('/api/energy?days=800').catch(() => ({ days: [] })),
    ]);
    const outs = clipOuts(out.items, from, to);
    cur = energy.currency || cur; hasTariff = !!energy.tariffs;
    dayData = { day, from, to, series, outs, kwh: (energy.days || []).find((d) => d.day === day) || {} };
    if (range === 'day') render();
  }

  function dayView() {
    if (!dayData || dayData.day !== day) { loadDay(); return; }
    const { from, to, series, outs, kwh } = dayData;
    const s = Object.fromEntries(DAYSER.map((c, i) => [c.k, series[i]]));
    // Exact daily peaks come from the server; the curve is only a fallback.
    const fromCurve = (k, fn) => (s[k].v.length ? fn(...s[k].v) : undefined);
    const k = { ...kwh };
    k.pv_max ??= fromCurve('pv', Math.max); k.load_max ??= fromCurve('load', Math.max);
    k.soc_min ??= fromCurve('soc', Math.min); k.soc_max ??= fromCurve('soc', Math.max);
    $('#st-title').textContent = dayLabel(day);
    summary([k], outs, 1);
    $('#st-legend').innerHTML = DAYSER.map((c) => `<label><input type="checkbox" data-dk="${c.k}"${dayHidden.has(c.k) ? '' : ' checked'}>` +
      `<i style="background:${c.color}"></i>${c.label}${c.axis === 'right' ? ' (правая шкала)' : ''}</label>`).join('') +
      `<label><i style="background:${OUT_BG};width:18px"></i>фон — без света</label>`;
    document.querySelectorAll('#st-legend [data-dk]').forEach((i) => (i.onchange = () => {
      i.checked ? dayHidden.delete(i.dataset.dk) : dayHidden.add(i.dataset.dk); save('sbDayHidden', [...dayHidden]); render();
    }));
    const canvas = $('#st-chart');
    canvas.onclick = null; canvas.style.cursor = '';
    Plot(canvas, {
      from, to,
      bands: outs.map((o) => ({ from: o.from, to: o.to, color: OUT_BG })),
      series: DAYSER.filter((c) => !dayHidden.has(c.k)).map((c) => ({ ...s[c.k], color: c.color, label: c.label, unit: c.unit, axis: c.axis, fixed: c.fixed })),
    });
    $('#st-tip').innerHTML = '&nbsp;';
  }

  // ---- table (always days/months of the data loaded; day rows open the day) ----
  function table() {
    const R = rows().filter((r) => SER.some((s) => r[s.k])).reverse();
    const tot = {}; for (const r of R) for (const k of [...SER.map((s) => s.k), 'cost', 'cost_night']) tot[k] = (tot[k] || 0) + (r[k] || 0);
    const zones = hasTariff && R.some((r) => r.cost_night !== undefined);
    const m2 = (v) => `<td>${v ? v.toFixed(v < 100 ? 2 : 0) : '—'}</td>`;
    const mc = (r) => (!hasTariff ? '' : zones ? m2((r.cost || 0) - (r.cost_night || 0)) + m2(r.cost_night) : m2(r.cost));
    const mh = !hasTariff ? '' : zones ? `<th>днём, ${cur}</th><th>ночью, ${cur}</th>` : `<th>${cur}</th>`;
    const f = (v) => (v ? v.toFixed(1) : '—');
    $('#st-table').innerHTML = R.length ? `<table class="stab"><tr><th></th>${SER.map((s) => `<th style="color:${s.color}">${s.label}</th>`).join('')}${mh}</tr>
      <tr class="tot"><td>Итого</td>${SER.map((s) => `<td>${f(tot[s.k])}</td>`).join('')}${mc(tot)}</tr>
      ${R.map((r) => `<tr${r.day.length === 10 ? ` class="link${range === 'day' && r.day === day ? ' sel' : ''}" data-day="${r.day}"` : ` class="link" data-month="${r.day}"`}><td>${r.label}</td>${SER.map((s) => `<td>${f(r[s.k])}</td>`).join('')}${mc(r)}</tr>`).join('')}</table>
      <p class="muted small">кВт·ч. Считается на сервере по каждому пакету от плат (раз в 5 с). «Сеть ≈» — оценка по балансу: инвертор не измеряет мощность сети${hasTariff ? `, поэтому и деньги — оценка (${cur} считаются по тарифу на каждый час)` : ''}. Нажми на строку, чтобы открыть ${range === '12m' ? 'месяц' : 'день'}.</p>`
      : '<div class="empty">Пока нет данных за этот период.</div>';
    document.querySelectorAll('#st-table [data-day]').forEach((tr) => (tr.onclick = () => openDay(tr.dataset.day)));
    document.querySelectorAll('#st-table [data-month]').forEach((tr) => (tr.onclick = () => openMonth(tr.dataset.month)));
  }

  function render() {
    const isDay = range === 'day';
    $('#sd-nav').hidden = !isDay;
    $('#sm-nav').hidden = range !== 'month';
    if (isDay) dayView();
    else {
      const r = rng(range);
      let first, end = '9999-12-31';
      if (range === 'month') { first = month + '-01'; end = nextMonth(month) + '-01'; }
      else first = range === '12m' ? (data.days[0]?.day || data.today) : shift(data.today || iso(new Date()), -(r[2] - 1));
      const from = Math.floor(new Date(first + 'T00:00:00') / 1000);
      const to = Math.min(Math.floor(Date.now() / 1000), range === 'month' ? Math.floor(new Date(end + 'T00:00:00') / 1000) : Infinity);
      summary(data.days.filter((d) => d.day >= first && d.day < end), clipOuts(periodOuts, from, to), 2);
      $('#st-title').textContent = range === 'month' ? monthLabel(month) : range === '12m' ? 'Энергия по месяцам' : 'Энергия по дням';
      $('#st-legend').innerHTML = SER.map((s) => `<label><input type="checkbox" data-k="${s.k}"${hidden.has(s.k) ? '' : ' checked'}><i style="background:${s.color}"></i>${s.label}</label>`).join('');
      document.querySelectorAll('#st-legend [data-k]').forEach((i) => (i.onchange = () => {
        i.checked ? hidden.delete(i.dataset.k) : hidden.add(i.dataset.k); save('sbStatsHidden', [...hidden]); bars();
      }));
      bars();
    }
    table();
  }

  function controls() {
    $('#st-ranges').innerHTML = RANGES.map(([k, l]) => `<button class="btn${k === range ? ' on' : ''}" data-r="${k}">${l}</button>`).join('');
    document.querySelectorAll('#st-ranges button').forEach((b) => (b.onclick = () => {
      range = b.dataset.r; save('sbStatsRange', range); controls(); fetchData();
    }));
  }

  function monthControls() {
    const go = (m) => { if (m && m <= thisMonth()) { month = m; save('sbStatsMonth', month); fetchData(); } };
    $('#sm-month').max = thisMonth();
    $('#sm-month').onchange = () => go($('#sm-month').value);
    $('#sm-prev').onclick = () => go(prevMonth(month));
    $('#sm-next').onclick = () => go(nextMonth(month));
    $('#sm-this').onclick = () => go(thisMonth());
  }

  function dayControls() {
    const go = (key) => { if (key && key <= iso(new Date())) { day = key; save('sbStatsDay', day); loadDay(); } };
    $('#sd-date').max = iso(new Date());
    $('#sd-date').onchange = () => go($('#sd-date').value);
    $('#sd-prev').onclick = () => go(shift(day, -1));
    $('#sd-next').onclick = () => go(shift(day, 1));
    $('#sd-today').onclick = () => go(iso(new Date()));
  }

  let periodOuts = [];
  async function fetchData() {
    const r = rng(range);
    const now = Math.floor(Date.now() / 1000);
    let days = r[2], from = now - r[2] * 86400, to = now;
    if (range === 'month') {
      $('#sm-month').value = month;
      $('#sm-next').disabled = month >= thisMonth();
      from = Math.floor(new Date(month + '-01T00:00:00') / 1000);
      to = Math.min(now, Math.floor(new Date(nextMonth(month) + '-01T00:00:00') / 1000));
      days = Math.max(1, Math.ceil((now - from) / 86400) + 1);
    }
    const [d, o] = await Promise.all([
      api('/api/energy?days=' + days).catch(() => ({ days: [], today: '' })),
      api(`/api/outages?from=${from}&to=${to}`).catch(() => ({ items: [] })),
    ]);
    data = d; periodOuts = o.items || [];
    cur = d.currency || cur; hasTariff = !!d.tariffs;
    render();
  }

  page('stats', { show() {
    day = day || load('sbStatsDay', null) || iso(new Date());
    if (day > iso(new Date())) day = iso(new Date());
    month = month || load('sbStatsMonth', null) || thisMonth();
    if (month > thisMonth()) month = thisMonth();
    controls(); dayControls(); monthControls(); fetchData();
  } });
  window.addEventListener('resize', () => { if (!$('#page-stats').hidden) render(); });
})();
