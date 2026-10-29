// «Режим работы» on the main page: what the inverter is doing now, output and
// charger priority with one-tap switching (admin, confirmed). Read from the
// inverter board on demand while the main page is open.

(() => {
  const { $, esc, entity, act, toast, requireAdmin } = SB;
  const P = 'inverter ';
  const MODES = { P: ['Включение', ''], S: ['Ожидание', ''], L: ['От сети', 'grid'], B: ['От батареи', 'batt'], F: ['Авария', 'bad'], H: ['Энергосбережение', ''] };
  const OUT = [['output_source_priority_utility', 'Сеть'], ['output_source_priority_solar', 'Солнце'], ['output_source_priority_battery', 'SBU']];
  const CHG = [['0', 'Сначала сеть'], ['1', 'Сначала солнце'], ['2', 'Солнце + сеть'], ['3', 'Только солнце']];
  let S = {}, at = 0, busy = false;

  async function read() {
    const get = async (p) => { try { return (await entity('inverter', p)).value; } catch (e) { return undefined; } };
    const [mode, qpiri, ...out] = await Promise.all([get('text_sensor/' + P + 'device_mode'), get('text_sensor/' + P + 'raw qpiri'), ...OUT.map(([k]) => get('switch/' + P + k))]);
    const q = (qpiri || '').replace('(', '').trim().split(/\s+/);
    S = { mode, chg: q.length > 17 ? String(+q[17]) : undefined, out: OUT.find((_, i) => out[i] === true)?.[0], ok: mode !== undefined };
    render();
  }

  function render() {
    const el = $('#flow-modes'); if (!el) return;
    const m = MODES[S.mode] || [S.mode || '—', ''];
    el.innerHTML = `<div class="ph"><h2>Режим работы</h2><a class="small" href="#inverter">Все настройки инвертора →</a></div>
      <div class="mnow ${m[1]}">${S.ok ? esc(m[0]) : 'нет связи с платой инвертора'}</div>
      <div class="mrow"><span>Приоритет выхода</span><div class="seg">${OUT.map(([k, t]) => `<button class="btn adm${S.out === k ? ' on' : ''}" data-out="${k}">${t}</button>`).join('')}</div></div>
      <div class="mrow"><span>Приоритет заряда</span><div class="seg">${CHG.map(([k, t]) => `<button class="btn adm${S.chg === k ? ' on' : ''}" data-chg="${k}">${t}</button>`).join('')}</div></div>`;
    el.querySelectorAll('[data-out]').forEach((b) => (b.onclick = () => change(`Приоритет выхода: «${b.textContent}»`, () => act('inverter', 'switch/' + P + b.dataset.out, 'turn_on'), S.out === b.dataset.out)));
    el.querySelectorAll('[data-chg]').forEach((b) => (b.onclick = () => change(`Приоритет заряда: «${b.textContent}»`, () => act('inverter', 'text/' + P + 'command', 'set', 'PCP0' + b.dataset.chg), S.chg === b.dataset.chg)));
  }

  async function change(what, send, already) {
    if (already || busy || !requireAdmin()) return;
    if (!confirm(`${what}? Это меняет режим реального инвертора.`)) return;
    busy = true;
    try { await send(); toast('Отправлено, жду подтверждения от инвертора'); for (const d of [2500, 4000, 6000]) { await new Promise((r) => setTimeout(r, d)); await read(); } }
    catch (e) { toast('Не получилось: ' + e.message, true); }
    finally { busy = false; at = Date.now(); }
  }

  SB.modes = {
    refresh(force) { if (busy || (!force && Date.now() - at < 15000)) return; at = Date.now(); read(); },
    render,
  };
})();
