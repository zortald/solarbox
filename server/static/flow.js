// Energy flow: PV (top) → inverter (centre) → load (right); grid (left) and
// battery (bottom) flow either way. The SVG is built once; updates only change
// texts and the direction/visibility of the moving dashes, so the animation
// never restarts and doesn't stutter.

(() => {
  const { $, v, num, fresh, page } = SB;
  // id, path drawn source→inverter (load: inverter→load), colour
  const LINKS = [
    ['pv', 'M180 74 V118', '#f5b041'],
    ['grid', 'M76 150 H140', '#9aa5b1'],
    ['load', 'M220 150 H284', '#4ea3ff'],
    ['batt', 'M180 226 V182', '#3fb950'],
  ];
  let built = false;

  function node(id, x, y, title, col) {
    return `<g class="fnode" id="fn-${id}">
      <rect x="${x - 52}" y="${y - 30}" width="104" height="60" rx="12" style="stroke:${col}"/>
      <text x="${x}" y="${y - 12}" class="ft">${title}</text>
      <text x="${x}" y="${y + 8}" class="fv" id="fv-${id}">—</text>
      <text x="${x}" y="${y + 23}" class="fs" id="fs-${id}"></text>
    </g>`;
  }

  function build() {
    const svg = $('#flow-svg');
    svg.innerHTML =
      LINKS.map(([id, d, c]) => `<path d="${d}" class="fline" style="stroke:${c}"/>` +
        `<path d="${d}" class="fdash" id="fd-${id}" style="stroke:${c}"/>`).join('') +
      `<g class="fnode inv"><rect x="140" y="118" width="80" height="64" rx="14"/>
        <text x="180" y="146" class="ft">Инвертор</text><text x="180" y="166" class="fs" id="fs-inv"></text></g>` +
      node('pv', 180, 44, 'Солнце', '#f5b041') +
      node('grid', 46, 150, 'Сеть', '#9aa5b1') +
      node('load', 314, 150, 'Дом', '#4ea3ff') +
      node('batt', 180, 256, 'АКБ', '#3fb950') +
      `<line id="grid-x" x1="10" y1="124" x2="82" y2="176" class="fcross"/>`;
    built = true;
  }

  const set = (id, t) => { const e = document.getElementById(id); if (e && e.textContent !== t) e.textContent = t; };
  function dash(id, watts, reverse) {
    const e = document.getElementById('fd-' + id);
    const on = Math.abs(watts) > 5;
    e.style.display = on ? '' : 'none';
    e.classList.toggle('rev', !!reverse);
    // Faster dashes for more power, in a few steps so the speed doesn't jitter.
    const sp = Math.abs(watts) > 1500 ? 'f3' : Math.abs(watts) > 400 ? 'f2' : 'f1';
    e.classList.remove('f1', 'f2', 'f3'); e.classList.add(sp);
  }

  function render() {
    if (!built) build();
    const pv = v('inverter', 'pv_w') ?? 0;
    const load = v('inverter', 'ac_out_w') ?? 0;
    // Battery power: BMS is the accurate source (+ = charging); fall back to the inverter.
    let batt = v('bms', 'bms_w');
    if (batt === undefined) {
      const bv = v('inverter', 'batt_v'), c = v('inverter', 'batt_chg_a'), d = v('inverter', 'batt_dis_a');
      batt = bv !== undefined ? bv * ((c ?? 0) - (d ?? 0)) : 0;
    }
    const gv = v('inverter', 'grid_v');
    const gridUp = gv !== undefined && gv > 150;
    // The inverter doesn't report grid power: estimate it from the balance.
    const grid = gridUp ? Math.max(0, load + Math.max(batt, 0) - pv) : 0;

    set('fv-pv', num(pv, 0) + ' Вт');
    set('fs-pv', v('inverter', 'pv_v') !== undefined ? num(v('inverter', 'pv_v'), 0) + ' В' : '');
    set('fv-load', num(load, 0) + ' Вт');
    set('fs-load', v('inverter', 'load_pct') !== undefined ? num(v('inverter', 'load_pct'), 0) + '% мощности' : '');
    set('fv-grid', gridUp ? '≈ ' + num(grid, 0) + ' Вт' : 'нет');
    set('fs-grid', gridUp ? num(gv, 0) + ' В · ' + num(v('inverter', 'grid_hz'), 1) + ' Гц' : 'сети нет');
    const soc = v('bms', 'soc');
    set('fv-batt', (soc !== undefined ? num(soc, 0) + '% · ' : '') + num(Math.abs(batt), 0) + ' Вт');
    set('fs-batt', batt > 5 ? 'заряд ' + num(v('bms', 'bms_a'), 1) + ' А' : batt < -5 ? 'разряд ' + num(Math.abs(v('bms', 'bms_a') ?? 0), 1) + ' А' : 'покой');
    set('fs-inv', (v('inverter', 'temp') !== undefined ? num(v('inverter', 'temp'), 0) + ' °C' : ''));
    document.getElementById('grid-x').style.display = gridUp ? 'none' : '';
    document.getElementById('fn-grid').classList.toggle('off', !gridUp);

    dash('pv', pv, false);
    dash('grid', grid, false);
    dash('load', load, false);
    dash('batt', batt, batt > 0); // drawn battery→inverter; charging runs the other way

    document.getElementById('flow-svg').classList.toggle('stale', !fresh('inverter'));

    // Key numbers under the diagram.
    const rows = [
      ['Выход', num(v('inverter', 'out_v'), 0) + ' В'],
      ['АКБ по инвертору', num(v('inverter', 'batt_v'), 2) + ' В'],
      ['АКБ по BMS', num(v('bms', 'bms_v'), 2) + ' В'],
      ['Остаток', num(v('bms', 'cap_ah'), 0) + ' Ач'],
      ['Разброс ячеек', v('bms', 'cell_delta') !== undefined ? num(v('bms', 'cell_delta') * 1000, 0) + ' мВ' : '—'],
      ['Темп. АКБ', num(v('bms', 't1'), 1) + ' °C'],
    ];
    $('#flow-facts').innerHTML = rows.map(([k, x]) => `<div><span>${k}</span><b>${x}</b></div>`).join('');
  }

  // ---- charts under the diagram ----
  const SERIES = [
    ['inverter', 'pv_w', 'Солнце', '#f5b041'],
    ['inverter', 'ac_out_w', 'Дом', '#4ea3ff'],
    ['bms', 'bms_w', 'АКБ (+ заряд / − разряд)', '#3fb950'],
  ];
  const RANGES = [['6ч', 21600], ['24ч', 86400], ['7д', 604800]];
  let range = 86400, chartsAt = 0;

  function rangeButtons() {
    $('#fc-ranges').innerHTML = RANGES.map(([l, s]) => `<button class="btn${s === range ? ' on' : ''}" data-s="${s}">${l}</button>`).join('');
    document.querySelectorAll('#fc-ranges button').forEach((b) => (b.onclick = () => { range = +b.dataset.s; rangeButtons(); chartsAt = 0; charts(); }));
    $('#fc-legend').innerHTML = SERIES.map(([, , l, c]) => `<span><i style="background:${c}"></i>${l}</span>`).join('');
  }

  async function charts() {
    if (Date.now() - chartsAt < 60000) return;
    chartsAt = Date.now();
    const to = Math.floor(Date.now() / 1000), from = to - range;
    const get = async (dev, m) => { try { return await SB.api(`/api/series?device=${dev}&metric=${m}&from=${from}&to=${to}&points=400`); } catch (e) { return { ts: [], v: [] }; } };
    const power = await Promise.all(SERIES.map(async ([dev, m, label, color]) => ({ ...(await get(dev, m)), color, label })));
    Plot($('#fc-power'), { series: power, unit: 'Вт' });
    const soc = await get('bms', 'soc');
    Plot($('#fc-soc'), { series: [{ ...soc, color: '#3fb950' }], unit: '%' });
  }

  let resized;
  window.addEventListener('resize', () => { clearTimeout(resized); resized = setTimeout(() => { if (!$('#page-flow').hidden) { chartsAt = 0; charts(); } }, 200); });

  page('flow', {
    show() { rangeButtons(); chartsAt = 0; },
    tick() { render(); charts(); },
  });
})();
