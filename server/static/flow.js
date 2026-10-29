// Energy flow in the layout of the Sunsynk Power Flow card: PV on top, grid on
// the left, load on the right, battery at the bottom, inverter in the middle,
// value boxes sitting on the lines and today's energy next to each source.
// The SVG is built once; updates only change texts, colours, the battery level
// and the direction/visibility of the moving dashes (no animation restarts).
// Today's energy is integrated in the browser from the stored history.

(() => {
  const { $, v, num, fresh, page, api } = SB;
  const C = { pv: '#f5b041', load: '#ff9eb5', batt: '#ff9eb5', grid: '#4ea3ff' };
  // Lines run source → inverter (pv, grid, batt) or inverter → load.
  const LINKS = [
    ['pv', 'M250 96 V150'],
    ['grid', 'M84 205 H215'],
    ['load', 'M285 205 H418'],
    ['batt', 'M250 336 V245'],
  ];
  let built = false;

  const T = (id, x, y, cls, anchor = 'middle', txt = '') => `<text id="${id}" x="${x}" y="${y}" class="${cls}" text-anchor="${anchor}">${txt}</text>`;
  const box = (id, x, y, col) => `<rect id="fb-${id}" x="${x}" y="${y}" width="84" height="32" rx="8" class="fbox" style="stroke:${col}"/>` + T('fv-' + id, x + 42, y + 21, 'fv');

  // Simple line-art icons.
  const ICON = {
    pv: (x, y) => `<g class="ficon" style="stroke:${C.pv}" transform="translate(${x} ${y})">
      <path d="M0 18 L8 0 H34 L26 18 Z M4 9 H30 M13 0 L9 18 M22 0 L18 18"/><path d="M38 2 L32 13 H39 L34 24" class="bolt"/></g>`,
    grid: (x, y) => `<g class="ficon" id="fi-grid" style="stroke:${C.grid}" transform="translate(${x} ${y})">
      <path d="M18 0 L6 50 M18 0 L30 50 M10 34 H26 M12 24 H24 M14 14 H22 M0 12 H36 M3 22 H33 M10 34 L24 24 M26 34 L12 24"/></g>`,
    load: (x, y) => `<g class="ficon" style="stroke:${C.load}" transform="translate(${x} ${y})">
      <path d="M0 22 L24 2 L48 22 M6 18 V48 H42 V18 M18 48 V32 H30 V48"/></g>`,
  };

  function build() {
    $('#flow-svg').innerHTML =
      LINKS.map(([id, d]) => `<path d="${d}" class="fline" style="stroke:${C[id]}"/><path d="${d}" class="fdash" id="fd-${id}" style="stroke:${C[id]}"/>`).join('') +
      // PV header + box
      // PV row: icon + "today" label to the left of the box, voltage to the right
      ICON.pv(56, 66) + T('ft-pv', 198, 74, 'flab', 'end', 'СОЛНЦЕ СЕГОДНЯ') + T('fe-pv', 198, 95, 'fe', 'end') +
      box('pv', 208, 64, C.pv) + T('fs-pv', 300, 84, 'fs', 'start') +
      // grid
      ICON.grid(32, 178) + `<line id="grid-x" x1="26" y1="176" x2="76" y2="234" class="fcross"/>` +
      box('grid', 108, 189, C.grid) + T('fl-grid', 150, 242, 'flab', 'middle', 'Из сети сегодня') + T('fe-grid', 150, 264, 'fe') +
      T('fg-1', 208, 168, 'fs', 'end') + T('fg-2', 208, 183, 'fs', 'end') +
      // inverter body
      `<rect x="215" y="150" width="70" height="95" rx="6" class="finv"/>
       <rect x="229" y="162" width="42" height="26" rx="3" class="fscr"/>
       <circle id="fi-dot" cx="226" cy="257" r="4" class="fdot"/>` +
      T('fi-temp', 250, 234, 'fsm') +
      T('fo-1', 292, 160, 'fs', 'start') + T('fo-2', 292, 174, 'fs', 'start') + T('fo-3', 292, 188, 'fs', 'start') +
      // load
      box('load', 308, 189, C.load) + ICON.load(426, 178) +
      T('fl-load', 350, 242, 'flab', 'middle', 'Потребление сегодня') + T('fe-load', 350, 264, 'fe') +
      // battery
      box('batt', 208, 278, C.batt) +
      `<g id="batt-icon"><rect x="228" y="346" width="44" height="74" rx="6" class="fbat" style="stroke:${C.batt}"/>
       <rect x="241" y="339" width="18" height="7" rx="2" class="fbatcap" style="fill:${C.batt}"/>
       ${[0, 1, 2, 3, 4].map((i) => `<rect id="bl-${i}" x="234" y="${405 - i * 13}" width="32" height="10" rx="2" class="blev"/>`).join('')}</g>` +
      T('fb-temp', 222, 338, 'fs', 'end') +
      T('fl-chg', 194, 352, 'flab', 'end', 'Заряд сегодня') + T('fe-chg', 194, 373, 'fe', 'end') +
      T('fl-dis', 194, 395, 'flab', 'end', 'Разряд сегодня') + T('fe-dis', 194, 416, 'fe', 'end') +
      T('fb-kwh', 284, 352, 'fs', 'start') + T('fb-soc', 284, 380, 'fsoc', 'start') +
      T('fb-eta1', 284, 398, 'flab', 'start') + T('fb-eta2', 284, 416, 'fs', 'start');
    built = true;
  }

  const set = (id, s) => { const e = document.getElementById(id); if (e && e.textContent !== s) e.textContent = s; };
  function dash(id, watts, reverse) {
    const e = document.getElementById('fd-' + id), on = Math.abs(watts) > 5;
    e.style.display = on ? '' : 'none';
    e.classList.toggle('rev', !!reverse);
    const sp = Math.abs(watts) > 1500 ? 'f3' : Math.abs(watts) > 400 ? 'f2' : 'f1';
    if (!e.classList.contains(sp)) { e.classList.remove('f1', 'f2', 'f3'); e.classList.add(sp); }
    document.getElementById('fb-' + id).classList.toggle('idle', !on);
  }
  const W = (x) => (Math.abs(x) >= 10000 ? (x / 1000).toFixed(1) + ' kW' : num(x, 0) + ' W');
  const kwh = (x) => (x === undefined ? '—' : x.toFixed(1).replace('.', ',') + ' kWh');
  function eta(hours, toward) {
    if (!isFinite(hours) || hours <= 0 || hours > 24 * 30) return ['', ''];
    const at = new Date(Date.now() + hours * 3600e3);
    const hh = String(at.getHours()).padStart(2, '0'), mm = String(at.getMinutes()).padStart(2, '0');
    const d = Math.floor(hours / 24), h = Math.floor(hours % 24), m = Math.round((hours * 60) % 60);
    return [`ВРЕМЯ ДО ${toward} @${hh}:${mm}`, (d ? d + ' д ' : '') + (d || h ? h + ' ч ' : '') + m + ' мин'];
  }

  function power() {
    const pv = v('inverter', 'pv_w') ?? 0, load = v('inverter', 'ac_out_w') ?? 0;
    let batt = v('bms', 'bms_w');
    if (batt === undefined) {
      const bv = v('inverter', 'batt_v'), c = v('inverter', 'batt_chg_a'), d = v('inverter', 'batt_dis_a');
      batt = bv !== undefined ? bv * ((c ?? 0) - (d ?? 0)) : 0;
    }
    const gv = v('inverter', 'grid_v'), gridUp = gv !== undefined && gv > 150;
    // The inverter doesn't report grid power: estimate it from the balance
    // load + battery (signed, + = charging) − PV. Discharge must count too —
    // in SBU mode the grid is present but the battery carries the load.
    // Small residuals are the inverter's own losses, not grid draw.
    const g = load + batt - pv;
    return { pv, load, batt, gv, gridUp, grid: gridUp && g > 40 ? g : 0 };
  }

  function render() {
    if (!built) build();
    const p = power(), E = today;
    // PV
    set('fv-pv', W(p.pv));
    set('fs-pv', v('inverter', 'pv_v') !== undefined ? num(v('inverter', 'pv_v'), 0) + ' V' : '');
    set('fe-pv', kwh(E.pv));
    // grid
    set('fv-grid', p.gridUp ? '≈ ' + W(p.grid) : 'нет');
    set('fe-grid', kwh(E.grid));
    set('fg-1', p.gridUp ? num(p.gv, 0) + ' V' : out_()?.current ? 'нет ' + span(Date.now() / 1000 - out_().current.start) : 'сети нет');
    set('fg-2', p.gridUp ? num(v('inverter', 'grid_hz'), 1) + ' Hz' : '');
    document.getElementById('grid-x').style.display = p.gridUp ? 'none' : '';
    document.getElementById('fi-grid').classList.toggle('off', !p.gridUp);
    // inverter / output
    const ov = v('inverter', 'out_v');
    set('fo-1', ov !== undefined ? num(ov, 0) + ' V' : '');
    set('fo-2', ov ? num(p.load / ov, 1) + ' A' : '');
    set('fo-3', v('inverter', 'load_pct') !== undefined ? num(v('inverter', 'load_pct'), 0) + ' %' : '');
    set('fi-temp', v('inverter', 'temp') !== undefined ? num(v('inverter', 'temp'), 0) + '°' : '');
    document.getElementById('fi-dot').classList.toggle('bad', !fresh('inverter'));
    // load
    set('fv-load', W(p.load));
    set('fe-load', kwh(E.load));
    // battery
    const soc = v('bms', 'soc'), a = v('bms', 'bms_a'), cap = v('bms', 'cap_ah'), bv = v('bms', 'bms_v') ?? v('inverter', 'batt_v');
    set('fv-batt', W(Math.abs(p.batt)));
    set('fb-temp', v('bms', 't1') !== undefined ? num(v('bms', 't1'), 0) + '°' : '');
    set('fe-chg', kwh(E.chg));
    set('fe-dis', kwh(E.dis));
    set('fb-kwh', cap !== undefined && bv ? (cap * bv / 1000).toFixed(2) + ' kWh' : '');
    set('fb-soc', soc !== undefined ? num(soc, 0) + '%' : '—');
    let e = ['', ''];
    if (a !== undefined && cap !== undefined && Math.abs(a) > 0.3) {
      if (a < 0) e = eta(cap * 0.99 / -a, '1%');                       // ~to empty
      else if (soc) e = eta((cap / (soc / 100) - cap) / a, '100%');   // to full
    }
    set('fb-eta1', e[0]); set('fb-eta2', e[1] || (Math.abs(a ?? 0) <= 0.3 ? 'покой' : ''));
    const lvl = soc === undefined ? 0 : Math.ceil(soc / 20);
    const col = soc === undefined ? '#3a4552' : soc < 20 ? 'var(--bad)' : soc < 40 ? 'var(--warn)' : 'var(--ok)';
    for (let i = 0; i < 5; i++) document.getElementById('bl-' + i).style.fill = i < lvl ? col : '#2a3542';

    dash('pv', p.pv, false);
    dash('grid', p.grid, false);
    dash('load', p.load, false);
    dash('batt', p.batt, p.batt > 0); // drawn battery→inverter; charging flows the other way
    document.getElementById('flow-svg').classList.toggle('stale', !fresh('inverter'));

    const rows = [
      ['АКБ по BMS / инвертору', num(v('bms', 'bms_v'), 2) + ' / ' + num(v('inverter', 'batt_v'), 2) + ' В'],
      ['Ток АКБ', num(a, 1) + ' А'],
      ['Остаток', num(cap, 0) + ' Ач'],
      ['Разброс ячеек', v('bms', 'cell_delta') !== undefined ? num(v('bms', 'cell_delta') * 1000, 0) + ' мВ' : '—'],
      ['Темп. АКБ / MOS', num(v('bms', 't1'), 1) + ' / ' + num(v('bms', 't_mos'), 1) + ' °C'],
      ['Заряд / разряд (инв.)', num(v('inverter', 'batt_chg_a'), 0) + ' / ' + num(v('inverter', 'batt_dis_a'), 0) + ' А'],
    ];
    $('#flow-facts').innerHTML = rows.map(([k, x]) => `<div><span>${k}</span><b>${x}</b></div>`).join('');    SB.cellTiles?.($('#flow-cells'));
  }

  // Outage data comes from outages.js (shared with the «Отключения» page).
  const out_ = () => SB.outages?.data();
  const span = (s) => SB.outages.span(s);

  // ---- today's energy, integrated from history (refreshed every 2 min) ----
  let today = {}, todayAt = 0;
  async function energy() {
    if (Date.now() - todayAt < 120000) return;
    todayAt = Date.now();
    const mid = new Date(); mid.setHours(0, 0, 0, 0);
    const from = Math.floor(mid / 1000), to = Math.floor(Date.now() / 1000);
    const get = async (d, m) => { try { return await api(`/api/series?device=${d}&metric=${m}&from=${from}&to=${to}&points=288`); } catch (e) { return { ts: [], v: [] }; } };
    const [pv, load, batt, gv] = await Promise.all([get('inverter', 'pv_w'), get('inverter', 'ac_out_w'), get('bms', 'bms_w'), get('inverter', 'grid_v')]);
    // Each point is a bucket average; weight it by the gap to the next one (capped, so gaps don't count).
    const integ = (s, f = (x) => x) => {
      let wh = 0;
      for (let i = 0; i < s.ts.length; i++) wh += f(s.v[i], s.ts[i]) * Math.min((s.ts[i + 1] ?? to) - s.ts[i], 900) / 3600;
      return wh / 1000;
    };
    const at = (s) => { const m = new Map(); s.ts.forEach((t, i) => m.set(t, s.v[i])); return m; };
    const P = at(pv), B = at(batt), G = at(gv);
    today = {
      pv: integ(pv), load: integ(load),
      chg: integ(batt, (x) => Math.max(x, 0)), dis: integ(batt, (x) => Math.max(-x, 0)),
      grid: integ(load, (x, t) => { const g = x + (B.get(t) ?? 0) - (P.get(t) ?? 0); return (G.get(t) ?? 0) > 150 && g > 40 ? g : 0; }),
    };
  }

  // ---- one combined chart; legend checkboxes hide series (remembered) ----
  const CHART = [
    { k: 'pv', d: 'inverter', m: 'pv_w', label: 'Солнце', unit: 'Вт', color: '#f5b041' },
    { k: 'load', d: 'inverter', m: 'ac_out_w', label: 'Дом', unit: 'Вт', color: '#ff9eb5' },
    { k: 'batt', d: 'bms', m: 'bms_w', label: 'АКБ ±', unit: 'Вт', color: '#3fb950' },
    { k: 'soc', d: 'bms', m: 'soc', label: 'Заряд АКБ', unit: '%', color: '#56d4dd', axis: 'right', fixed: [0, 100] },
  ];
  const store = (k, v) => { try { localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} };
  const load = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch (e) { return d; } };
  let period = load('sbFlowPeriod', 'today');
  if (!PERIODS[period]) period = 'today';
  const hidden = new Set(load('sbFlowHidden', []));
  const cache = {};   // k → {ts, v} for the current period
  let chartsAt = 0;

  function controls() {
    $('#fc-ranges').innerHTML = Object.entries(PERIODS).filter(([k]) => k !== '30d')
      .map(([k, [l]]) => `<button class="btn${k === period ? ' on' : ''}" data-p="${k}">${l}</button>`).join('');
    document.querySelectorAll('#fc-ranges button').forEach((b) => (b.onclick = () => {
      period = b.dataset.p; store('sbFlowPeriod', period); controls(); chartsAt = 0; charts();
    }));
    $('#fc-legend').innerHTML = CHART.map((c) => `<label><input type="checkbox" data-k="${c.k}"${hidden.has(c.k) ? '' : ' checked'}>` +
      `<i style="background:${c.color}"></i>${c.label}${c.axis === 'right' ? ' (правая шкала)' : ''}</label>`).join('');
    document.querySelectorAll('#fc-legend input').forEach((i) => (i.onchange = () => {
      i.checked ? hidden.delete(i.dataset.k) : hidden.add(i.dataset.k);
      store('sbFlowHidden', [...hidden]);
      charts(true);
    }));
  }

  async function charts(redrawOnly) {
    const [from, to] = PERIODS[period][1]();
    if (!redrawOnly || CHART.some((c) => !hidden.has(c.k) && !cache[c.k])) {
      if (redrawOnly !== true && Date.now() - chartsAt < 60000) return;
      chartsAt = Date.now();
      // Only fetch what's shown; hidden series aren't loaded at all.
      await Promise.all(CHART.filter((c) => !hidden.has(c.k)).map(async (c) => {
        try { cache[c.k] = await api(`/api/series?device=${c.d}&metric=${c.m}&from=${from}&to=${to}&points=500`); }
        catch (e) { cache[c.k] = { ts: [], v: [] }; }
      }));
      for (const c of CHART) if (hidden.has(c.k)) delete cache[c.k];
    }
    Plot($('#fc-chart'), {
      from, to,
      bands: (out_()?.items || []).map((o) => ({ from: o.start, to: o.end ?? Math.floor(Date.now() / 1000) })),
      series: CHART.filter((c) => !hidden.has(c.k) && cache[c.k]).map((c) => ({ ...cache[c.k], color: c.color, label: c.label, unit: c.unit, axis: c.axis, fixed: c.fixed })),
    });
  }

  let resized;
  window.addEventListener('resize', () => { clearTimeout(resized); resized = setTimeout(() => { if (!$('#page-flow').hidden) { chartsAt = 0; charts(); } }, 200); });

  page('flow', {
    show() { controls(); chartsAt = 0; todayAt = 0; },
    async tick() { render(); await energy(); await SB.outages.refresh(); render(); charts(); },
  });
})();
