// Battery / cells: per-cell bars with deviation from the average, internal
// resistance, temperatures, BMS flags and errors, and a cell-voltage history.

(() => {
  const { $, v, num, esc, page, entity, api } = SB;
  const N = 8;                       // 8S pack
  const LO = 2.9, HI = 3.65;          // LiFePO4 bar scale, V
  const COLORS = ['#4ea3ff', '#3fb950', '#e3b341', '#db61a2', '#f0883e', '#a371f7', '#56d4dd', '#f85149'];
  let extrasAt = 0, chartAt = 0;

  function bars() {
    const cells = [...Array(N)].map((_, i) => v('bms', 'c' + (i + 1)));
    const have = cells.filter((x) => x !== undefined);
    if (!have.length) {
      $('#cells').innerHTML = '<div class="empty">Плата BMS пока не присылает напряжения ячеек — нужна новая прошивка (вкладка «Платы»).</div>';
      return;
    }
    const avg = v('bms', 'cell_avg') ?? have.reduce((a, b) => a + b, 0) / have.length;
    const mx = Math.max(...have), mn = Math.min(...have);
    $('#cells').innerHTML = cells.map((c, i) => {
      if (c === undefined) return `<div class="cell"><span class="ci">${i + 1}</span><span class="cb"></span><span class="cv">—</span></div>`;
      const w = Math.max(2, Math.min(100, ((c - LO) / (HI - LO)) * 100));
      const cls = have.length > 1 && c === mx ? 'max' : have.length > 1 && c === mn ? 'min' : '';
      const dev = (c - avg) * 1000;
      const r = v('bms', 'r' + (i + 1));
      return `<div class="cell ${cls}"><span class="ci">${i + 1}</span>
        <span class="cb"><i style="width:${w.toFixed(1)}%"></i></span>
        <span class="cv">${c.toFixed(3)} В</span>
        <span class="cd">${dev >= 0 ? '+' : ''}${dev.toFixed(0)} мВ</span>
        <span class="cr">${r !== undefined ? (r * 1000).toFixed(0) + ' мОм' : ''}</span></div>`;
    }).join('');
  }

  function facts() {
    const flag = (k) => v('bms', k) === undefined ? '—' : v('bms', k) >= 0.5 ? 'да' : 'нет';
    const rows = [
      ['Заряд', num(v('bms', 'soc'), 0) + ' %'],
      ['Напряжение', num(v('bms', 'bms_v'), 2) + ' В'],
      ['Ток', num(v('bms', 'bms_a'), 2) + ' А'],
      ['Мощность', num(v('bms', 'bms_w'), 0) + ' Вт'],
      ['Остаток', num(v('bms', 'cap_ah'), 1) + ' Ач'],
      ['Среднее ячейки', num(v('bms', 'cell_avg'), 3) + ' В'],
      ['Разброс', v('bms', 'cell_delta') !== undefined ? num(v('bms', 'cell_delta') * 1000, 0) + ' мВ' : '—'],
      ['Мин / макс', (v('bms', 'cell_min_i') ? '№' + num(v('bms', 'cell_min_i'), 0) : '—') + ' / ' + (v('bms', 'cell_max_i') ? '№' + num(v('bms', 'cell_max_i'), 0) : '—')],
      ['Балансировка', flag('bal') + (v('bms', 'bal_a') ? ' · ' + num(v('bms', 'bal_a'), 2) + ' А' : '')],
      ['Заряд разрешён', flag('chg')],
      ['Разряд разрешён', flag('dsg')],
      ['Темп. 1 / 2', num(v('bms', 't1'), 1) + ' / ' + num(v('bms', 't2'), 1) + ' °C'],
      ['Темп. MOS', num(v('bms', 't_mos'), 1) + ' °C'],
    ];
    $('#cell-facts').innerHTML = rows.map(([k, x]) => `<div><span>${k}</span><b>${x}</b></div>`).join('');
  }

  // Errors / runtime are text on the board itself — fetched on demand, at most every 30 s.
  async function extras() {
    if (Date.now() - extrasAt < 30000) return;
    extrasAt = Date.now();
    const get = async (p) => { try { return (await entity('bms', p)).value; } catch (e) { return null; } };
    const [errs, rt] = await Promise.all([get('text_sensor/Ошибки ERR'), get('text_sensor/Наработка RT')]);
    $('#bms-errors').innerHTML = errs === null ? '<span class="muted">нет связи с платой</span>'
      : errs && errs.trim() ? `<b class="bad">${esc(errs)}</b>` : '<span class="ok">ошибок нет</span>';
    $('#bms-runtime').textContent = rt || '—';
  }

  async function chart() {
    if (Date.now() - chartAt < 60000) return;
    chartAt = Date.now();
    const to = Math.floor(Date.now() / 1000), from = to - 24 * 3600;
    const series = await Promise.all([...Array(N)].map(async (_, i) => {
      try {
        const d = await api(`/api/series?device=bms&metric=c${i + 1}&from=${from}&to=${to}&points=300`);
        return { ts: d.ts, v: d.v, color: COLORS[i], label: 'Ячейка ' + (i + 1) };
      } catch (e) { return { ts: [], v: [] }; }
    }));
    Plot($('#cells-chart'), { series, unit: 'В' });
    $('#cells-legend').innerHTML = COLORS.map((c, i) => `<span><i style="background:${c}"></i>${i + 1}</span>`).join('');
  }

  // Compact tile grid (min blue, max red), used on this page and on «Потоки».
  function tiles(el) {
    if (!el) return;
    const cells = [...Array(N)].map((_, i) => v('bms', 'c' + (i + 1)));
    const have = cells.filter((x) => x !== undefined);
    const mx = Math.max(...have), mn = Math.min(...have);
    const d = v('bms', 'cell_delta'), t = v('bms', 't1'), bal = v('bms', 'bal') >= 0.5;
    const head = `<div class="chead"><b>▯ Ячейки батареи</b><div>` +
      `<span title="разброс">↕ ${d !== undefined ? num(d * 1000, 0) + ' mV' : '—'}</span>` +
      `<span title="температура">🌡 ${t !== undefined ? num(t, 1) + ' °C' : '—'}</span>` +
      `<span title="балансировка">${bal ? '⚖ баланс' : '⚖ —'}</span></div></div>`;
    el.innerHTML = head + (have.length ? `<div class="ctiles">` + cells.map((c, i) => {
      const cls = have.length > 1 && mx !== mn ? (c === mx ? ' max' : c === mn ? ' min' : '') : '';
      return `<div class="ctile${cls}"><small>${i + 1}</small><b>${c === undefined ? '—' : c.toFixed(3)}</b></div>`;
    }).join('') + '</div>' : '<div class="empty">Нет данных по ячейкам.</div>');
  }
  SB.cellTiles = tiles;

  page('cells', {
    show() { extrasAt = 0; chartAt = 0; },
    tick() { tiles($('#cells-tiles')); bars(); facts(); extras(); chart(); },
  });
})();
