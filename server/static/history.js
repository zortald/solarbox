// History: pick metrics + a range, one chart per metric (each auto-scaled).

(() => {
  const { $, api, page } = SB;
  const META = {
    // inverter
    ac_out_w: { label: 'Нагрузка', unit: 'Вт', color: '#4ea3ff' },
    pv_w: { label: 'Солнце', unit: 'Вт', color: '#f5b041' },
    pv_v: { label: 'Напряжение PV', unit: 'В', color: '#e3b341' },
    grid_v: { label: 'Сеть', unit: 'В', color: '#9aa5b1' },
    grid_hz: { label: 'Частота сети', unit: 'Гц', color: '#9aa5b1' },
    out_v: { label: 'Выход', unit: 'В', color: '#58a6ff' },
    load_pct: { label: 'Загрузка инвертора', unit: '%', color: '#db61a2' },
    batt_v: { label: 'АКБ (инвертор)', unit: 'В', color: '#3fb950' },
    batt_chg_a: { label: 'Ток заряда (инвертор)', unit: 'А', color: '#3fb950' },
    batt_dis_a: { label: 'Ток разряда (инвертор)', unit: 'А', color: '#f85149' },
    temp: { label: 'Темп. инвертора', unit: '°C', color: '#f0883e' },
    rssi: { label: 'Wi-Fi', unit: 'дБм', color: '#8b98a8' },
    uptime: { label: 'Аптайм', unit: 'с', color: '#8b98a8' },
    // bms
    soc: { label: 'Заряд', unit: '%', color: '#3fb950' },
    bms_v: { label: 'Напряжение АКБ', unit: 'В', color: '#58a6ff' },
    bms_a: { label: 'Ток АКБ', unit: 'А', color: '#e3b341' },
    bms_w: { label: 'Мощность АКБ', unit: 'Вт', color: '#4ea3ff' },
    cap_ah: { label: 'Остаток', unit: 'Ач', color: '#3fb950' },
    t1: { label: 'Темп. 1', unit: '°C', color: '#f0883e' },
    t2: { label: 'Темп. 2', unit: '°C', color: '#f0883e' },
    t_mos: { label: 'Темп. MOS', unit: '°C', color: '#f85149' },
    cell_min: { label: 'Ячейка мин', unit: 'В', color: '#8b98a8' },
    cell_max: { label: 'Ячейка макс', unit: 'В', color: '#58a6ff' },
    cell_avg: { label: 'Ячейка средн.', unit: 'В', color: '#3fb950' },
    cell_delta: { label: 'Разброс ячеек', unit: 'В', color: '#db61a2' },
    bal_a: { label: 'Ток балансировки', unit: 'А', color: '#a371f7' },
  };
  for (let i = 1; i <= 8; i++) {
    META['c' + i] = { label: 'Ячейка ' + i, unit: 'В', color: '#56d4dd' };
    META['r' + i] = { label: 'Сопр. ячейки ' + i, unit: 'Ом', color: '#a371f7' };
  }
  const HIDDEN = new Set(['chg', 'dsg', 'bal', 'cell_min_i', 'cell_max_i', 'uptime']);
  const DEV = { inverter: 'Инвертор', bms: 'АКБ / BMS' };
  const RANGES = [['1ч', 3600], ['6ч', 21600], ['24ч', 86400], ['7д', 604800], ['30д', 2592000]];
  // config.json → ui.metrics overrides/extends these labels live.
  const meta = (m) => Object.assign({ label: m, unit: '', color: '#4ea3ff' }, META[m], SB.cfg.ui.metrics?.[m]);
  SB.metricLabel = (m) => meta(m).label;
  const state = { range: 21600, selected: new Set(['inverter/pv_w', 'inverter/ac_out_w', 'bms/soc', 'bms/bms_w']), catalog: [] };
  let timer = null;

  function ranges() {
    $('#h-ranges').innerHTML = RANGES.map(([l, s]) => `<button class="btn${s === state.range ? ' on' : ''}" data-s="${s}">${l}</button>`).join('');
    document.querySelectorAll('#h-ranges button').forEach((b) => (b.onclick = () => { state.range = +b.dataset.s; ranges(); charts(); }));
  }

  function picker() {
    const by = {};
    for (const it of state.catalog) if (!HIDDEN.has(it.metric)) (by[it.device] ||= []).push(it.metric);
    $('#h-metrics').innerHTML = Object.keys(by).sort().map((dev) => `<details${dev === 'inverter' ? ' open' : ''}><summary>${DEV[dev] || dev}</summary>` +
      by[dev].map((m) => { const k = dev + '/' + m; return `<label><input type="checkbox" data-k="${k}"${state.selected.has(k) ? ' checked' : ''}>${meta(m).label}</label>`; }).join('') +
      '</details>').join('');
    document.querySelectorAll('#h-metrics input').forEach((i) => (i.onchange = () => { i.checked ? state.selected.add(i.dataset.k) : state.selected.delete(i.dataset.k); charts(); }));
  }

  async function charts() {
    const host = $('#h-charts'), keys = [...state.selected];
    if (!keys.length) { host.innerHTML = '<div class="empty">Выбери показатели.</div>'; return; }
    const to = Math.floor(Date.now() / 1000), from = to - state.range;
    host.innerHTML = keys.map((k) => { const m = meta(k.split('/')[1]); return `<div class="chart" data-k="${k}"><div class="ch"><b>${m.label}</b><span data-cur></span></div><canvas></canvas></div>`; }).join('');
    for (const k of keys) {
      const [dev, m] = k.split('/'), mt = meta(m), el = host.querySelector(`[data-k="${CSS.escape(k)}"]`);
      try {
        const d = await api(`/api/series?device=${dev}&metric=${m}&from=${from}&to=${to}`);
        Plot(el.querySelector('canvas'), { series: [{ ts: d.ts, v: d.v, color: mt.color }], unit: mt.unit });
        if (d.v.length) el.querySelector('[data-cur]').textContent = SB.num(d.v[d.v.length - 1]) + ' ' + mt.unit;
      } catch (e) { el.querySelector('[data-cur]').textContent = 'ошибка'; }
    }
  }

  page('history', {
    async show() {
      try { state.catalog = (await api('/api/catalog')).items; } catch (e) { state.catalog = []; }
      ranges(); picker(); charts();
      timer = setInterval(charts, 60000);
    },
    hide() { clearInterval(timer); },
  });
  window.addEventListener('resize', () => { if (!$('#page-history').hidden) charts(); });
})();
