// Automations: list, editor, templates and the journal. Rules run on the
// server (no browser needed); this page only edits automations.json.

(() => {
  const { $, esc, api, toast, requireAdmin, page } = SB;
  let rules = [], state = {}, log = [], catalog = [], editing = null, timer = null;

  const OPS = [['<', 'меньше'], ['>', 'больше'], ['<=', '≤'], ['>=', '≥'], ['==', 'равно'], ['!=', 'не равно'], ['changed', 'изменилось'], ['stale', 'нет данных, с']];
  const opLabel = (o) => (OPS.find(([k]) => k === o) || [, o])[1];
  const label = (m) => (SB.metricLabel ? SB.metricLabel(m) : m);

  // Ready-made board actions; config.json → ui.action_presets adds/overrides.
  const PRESETS = () => Object.assign({
    'out_utility': { board: 'inverter', path: 'switch/inverter output_source_priority_utility', action: 'turn_on', label: 'Инвертор: приоритет выхода — сеть' },
    'out_solar': { board: 'inverter', path: 'switch/inverter output_source_priority_solar', action: 'turn_on', label: 'Инвертор: приоритет выхода — солнце' },
    'out_sbu': { board: 'inverter', path: 'switch/inverter output_source_priority_battery', action: 'turn_on', label: 'Инвертор: приоритет выхода — SBU (батарея)' },
    'chg_grid': { board: 'inverter', path: 'text/inverter command', action: 'set', value: 'PCP00', label: 'Инвертор: заряд — сначала сеть' },
    'chg_solar': { board: 'inverter', path: 'text/inverter command', action: 'set', value: 'PCP01', label: 'Инвертор: заряд — сначала солнце' },
    'chg_mix': { board: 'inverter', path: 'text/inverter command', action: 'set', value: 'PCP02', label: 'Инвертор: заряд — солнце + сеть' },
    'chg_only_solar': { board: 'inverter', path: 'text/inverter command', action: 'set', value: 'PCP03', label: 'Инвертор: заряд — только солнце' },
    'inv_restart': { board: 'inverter', path: 'button/inverter restart', action: 'press', label: 'Перезагрузить плату инвертора' },
    'bms_restart': { board: 'bms', path: 'button/Перезагрузка RST', action: 'press', label: 'Перезагрузить плату BMS' },
  }, SB.cfg.ui?.action_presets || {});

  const TEMPLATES = [
    { name: 'Пропала сеть → Telegram', when: { device: 'inverter', metric: 'grid_v', op: '<', value: 150 }, for_s: 10, cooldown_s: 60,
      then: [{ kind: 'telegram', message: '⚡ Пропала сеть. АКБ {bms.soc}%, дом {inverter.ac_out_w} Вт' }],
      on_clear: [{ kind: 'telegram', message: '✅ Сеть вернулась ({inverter.grid_v} В). АКБ {bms.soc}%' }] },
    { name: 'АКБ < 30% → питание от сети', when: { device: 'bms', metric: 'soc', op: '<', value: 30 }, for_s: 60, cooldown_s: 600,
      then: [{ kind: 'board', ...PRESETS().out_utility }, { kind: 'telegram', message: '🔋 АКБ {bms.soc}% — переключил выход на сеть' }] },
    { name: 'АКБ > 90% → SBU', when: { device: 'bms', metric: 'soc', op: '>', value: 90 }, and: [{ device: 'inverter', metric: 'grid_v', op: '>', value: 150 }], for_s: 120, cooldown_s: 600,
      then: [{ kind: 'board', ...PRESETS().out_sbu }, { kind: 'telegram', message: '🔋 АКБ {bms.soc}% — переключил выход на SBU' }] },
    { name: 'Плата BMS молчит → Telegram', when: { device: 'bms', metric: '*', op: 'stale', value: 120 }, cooldown_s: 3600,
      then: [{ kind: 'telegram', message: '📵 Плата BMS не присылает данные больше 2 минут' }],
      on_clear: [{ kind: 'telegram', message: '📶 Плата BMS снова на связи' }] },
    { name: 'Перегрев инвертора → Telegram', when: { device: 'inverter', metric: 'temp', op: '>', value: 60 }, for_s: 60, cooldown_s: 1800,
      then: [{ kind: 'telegram', message: '🌡 Инвертор {inverter.temp} °C, нагрузка {inverter.ac_out_w} Вт' }] },
    { name: 'Большой разброс ячеек → Telegram', when: { device: 'bms', metric: 'cell_delta', op: '>', value: 0.05 }, for_s: 300, cooldown_s: 3600,
      then: [{ kind: 'telegram', message: '▯ Разброс ячеек {bms.cell_delta} В (мин {bms.cell_min}, макс {bms.cell_max})' }] },
  ];

  const newId = () => 'r' + Date.now().toString(36);
  const cond = (c) => c.op === 'stale' ? `${c.device}: нет данных ${c.metric === '*' ? '' : '«' + label(c.metric) + '» '}дольше ${c.value} с`
    : c.op === 'changed' ? `${c.device}: изменилось «${label(c.metric)}»` : `${c.device}: ${label(c.metric)} ${opLabel(c.op)} ${c.value}`;
  const actText = (a) => a.kind === 'telegram' ? '✉ ' + a.message : a.kind === 'log' ? '📝 ' + (a.message || '') :
    (Object.values(PRESETS()).find((p) => p.path === a.path && p.action === a.action && (p.value ?? null) === (a.value ?? null))?.label || `${a.board}: ${a.action} ${a.path}${a.value ? ' = ' + a.value : ''}`);
  const when = (ts) => (ts ? new Date(ts * 1000).toLocaleString('ru') : 'ещё не было');

  async function load() {
    if (!SB.isAdmin) { $('#auto').innerHTML = '<div class="empty">Автоматизации видны после входа («Войти» в меню).</div>'; $('#auto-log').innerHTML = ''; return; }
    try {
      const d = await api('/api/automations');
      rules = d.rules; state = d.state; log = d.log;
      if (d.load_error) toast('automations.json с ошибкой, работают прежние правила: ' + d.load_error, true);
    } catch (e) { $('#auto').innerHTML = `<div class="empty">${esc(e.message)}</div>`; return; }
    if (!editing) list();
    journal();
  }

  async function save(next) {
    try { await api('/api/automations', { method: 'PUT', body: JSON.stringify(next, null, 2), headers: { 'Content-Type': 'application/json' } }); rules = next; toast('Сохранено'); return true; }
    catch (e) { toast('Не сохранилось: ' + e.message, true); return false; }
  }

  function list() {
    $('#auto').innerHTML = `<div class="row"><button class="btn" id="a-new">+ Новое правило</button>
      <select id="a-tpl"><option value="">Из шаблона…</option>${TEMPLATES.map((t, i) => `<option value="${i}">${esc(t.name)}</option>`).join('')}</select></div>` +
      (rules.length ? rules.map((r) => {
        const s = state[r.id] || {};
        return `<div class="arule${r.enabled ? '' : ' off'}"><div class="ah"><b>${esc(r.name)}</b>
            <span class="${s.fired ? 'bad' : s.active ? 'warn' : 'muted'}">${s.fired ? '● сработало' : s.active ? '● условие выполняется' : '○ ждёт'}</span></div>
          <div class="small">Когда ${esc(cond(r.when))}${(r.and || []).map((c) => ' и ' + esc(cond(c))).join('')}${r.for_s ? ` дольше ${r.for_s} с` : ''}</div>
          <div class="small">→ ${(r.then || []).map((a) => esc(actText(a))).join('; ') || '—'}</div>
          ${r.on_clear?.length ? `<div class="small muted">↩ когда прошло: ${r.on_clear.map((a) => esc(actText(a))).join('; ')}</div>` : ''}
          <div class="small muted">Последний раз: ${when(s.last_fired)} · пауза ${r.cooldown_s} с</div>
          <div class="row"><button class="btn" data-ed="${r.id}">Изменить</button><button class="btn" data-tg="${r.id}">${r.enabled ? 'Выключить' : 'Включить'}</button>
            <button class="btn" data-run="${r.id}">Проверить сейчас</button><button class="btn" data-del="${r.id}">Удалить</button></div></div>`;
      }).join('') : '<div class="empty">Правил пока нет — создай своё или возьми шаблон.</div>');
    $('#a-new').onclick = () => edit({ id: newId(), name: 'Новое правило', enabled: true, when: { device: 'bms', metric: 'soc', op: '<', value: 30 }, and: [], for_s: 0, cooldown_s: 60, then: [], on_clear: [] });
    $('#a-tpl').onchange = (e) => { const t = TEMPLATES[+e.target.value]; if (t) edit(JSON.parse(JSON.stringify({ id: newId(), enabled: true, and: [], on_clear: [], for_s: 0, ...t }))); };
    document.querySelectorAll('[data-ed]').forEach((b) => (b.onclick = () => edit(JSON.parse(JSON.stringify(rules.find((r) => r.id === b.dataset.ed))))));
    document.querySelectorAll('[data-tg]').forEach((b) => (b.onclick = () => { if (!requireAdmin()) return; save(rules.map((r) => r.id === b.dataset.tg ? { ...r, enabled: !r.enabled } : r)).then(load); }));
    document.querySelectorAll('[data-del]').forEach((b) => (b.onclick = async () => { const r = rules.find((x) => x.id === b.dataset.del); if (!requireAdmin() || !(await SB.ask(`Удалить правило «${r.name}»?`))) return; save(rules.filter((x) => x.id !== r.id)).then(load); }));
    document.querySelectorAll('[data-run]').forEach((b) => (b.onclick = async () => {
      const r = rules.find((x) => x.id === b.dataset.run);
      if (!requireAdmin() || !(await SB.ask(`Выполнить действия правила «${r.name}» прямо сейчас (без проверки условия)?`))) return;
      try { await api(`/api/automations/${r.id}/run`, { method: 'POST' }); toast('Запущено — смотри журнал'); setTimeout(load, 2500); } catch (e) { toast(e.message, true); }
    }));
  }

  // ---- editor ----
  const devices = () => [...new Set([...catalog.map((c) => c.device), ...Object.keys(SB.BOARDS)])];
  const metrics = (dev) => catalog.filter((c) => c.device === dev).map((c) => c.metric);
  function condRow(c, i, key) {
    const ms = metrics(c.device);
    return `<div class="crow" data-key="${key}" data-i="${i}">
      <select data-f="device">${devices().map((d) => `<option${d === c.device ? ' selected' : ''}>${esc(d)}</option>`).join('')}</select>
      <select data-f="metric"><option value="*"${c.metric === '*' ? ' selected' : ''}>(любой)</option>${ms.map((m) => `<option value="${esc(m)}"${m === c.metric ? ' selected' : ''}>${esc(label(m))}</option>`).join('')}</select>
      <select data-f="op">${OPS.map(([k, t]) => `<option value="${k}"${k === c.op ? ' selected' : ''}>${t}</option>`).join('')}</select>
      <input type="number" step="any" data-f="value" value="${c.value ?? 0}">
      ${key === 'and' ? `<button class="btn" data-rm="and" data-i="${i}">✕</button>` : ''}</div>`;
  }
  function actRow(a, i, key) {
    const P = PRESETS(), pk = Object.keys(P).find((k) => P[k].path === a.path && P[k].action === a.action && (P[k].value ?? null) === (a.value ?? null));
    const sel = a.kind === 'board' ? (pk || 'custom') : a.kind;
    const opts = [['telegram', '✉ Сообщение в Telegram'], ['log', '📝 Запись в журнал'], ...Object.entries(P).map(([k, p]) => [k, p.label]), ['custom', '⚙ Другое действие на плате…']];
    return `<div class="arow" data-key="${key}" data-i="${i}">
      <select data-f="kind">${opts.map(([k, t]) => `<option value="${k}"${k === sel ? ' selected' : ''}>${esc(t)}</option>`).join('')}</select>
      ${a.kind === 'telegram' || a.kind === 'log' ? `<textarea data-f="message" rows="2" placeholder="Текст. Подстановки: {bms.soc}, {inverter.ac_out_w}…">${esc(a.message || '')}</textarea>` : ''}
      ${sel === 'custom' ? `<input data-f="board" placeholder="плата" value="${esc(a.board || '')}"><input data-f="path" placeholder="domain/entity" value="${esc(a.path || '')}">
        <input data-f="action" placeholder="turn_on / press / set" value="${esc(a.action || '')}"><input data-f="value" placeholder="значение" value="${esc(a.value || '')}">` : ''}
      <label class="small">пауза перед, с <input type="number" min="0" data-f="delay_s" value="${a.delay_s || 0}"></label>
      <button class="btn" data-rm="${key}" data-i="${i}">✕</button></div>`;
  }

  function edit(r) {
    if (!requireAdmin()) return;
    editing = r;
    const el = $('#auto');
    const draw = () => {
      el.innerHTML = `<div class="aedit"><label>Название <input id="e-name" value="${esc(r.name)}"></label>
        <h3>Когда</h3>${condRow(r.when, 0, 'when')}
        <h3>И ещё (все должны выполняться)</h3>${(r.and || []).map((c, i) => condRow(c, i, 'and')).join('')}<button class="btn" id="e-add-and">+ условие</button>
        <div class="row"><label class="small">держится не меньше, с <input type="number" min="0" id="e-for" value="${r.for_s || 0}"></label>
          <label class="small">пауза между срабатываниями, с <input type="number" min="0" id="e-cool" value="${r.cooldown_s ?? 60}"></label></div>
        <h3>То</h3>${(r.then || []).map((a, i) => actRow(a, i, 'then')).join('')}<button class="btn" data-add="then">+ действие</button>
        <h3>Когда условие перестало выполняться</h3>${(r.on_clear || []).map((a, i) => actRow(a, i, 'on_clear')).join('')}<button class="btn" data-add="on_clear">+ действие</button>
        <p class="muted small">В тексте сообщения можно подставлять текущие значения: <code>{bms.soc}</code>, <code>{inverter.grid_v}</code>, <code>{bms.cell_delta}</code> и любые другие <code>{плата.показатель}</code>.</p>
        <div class="row"><button class="btn on" id="e-save">Сохранить</button><button class="btn" id="e-cancel">Отмена</button></div></div>`;
      wire();
    };
    const read = () => {
      r.name = $('#e-name').value.trim() || 'Правило';
      r.for_s = +$('#e-for').value || 0; r.cooldown_s = +$('#e-cool').value || 0;
      document.querySelectorAll('.crow').forEach((row) => {
        const c = row.dataset.key === 'when' ? r.when : r.and[+row.dataset.i];
        c.device = row.querySelector('[data-f=device]').value; c.metric = row.querySelector('[data-f=metric]').value;
        c.op = row.querySelector('[data-f=op]').value; c.value = +row.querySelector('[data-f=value]').value || 0;
      });
      document.querySelectorAll('.arow').forEach((row) => {
        const list = r[row.dataset.key], i = +row.dataset.i, k = row.querySelector('[data-f=kind]').value, P = PRESETS();
        const g = (f) => row.querySelector(`[data-f=${f}]`)?.value;
        const delay_s = +g('delay_s') || 0;
        if (k === 'telegram' || k === 'log') list[i] = { kind: k, message: g('message') ?? list[i].message ?? '', delay_s };
        else if (k === 'custom') list[i] = { kind: 'board', board: g('board') ?? list[i].board ?? '', path: g('path') ?? list[i].path ?? '', action: g('action') ?? list[i].action ?? '', value: (g('value') ?? list[i].value) || undefined, delay_s };
        else { const p = P[k]; list[i] = { kind: 'board', board: p.board, path: p.path, action: p.action, value: p.value, delay_s }; }
      });
    };
    const wire = () => {
      el.querySelectorAll('select[data-f=device], select[data-f=kind]').forEach((s) => (s.onchange = () => { read(); draw(); }));
      $('#e-add-and').onclick = () => { read(); (r.and ||= []).push({ device: r.when.device, metric: r.when.metric, op: '>', value: 0 }); draw(); };
      el.querySelectorAll('[data-add]').forEach((b) => (b.onclick = () => { read(); (r[b.dataset.add] ||= []).push({ kind: 'telegram', message: '' }); draw(); }));
      el.querySelectorAll('[data-rm]').forEach((b) => (b.onclick = () => { read(); r[b.dataset.rm].splice(+b.dataset.i, 1); draw(); }));
      $('#e-cancel').onclick = () => { editing = null; list(); };
      $('#e-save').onclick = async () => {
        read();
        const next = rules.some((x) => x.id === r.id) ? rules.map((x) => (x.id === r.id ? r : x)) : [...rules, r];
        if (await save(next)) { editing = null; load(); }
      };
    };
    draw();
  }

  function journal() {
    $('#auto-log').innerHTML = log.length ? log.slice(0, 60).map((l) => `<div class="lrow"><span class="muted">${new Date(l.ts * 1000).toLocaleString('ru')}</span><b>${esc(l.rule)}</b><span>${esc(l.msg)}</span></div>`).join('')
      : '<div class="muted small">Пока пусто.</div>';
  }

  page('automations', {
    async show() {
      try { catalog = (await api('/api/catalog')).items; } catch (e) { catalog = []; }
      editing = null; load(); timer = setInterval(() => { if (!editing) load(); }, 5000);
    },
    hide() { clearInterval(timer); editing = null; },
  });
})();
