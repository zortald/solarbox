// Inverter settings, read from the board's raw QPIRI / QFLAG and written via its
// whitelisted "inverter command" text entity (the server checks the whitelist
// too). After "Apply" we wait until the inverter itself reports the new value.

(() => {
  const { $, esc, page, entity, act, toast, requireAdmin } = SB;
  const P = 'inverter ';
  const S = {};          // raw values read from the board
  let timer = null, busy = false, built = false;

  const MODES = { P: 'Включение', S: 'Ожидание', L: 'От сети', B: 'От батареи', F: 'Авария', H: 'Энергосбережение' };
  const PRIOS = [['output_source_priority_utility', 'Сеть'], ['output_source_priority_solar', 'Солнце'], ['output_source_priority_battery', 'Батарея (SBU)']];
  const pad = (n, w) => String(n).padStart(w, '0');
  const sel = (o) => Object.entries(o);
  const QP = () => (S.qpiri || '').replace('(', '').trim().split(/\s+/);
  const QF = () => {
    const f = (S.qflag || '').replace('(', ''), e = f.indexOf('E'), d = f.indexOf('D'), on = {};
    for (let i = 0; i < f.length; i++) { const c = f[i]; if (c === 'E' || c === 'D') continue; on[c] = e >= 0 && i > e && (d < e || i < d); }
    return on;
  };
  const SETTINGS = [
    { id: 'pcp', l: 'Приоритет заряда', cur: (q) => q[17], opts: sel({ 0: 'Сначала сеть', 1: 'Сначала солнце', 2: 'Солнце + сеть', 3: 'Только солнце' }), cmd: (v) => ['PCP' + pad(v, 2)] },
    { id: 'pgr', l: 'Диапазон входа', cur: (q) => q[15], opts: sel({ 0: 'Бытовой (90–280 В)', 1: 'ИБП (170–280 В)' }), cmd: (v) => ['PGR' + pad(v, 2)] },
    { id: 'pbt', l: 'Тип батареи', cur: (q) => q[12], opts: sel({ 0: 'AGM', 1: 'Заливная', 2: 'Пользовательская', 3: 'Pylontech', 4: 'Shinheung', 5: 'Weco', 6: 'Soltaro', 7: 'BAK', 8: 'Lib', 9: 'Lic' }), cmd: (v) => ['PBT' + pad(v, 2)] },
    { id: 'much', l: 'Макс. ток заряда от сети', cur: (q) => +q[13] + '', opts: [2, 10, 20, 30, 40, 50, 60, 70, 80].map((x) => [x + '', x + ' А']), cmd: (v) => ['MUCHGC0' + pad(v, 2)] },
    { id: 'mch', l: 'Макс. общий ток заряда', cur: (q) => +q[14] + '', opts: [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120].map((x) => [x + '', x + ' А']), cmd: (v) => ['MNCHGC0' + pad(v, 3), 'MCHGC' + pad(v, 3)] },
    { id: 'pbcv', l: 'Возврат на сеть', unit: 'В', cur: (q) => q[8], num: [22, 25.5, 0.5], cmd: (v) => ['PBCV' + (+v).toFixed(1)] },
    { id: 'pbdv', l: 'Возврат на батарею (0 = полный заряд)', unit: 'В', cur: (q) => q[22], num: [0, 29, 0.5], cmd: (v) => ['PBDV' + pad((+v).toFixed(1), 4)] },
    { id: 'psdv', l: 'Отсечка батареи', unit: 'В', cur: (q) => q[9], num: [20, 24, 0.1], cmd: (v) => ['PSDV' + (+v).toFixed(1)] },
    { id: 'pcvv', l: 'Напряжение заряда (bulk/CV)', unit: 'В', cur: (q) => q[10], num: [24, 29.2, 0.1], cmd: (v) => ['PCVV' + (+v).toFixed(1)] },
    { id: 'pbft', l: 'Напряжение поддержки (float)', unit: 'В', cur: (q) => q[11], num: [24, 29.2, 0.1], cmd: (v) => ['PBFT' + (+v).toFixed(1)] },
    { id: 'freq', l: 'Выходная частота', cur: (q) => Math.round(+q[3]) + '', opts: [['50', '50 Гц'], ['60', '60 Гц']], cmd: (v) => ['F' + v] },
  ];
  const FLAGSET = { a: 'Звуковой сигнал', b: 'Байпас при перегрузке', k: 'Возврат экрана через 1 мин', u: 'Авторестарт после перегрузки',
    v: 'Авторестарт после перегрева', x: 'Подсветка экрана', y: 'Сигнал при пропадании сети', z: 'Запись кодов ошибок' };
  const same = (a, b) => (isNaN(+a) || isNaN(+b)) ? a == b : Math.abs(+a - +b) < 0.05;

  async function read() {
    const get = async (p) => { try { return (await entity('inverter', p)).value; } catch (e) { return undefined; } };
    const [qpiri, qflag, mode, ...prio] = await Promise.all([
      get('text_sensor/' + P + 'raw qpiri'), get('text_sensor/' + P + 'raw qflag'), get('text_sensor/' + P + 'device_mode'),
      ...PRIOS.map(([k]) => get('switch/' + P + k)), get('switch/' + P + 'pv_power_balance'),
    ]);
    Object.assign(S, { qpiri, qflag, mode });
    PRIOS.forEach(([k], i) => (S[k] = prio[i]));
    S.pv_power_balance = prio[PRIOS.length];
    render();
  }

  function build() {
    $('#inv-settings').innerHTML = SETTINGS.map((d) => `<div class="srow"><span class="lab">${d.l}</span><span class="cur" data-cur="${d.id}"></span>
      <span class="ctl">${d.opts ? `<select data-in="${d.id}">${d.opts.map(([v, t]) => `<option value="${v}">${t}</option>`).join('')}</select>`
        : `<input type="number" data-in="${d.id}" min="${d.num[0]}" max="${d.num[1]}" step="${d.num[2]}">`}
      <button class="btn adm" data-ap="${d.id}">Применить</button></span></div>`).join('');
    $('#inv-flags').innerHTML = Object.entries(FLAGSET).map(([f, t]) =>
      `<div class="srow"><span class="lab">${t}</span><span class="ctl"><button class="btn tg adm" data-fl="${f}">—</button></span></div>`).join('') +
      `<div class="srow"><span class="lab">Баланс мощности PV</span><span class="ctl"><button class="btn tg adm" data-tg="pv_power_balance">—</button></span></div>`;
    $('#inv-prio').innerHTML = PRIOS.map(([k, t]) => `<button class="btn adm" data-pr="${k}">${t}</button>`).join('');

    SETTINGS.forEach((d) => {
      const inp = document.querySelector(`[data-in="${d.id}"]`);
      inp.oninput = () => (inp.dataset.touched = 1);
      document.querySelector(`[data-ap="${d.id}"]`).onclick = () => {
        if (!requireAdmin()) return;
        const v = inp.value, label = d.opts ? inp.selectedOptions[0].text : v + ' ' + d.unit;
        if (v === '' || !confirm(`${d.l}: установить «${label}»?`)) return;
        apply(d.cmd(v), () => same(d.cur(QP()), v));
      };
    });
    document.querySelectorAll('[data-fl]').forEach((b) => (b.onclick = () => {
      if (!requireAdmin()) return;
      const f = b.dataset.fl, on = QF()[f];
      if (!confirm(`${FLAGSET[f]}: ${on ? 'выключить' : 'включить'}?`)) return;
      apply([(on ? 'PD' : 'PE') + f], () => QF()[f] === !on);
    }));
    document.querySelectorAll('[data-tg]').forEach((b) => (b.onclick = async () => {
      if (!requireAdmin()) return;
      const on = S.pv_power_balance === true;
      if (!confirm(`Баланс мощности PV: ${on ? 'выключить' : 'включить'}?`)) return;
      try { await act('inverter', 'switch/' + P + 'pv_power_balance', on ? 'turn_off' : 'turn_on'); toast('Отправлено'); setTimeout(read, 3000); }
      catch (e) { toast('Не получилось: ' + e.message, true); }
    }));
    document.querySelectorAll('[data-pr]').forEach((b) => (b.onclick = async () => {
      if (!requireAdmin() || S[b.dataset.pr] === true) return;
      if (!confirm(`Переключить приоритет выхода на «${b.textContent}»? Это меняет режим реального инвертора.`)) return;
      try { await act('inverter', 'switch/' + P + b.dataset.pr, 'turn_on'); toast('Отправлено, жду подтверждения от инвертора'); setTimeout(read, 4000); }
      catch (e) { toast('Не получилось: ' + e.message, true); }
    }));
    built = true;
  }

  async function apply(cmds, check) {
    if (busy) return toast('Подожди, предыдущая команда ещё выполняется');
    busy = true;
    try {
      for (const c of cmds) {
        await act('inverter', 'text/' + P + 'command', 'set', c);
        toast('Отправлено ' + c + ', жду подтверждения…');
        for (let i = 0; i < 12; i++) {
          await new Promise((r) => setTimeout(r, 1500));
          await read();
          if (check()) { toast('Применено: ' + c); return; }
        }
      }
      toast('Инвертор не подтвердил новое значение', true);
    } catch (e) { toast('Не получилось: ' + e.message, true); } finally { busy = false; }
  }

  function render() {
    const q = QP();
    $('#inv-mode').textContent = S.mode === undefined ? 'нет связи с платой' : (MODES[S.mode] || S.mode || '—');
    document.querySelectorAll('[data-pr]').forEach((b) => b.classList.toggle('on', S[b.dataset.pr] === true));
    if (q.length < 20) { $('#inv-raw').textContent = S.qpiri === undefined ? 'нет связи с платой' : 'QPIRI ещё не прочитан'; return; }
    if (!built) build();
    SETTINGS.forEach((d) => {
      const c = d.cur(q), cur = document.querySelector(`[data-cur="${d.id}"]`), inp = document.querySelector(`[data-in="${d.id}"]`);
      cur.textContent = 'сейчас: ' + (d.opts ? ((d.opts.find(([v]) => v == c) || [, c])[1]) : c + ' ' + d.unit);
      if (document.activeElement !== inp && !inp.dataset.touched) inp.value = d.opts ? (d.opts.find(([v]) => v == c) ? c : inp.value) : c;
    });
    const fl = QF();
    document.querySelectorAll('[data-fl]').forEach((b) => { const x = fl[b.dataset.fl]; b.textContent = x === undefined ? '—' : x ? 'вкл' : 'выкл'; b.classList.toggle('on', !!x); });
    document.querySelectorAll('[data-tg]').forEach((b) => { const x = S.pv_power_balance; b.textContent = x === undefined ? '—' : x ? 'вкл' : 'выкл'; b.classList.toggle('on', !!x); });
    $('#inv-raw').innerHTML = `QPIRI: <code>${esc(S.qpiri)}</code><br>QFLAG: <code>${esc(S.qflag)}</code>`;
  }

  page('inverter', {
    show() { read(); timer = setInterval(() => { if (!busy) read(); }, 15000); },
    hide() { clearInterval(timer); },
  });
})();
