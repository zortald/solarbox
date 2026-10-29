// Панель инвертора, зашивается в прошивку ESPHome (web_server js_include).
// Данные — поток /events, команды — REST самой платы. Внешних зависимостей нет.
(() => {
  const P = 'inverter ';
  const S = {};
  const $ = (q) => document.querySelector(q);

  document.head.insertAdjacentHTML('beforeend', `<meta name=viewport content="width=device-width,initial-scale=1"><title>Инвертор</title><style>
:root{--bg:#f3f4f6;--card:#fff;--fg:#111827;--mute:#6b7280;--line:#e5e7eb;--acc:#2563eb;--ok:#16a34a;--warn:#d97706;--bad:#dc2626}
@media(prefers-color-scheme:dark){:root{--bg:#0f172a;--card:#1e293b;--fg:#f1f5f9;--mute:#94a3b8;--line:#334155;--acc:#60a5fa}}
*{box-sizing:border-box}body{margin:0;font:15px/1.4 system-ui,sans-serif;background:var(--bg);color:var(--fg)}
header{display:flex;align-items:center;gap:10px;padding:14px 16px;background:var(--card);border-bottom:1px solid var(--line);position:sticky;top:0}
header h1{font-size:18px;margin:0;flex:1}.dot{width:10px;height:10px;border-radius:50%;background:var(--bad)}.dot.on{background:var(--ok)}
.sub{color:var(--mute);font-size:13px}main{padding:12px;display:grid;gap:12px;grid-template-columns:repeat(auto-fill,minmax(250px,1fr));max-width:1100px;margin:auto}
.card{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:14px}
.card h2{font-size:13px;font-weight:600;color:var(--mute);margin:0 0 8px;text-transform:uppercase;letter-spacing:.04em}
.big{font-size:34px;font-weight:700;line-height:1.1}.big small{font-size:16px;font-weight:500;color:var(--mute);margin-left:4px}
.row{display:flex;justify-content:space-between;padding:3px 0;border-top:1px dashed var(--line)}.row:first-of-type{border-top:0}.row span:last-child{font-weight:600}
.bar{height:8px;background:var(--line);border-radius:4px;overflow:hidden;margin:8px 0}.bar i{display:block;height:100%;background:var(--ok)}
.seg{display:flex;gap:6px;flex-wrap:wrap}.seg button,.btn{flex:1;padding:10px 8px;border:1px solid var(--line);border-radius:10px;background:var(--bg);color:var(--fg);font:inherit;cursor:pointer}
.seg button.on{background:var(--acc);border-color:var(--acc);color:#fff;font-weight:600}.seg button:disabled{opacity:.5}
.tg{display:flex;justify-content:space-between;align-items:center;padding:6px 0;border-top:1px dashed var(--line)}
.tg button{min-width:64px;padding:6px 10px;border-radius:8px;border:1px solid var(--line);background:var(--bg);color:var(--fg);font:inherit;cursor:pointer}.tg button.on{background:var(--ok);color:#fff;border-color:var(--ok)}
.flags{display:flex;flex-wrap:wrap;gap:6px}.flag{padding:3px 8px;border-radius:20px;font-size:12px;background:var(--bg);color:var(--mute);border:1px solid var(--line)}.flag.on{color:#fff;background:var(--ok);border-color:var(--ok)}
.wide{grid-column:1/-1}ul.scan{margin:8px 0 0;padding:0;list-style:none}ul.scan li{padding:4px 0;border-top:1px dashed var(--line);font-variant-numeric:tabular-nums}
.flow svg{width:100%;max-width:640px;height:auto;display:block;margin:auto}.flow text{fill:var(--fg);font:600 17px system-ui}.flow .lbl{fill:var(--mute);font:500 12.5px system-ui}
.flow .ln{fill:none;stroke:var(--line);stroke-width:3;stroke-linecap:round}.flow .mv{fill:none;stroke-width:3;stroke-dasharray:3 12;stroke-linecap:round;animation:dash 1s linear infinite}.flow .rev{animation-direction:reverse}
@keyframes dash{to{stroke-dashoffset:-15}}.flow .node{fill:var(--card);stroke:var(--line);stroke-width:2}
.set{display:grid;grid-template-columns:1fr auto auto;gap:6px 10px;align-items:center}.set .cur{color:var(--mute);font-size:13px;text-align:right}
.set select,.set input{font:inherit;padding:6px 8px;border-radius:8px;border:1px solid var(--line);background:var(--bg);color:var(--fg);max-width:170px}.set input{width:90px}
.set .lab{padding:6px 0}.set .ro{color:var(--mute)}
@media(max-width:560px){.card[style]{grid-column:auto!important}}.st{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}.st th,.st td{padding:5px 6px;text-align:right;border-top:1px dashed var(--line)}.st th:first-child,.st td:first-child{text-align:left}.st th{color:var(--mute);font-weight:600;font-size:12px;border-top:0}
.chart svg{width:100%;height:auto;display:block}.chart .ax{stroke:var(--line);stroke-width:1}.chart text{fill:var(--mute);font:11px system-ui}.chart polyline{fill:none;stroke-width:2;stroke-linejoin:round}
.lg{display:flex;gap:12px;flex-wrap:wrap;font-size:13px;color:var(--mute);margin-top:6px}.lg i{display:inline-block;width:12px;height:3px;border-radius:2px;margin-right:5px;vertical-align:middle}
nav{display:flex;gap:4px}nav a{padding:6px 10px;border-radius:8px;color:var(--fg);text-decoration:none;font-size:14px}nav a.on{background:var(--acc);color:#fff}
@media(max-width:560px){header{flex-wrap:wrap}header h1{flex:0}nav{order:3;width:100%}nav a{flex:1;text-align:center}}
main[hidden]{display:none}
#msg{position:fixed;left:50%;bottom:16px;transform:translateX(-50%);background:var(--fg);color:var(--bg);padding:8px 14px;border-radius:10px;display:none}
</style>`);

  document.body.innerHTML = `
<header><span class=dot id=dot></span><h1>Инвертор</h1><nav><a href=#main>Главная</a><a href=#settings>Настройки</a></nav><span class=sub id=hdr>подключение…</span></header>
<main data-page="main">
 <div class="card wide flow"><h2>Потоки энергии</h2><div id=flow></div></div>
 <div class=card><h2>Нагрузка</h2><div class=big data-k="ac_output_active_power" data-u="Вт"></div>
  <div class=bar><i data-bar="output_load_percent"></i></div>
  <div class=row><span>Загрузка</span><span data-k="output_load_percent" data-u="%"></span></div>
  <div class=row><span>Полная мощность</span><span data-k="ac_output_apparent_power" data-u="ВА"></span></div>
  <div class=row><span>Выход</span><span><b data-k="ac_output_voltage" data-u="В"></b> · <b data-k="ac_output_frequency" data-u="Гц"></b></span></div></div>
 <div class=card><h2>Батарея</h2><div class=big data-k="battery_capacity_percent" data-u="%"></div>
  <div class=bar><i data-bar="battery_capacity_percent"></i></div>
  <div class=row><span>Напряжение</span><span data-k="battery_voltage" data-u="В" data-d=2></span></div>
  <div class=row><span>Заряд</span><span data-k="battery_charging_current" data-u="А"></span></div>
  <div class=row><span>Разряд</span><span data-k="battery_discharge_current" data-u="А"></span></div></div>
 <div class=card><h2>Сеть</h2><div class=big data-k="grid_voltage" data-u="В"></div>
  <div class=row><span>Частота</span><span data-k="grid_frequency" data-u="Гц"></span></div>
  <div class=row><span>Заряд от сети</span><span data-f="ac_charging_status"></span></div></div>
 <div class=card><h2>Солнце</h2><div class=big data-k="pv_charging_power" data-u="Вт"></div>
  <div class=row><span>Напряжение PV</span><span data-k="pv_input_voltage" data-u="В"></span></div>
  <div class=row><span>Ток PV</span><span data-k="pv_input_current_for_battery" data-u="А"></span></div>
  <div class=row><span>Заряд от солнца</span><span data-f="scc_charging_status"></span></div></div>
 <div class=card><h2>Инвертор</h2><div class=big id=mode>—</div>
  <div class=row><span>Радиатор</span><span data-k="inverter_heat_sink_temperature" data-u="°C"></span></div>
  <div class=row><span>Шина DC</span><span data-k="bus_voltage" data-u="В"></span></div>
  <div class=row><span>Wi‑Fi</span><span data-k="wifi_signal" data-u="дБм"></span></div></div>
 <div class=card><h2>Отключения сети</h2><div class=big id=out-now>—</div><div class=sub id=out-since></div>
  <table class=st id=out-stat></table><ul class=scan id=out-log></ul></div>
 <div class="card" style="grid-column:span 2"><h2>Энергия, кВт·ч</h2><table class=st id=stats></table><p class=sub>Сеть — оценка: нагрузка + заряд − солнце.</p></div>
 <div class="card wide chart"><h2>Последние сутки</h2><div id=chart></div>
  <div class=lg><span><i style="background:#f59e0b"></i>Солнце</span><span><i style="background:#2563eb"></i>Нагрузка</span><span><i style="background:#16a34a"></i>Батарея (+заряд / −разряд)</span><span><i style="background:#6b7280"></i>Сеть ≈</span></div></div>
 <div class="card wide"><h2>Состояние</h2><div class=flags id=flags></div></div>
</main>
<main data-page="settings">
 <div class="card wide"><h2>Приоритет питания нагрузки</h2>
  <div class=seg id=prio>
   <button data-sw="output_source_priority_utility">Сеть</button>
   <button data-sw="output_source_priority_solar">Солнце</button>
   <button data-sw="output_source_priority_battery">SBU</button></div>
  <p class=sub>Сеть — от сети, солнце в помощь. Солнце — сначала солнце, потом сеть. SBU — солнце, затем батарея, сеть в последнюю очередь.</p></div>
 <div class="card wide"><h2>Wi‑Fi</h2>
  <div class=row><span>Сеть</span><span data-t="wifi_ssid"></span></div>
  <div class=row><span>Адрес</span><span data-t="wifi_ip"></span></div>
  <div class=row><span>Сигнал</span><span data-k="wifi_signal" data-u="дБм"></span></div>
  <div class=row><span>Статус</span><span data-t="wifi_status"></span></div>
  <p><button class=btn id=scan>Найти сети</button></p><ul class=scan id=scanres></ul>
  <div class=set><span class=lab>Имя сети</span><span></span><span><input id=w-ssid maxlength=32 style="width:170px"></span>
   <span class=lab>Пароль</span><span></span><span><input id=w-pass type=password maxlength=64 style="width:170px"></span></div>
  <p><button class=btn id=w-go>Подключиться</button></p>
  <p class=sub>Плата пробует новую сеть 30 секунд. Если не получится — сама вернётся к прежней. Если получится, эта страница перестанет открываться по старому адресу: плата будет в другой сети.</p></div>
 <div class="card wide"><h2>Настройки инвертора</h2><div class=set id=settings></div>
  <p class=sub>Значения читаются из инвертора (QPIRI/QFLAG). После «Применить» панель ждёт, пока инвертор покажет новое значение. Диапазоны напряжений — типовые для 24 В; недопустимое инвертор отклонит сам.</p></div>
</main>
<div id=msg></div>`;

  const MODES = {P: 'Включение', S: 'Ожидание', L: 'От сети', B: 'От батареи', F: 'Авария', H: 'Энергосбережение'};
  const FLAGS = {
    load_status: 'Нагрузка есть', charging_status: 'Идёт заряд', scc_charging_status: 'Заряд от солнца',
    ac_charging_status: 'Заряд от сети', charging_to_floating_mode: 'Поддерживающий заряд',
    battery_voltage_to_steady_while_charging: 'Напряжение АКБ стабильно', switch_on: 'Инвертор включён',
    add_sbu_priority_version: 'Поддержка SBU', configuration_status: 'Конфигурация изменена',
    silence_buzzer_open_buzzer: 'Звук отключён', overload_bypass_function: 'Байпас при перегрузке',
    lcd_escape_to_default: 'Возврат экрана', overload_restart_function: 'Рестарт после перегрузки',
    over_temperature_restart_function: 'Рестарт после перегрева',
  };

  const toast = (t) => { const m = $('#msg'); m.textContent = t; m.style.display = 'block'; clearTimeout(toast.t); toast.t = setTimeout(() => m.style.display = 'none', 3000); };
  const fmt = (k, d) => { const v = S[k]; if (v === undefined || v === null || Number.isNaN(v)) return '—'; return typeof v === 'number' ? v.toFixed(d ?? (Math.abs(v) < 100 && v % 1 ? 1 : 0)) : v; };


  const pad = (n, w) => String(n).padStart(w, '0');
  const QP = () => (S.raw_qpiri || '').replace('(', '').trim().split(/\s+/);
  const QF = () => { const f = (S.raw_qflag || '').replace('(', ''), e = f.indexOf('E'), d = f.indexOf('D'), on = {};
    for (let i = 0; i < f.length; i++) { const c = f[i]; if (c === 'E' || c === 'D') continue; on[c] = e >= 0 && i > e && (d < e || i < d); } return on; };
  const sel = (o) => Object.entries(o);
  const SETTINGS = [
    {id: 'pcp', l: 'Приоритет заряда', cur: (q) => q[17], opts: sel({0: 'Сначала сеть', 1: 'Сначала солнце', 2: 'Солнце + сеть', 3: 'Только солнце'}), cmd: (v) => ['PCP' + pad(v, 2)]},
    {id: 'pgr', l: 'Диапазон входа', cur: (q) => q[15], opts: sel({0: 'Бытовой (90–280 В)', 1: 'ИБП (170–280 В)'}), cmd: (v) => ['PGR' + pad(v, 2)]},
    {id: 'pbt', l: 'Тип батареи', cur: (q) => q[12], opts: sel({0: 'AGM', 1: 'Заливная', 2: 'Пользовательская', 3: 'Pylontech', 4: 'Shinheung', 5: 'Weco', 6: 'Soltaro', 7: 'BAK', 8: 'Lib', 9: 'Lic'}), cmd: (v) => ['PBT' + pad(v, 2)]},
    {id: 'much', l: 'Макс. ток заряда от сети, А', cur: (q) => +q[13] + '', opts: [2, 10, 20, 30, 40, 50, 60, 70, 80].map((x) => [x + '', x + ' А']), cmd: (v) => ['MUCHGC0' + pad(v, 2)]},
    {id: 'mch', l: 'Макс. общий ток заряда, А', cur: (q) => +q[14] + '', opts: [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120].map((x) => [x + '', x + ' А']), cmd: (v) => ['MNCHGC0' + pad(v, 3), 'MCHGC' + pad(v, 3)]},
    {id: 'pbcv', l: 'Возврат на сеть, В', cur: (q) => q[8], num: [22, 25.5, 0.5], cmd: (v) => ['PBCV' + (+v).toFixed(1)]},
    {id: 'pbdv', l: 'Возврат на батарею, В (0 = полный заряд)', cur: (q) => q[22], num: [0, 29, 0.5], cmd: (v) => ['PBDV' + pad((+v).toFixed(1), 4)]},
    {id: 'psdv', l: 'Отсечка батареи, В', cur: (q) => q[9], num: [20, 24, 0.1], cmd: (v) => ['PSDV' + (+v).toFixed(1)]},
    {id: 'pcvv', l: 'Напряжение заряда (bulk/CV), В', cur: (q) => q[10], num: [24, 29.2, 0.1], cmd: (v) => ['PCVV' + (+v).toFixed(1)]},
    {id: 'pbft', l: 'Напряжение поддержки (float), В', cur: (q) => q[11], num: [24, 29.2, 0.1], cmd: (v) => ['PBFT' + (+v).toFixed(1)]},
    {id: 'freq', l: 'Выходная частота', cur: (q) => Math.round(+q[3]) + '', opts: [['50', '50 Гц'], ['60', '60 Гц']], cmd: (v) => ['F' + v]},
  ];
  const FLAGSET = {a: 'Звуковой сигнал', b: 'Байпас при перегрузке', k: 'Возврат экрана через 1 мин', u: 'Авторестарт после перегрузки',
    v: 'Авторестарт после перегрева', x: 'Подсветка экрана', y: 'Сигнал при пропадании сети', z: 'Запись кодов ошибок'};
  const same = (a, b) => (isNaN(+a) || isNaN(+b)) ? a == b : Math.abs(+a - +b) < 0.05;
  let built = false, busy = false;

  async function sendCmd(c) { await post(`/text/${encodeURIComponent(P + 'command')}/set?value=${encodeURIComponent(c)}`); }
  async function apply(cmds, check) {
    if (busy) return toast('Подожди, предыдущая команда ещё выполняется');
    busy = true;
    try {
      for (const c of cmds) {
        await sendCmd(c); toast('Отправлено ' + c + ', жду подтверждения…');
        for (let i = 0; i < 15; i++) { await new Promise((r) => setTimeout(r, 1000)); if (check()) { toast('Применено: ' + c); return; } }
      }
      toast('Инвертор не принял значение');
    } catch (e) { toast('Не получилось: ' + e.message); } finally { busy = false; }
  }

  function buildSettings() {
    const el = $('#settings');
    el.innerHTML = SETTINGS.map((d) => `<span class=lab>${d.l}</span><span class=cur data-cur="${d.id}"></span><span>` +
      (d.opts ? `<select data-in="${d.id}">${d.opts.map(([v, t]) => `<option value="${v}">${t}</option>`).join('')}</select>`
              : `<input type=number data-in="${d.id}" min=${d.num[0]} max=${d.num[1]} step=${d.num[2]}>`) +
      ` <button class=btn data-ap="${d.id}">Применить</button></span>`).join('') +
      Object.entries(FLAGSET).map(([f, t]) => `<span class=lab>${t}</span><span class=cur data-fcur="${f}"></span><span><button class=btn data-fl="${f}">—</button></span>`).join('') +
      `<span class=lab>Баланс мощности PV</span><span class=cur></span><span><button class=btn data-tg="pv_power_balance">—</button></span>` +
      ['Поле 26 (неизвестно)', 'Поле 27 (неизвестно)', 'Поле 28 (неизвестно)'].map((t, i) => `<span class="lab ro">${t}</span><span class=cur data-raw="${25 + i}"></span><span class=ro>только чтение</span>`).join('');
    SETTINGS.forEach((d) => {
      const inp = el.querySelector(`[data-in="${d.id}"]`);
      el.querySelector(`[data-ap="${d.id}"]`).onclick = () => {
        const v = inp.value, label = d.opts ? inp.selectedOptions[0].text : v + ' В';
        if (v === '' || !confirm(`${d.l}: установить «${label}»?`)) return;
        apply(d.cmd(v), () => same(d.cur(QP()), v));
      };
    });
    el.querySelectorAll('[data-fl]').forEach((b) => b.onclick = () => {
      const f = b.dataset.fl, on = QF()[f];
      if (!confirm(`${FLAGSET[f]}: ${on ? 'выключить' : 'включить'}?`)) return;
      apply([(on ? 'PD' : 'PE') + f], () => QF()[f] === !on);
    });
    el.querySelectorAll('[data-tg]').forEach((b) => b.onclick = async () => {
      const on = S[b.dataset.tg] === true;
      if (!confirm(`Баланс мощности PV: ${on ? 'выключить' : 'включить'}?`)) return;
      try { await post(`/switch/${encodeURIComponent(P + b.dataset.tg)}/${on ? 'turn_off' : 'turn_on'}`); toast('Команда отправлена'); } catch (e) { toast('Не получилось: ' + e.message); }
    });
    built = true;
  }

  function renderSettings() {
    const q = QP(); if (q.length < 20) return;
    if (!built) buildSettings();
    SETTINGS.forEach((d) => {
      const c = d.cur(q), cur = $(`[data-cur="${d.id}"]`), inp = $(`[data-in="${d.id}"]`);
      const t = d.opts ? ((d.opts.find(([v]) => v == c) || [, c])[1]) : c + ' В';
      cur.textContent = 'сейчас: ' + t;
      if (document.activeElement !== inp && !inp.dataset.touched) inp.value = d.opts ? (d.opts.find(([v]) => v == c) ? c : inp.value) : c;
      inp.oninput = () => inp.dataset.touched = 1;
    });
    const fl = QF();
    document.querySelectorAll('[data-fl]').forEach((b) => { const v = fl[b.dataset.fl]; b.textContent = v === undefined ? '—' : v ? 'вкл' : 'выкл'; b.classList.toggle('on', !!v); });
    document.querySelectorAll('[data-raw]').forEach((e) => e.textContent = q[+e.dataset.raw] ?? '—');
  }

  // Схема строится один раз; дальше меняются только тексты и направление/видимость анимации,
  // иначе пересоздание SVG на каждом событии перезапускает анимацию и она дёргается.
  const FLOW = [
    // id, путь, цвет; путь нарисован: солнце→инв, батарея→инв, сеть→инв, инв→нагрузка
    ['pv', 'M100 60 H196 Q210 60 210 74 V120', '#f59e0b'], ['batt', 'M100 240 H196 Q210 240 210 226 V180', '#16a34a'],
    ['grid', 'M420 60 H324 Q310 60 310 74 V120', '#6b7280'], ['load', 'M310 180 V226 Q310 240 324 240 H420', '#2563eb'],
  ];
  function buildFlow() {
    // three = третья строка (у батареи — заряд/разряд); блок чуть выше, строки сдвинуты вверх
    const box = (id, x, y, t, col, three) => `<rect class=node x="${x - 60}" y="${y - (three ? 36 : 30)}" width="120" height="${three ? 72 : 60}" rx="12" style="stroke:${col}"/>` +
      (id === 'grid' ? `<line id=fx-grid x1="${x - 52}" y1="${y + 24}" x2="${x + 52}" y2="${y - 24}" stroke="#dc2626" stroke-width="3" stroke-linecap="round" opacity=".7" style="display:none"/>` : '') +
      `<text id=fv-${id} x="${x}" y="${y - (three ? 10 : 4)}" text-anchor=middle></text><text class=lbl id=fl-${id} x="${x}" y="${y + (three ? 9 : 16)}" text-anchor=middle>${t}</text>` +
      (three ? `<text class=lbl id=fs-${id} x="${x}" y="${y + 25}" text-anchor=middle></text>` : '');
    $('#flow').innerHTML = `<svg viewBox="30 20 460 266">` +
      FLOW.map(([id, d, c]) => `<path class=ln d="${d}"/><path class=mv id=fm-${id} stroke="${c}" d="${d}" style="display:none"/>`).join('') +
      box('pv', 100, 60, 'Солнце', '#f59e0b') + box('batt', 100, 240, 'Батарея', '#16a34a', true) +
      box('grid', 420, 60, 'Сеть', '#6b7280') +
      `<line id=fx-grid x1="364" y1="86" x2="476" y2="34" stroke="#dc2626" stroke-width="4" stroke-linecap="round" style="display:none"/>` + box('load', 420, 240, 'Нагрузка', '#2563eb') +
      `<rect class=node x="210" y="120" width="100" height="60" rx="12"/><text x="260" y="146" text-anchor=middle>Инвертор</text>` +
      `<text class=lbl id=fl-inv x="260" y="166" text-anchor=middle></text></svg>`;
  }
  function renderFlow() {
    if (!$('#fm-pv')) buildFlow();
    const n = (k) => +S[k] || 0;
    const pv = n('pv_charging_power'), load = n('ac_output_active_power');
    const batt = Math.round(n('battery_voltage') * (n('battery_charging_current') - n('battery_discharge_current')));
    const grid = S.device_mode === 'L' ? Math.max(0, Math.round(load + Math.max(batt, 0) - pv)) : 0;
    // along: поток идёт в ту же сторону, что и путь; для батареи — только при разряде
    const noGrid = S.grid_voltage !== undefined && +S.grid_voltage < 150;
    const st = {pv: [pv, true], batt: [batt, batt < 0], grid: [noGrid ? 0 : grid, true], load: [load, true]};
    for (const [id, [w, along]] of Object.entries(st)) {
      const m = $('#fm-' + id), show = Math.abs(w) > 5 ? '' : 'none';
      if (m.style.display !== show) m.style.display = show;
      if (m.classList.contains('rev') === along) m.classList.toggle('rev', !along);
    }
    const set = (id, t) => { const e = $(id); if (e.textContent !== t) e.textContent = t; };
    const soc = S.battery_capacity_percent !== undefined ? ' · ' + S.battery_capacity_percent + '%' : '';
    set('#fv-pv', pv + ' Вт'); set('#fl-pv', 'Солнце · ' + n('pv_input_voltage').toFixed(0) + ' В');
    set('#fv-batt', Math.abs(batt) + ' Вт'); set('#fl-batt', 'Батарея' + soc); set('#fs-batt', batt > 5 ? 'заряд' : batt < -5 ? 'разряд' : '');
    $('#fx-grid').style.display = noGrid ? '' : 'none';
    set('#fv-grid', noGrid ? 'нет сети' : (S.device_mode === 'L' ? '≈' + grid : '0') + ' Вт'); set('#fl-grid', 'Сеть · ' + fmt('grid_voltage') + ' В');
    set('#fv-load', load + ' Вт'); set('#fl-load', 'Нагрузка · ' + fmt('output_load_percent') + '%');
    set('#fl-inv', S.device_mode ? (MODES[S.device_mode] || S.device_mode) : '');
  }

  function renderStats() {
    let st; try { st = JSON.parse(S.stats || ''); } catch (e) { return; }
    const rows = ['Солнце', 'Нагрузка', 'Заряд АКБ', 'Разряд АКБ', 'Сеть ≈'];
    const k = (x) => (x / 1000).toFixed(x >= 10000 ? 1 : 2);
    $('#stats').innerHTML = '<tr><th></th><th>Сегодня</th><th>Вчера</th><th>Месяц</th></tr>' +
      rows.map((r, i) => `<tr><td>${r}</td><td>${k(st.t[i])}</td><td>${k(st.y[i])}</td><td>${k(st.m[i])}</td></tr>`).join('');
  }

  function renderChart() {
    const ser = [['hist_pv', '#f59e0b'], ['hist_load', '#2563eb'], ['hist_batt', '#16a34a'], ['hist_grid', '#6b7280']]
      .map(([k, c]) => [(S[k] || '').split(',').filter((x) => x !== '').map(Number), c]);
    const n = Math.max(...ser.map(([d]) => d.length));
    if (n < 2) { $('#chart').innerHTML = '<p class=sub>График накапливается: первая точка через 10 минут после старта платы, сутки — через 24 часа.</p>'; return; }
    const W = 1000, H = 260, L = 44, R = 8, T = 10, B = 24;
    let hi = Math.max(100, ...ser.flatMap(([d]) => d)), lo = Math.min(0, ...ser.flatMap(([d]) => d));
    const step = [100, 200, 500, 1000, 2000].find((x) => (hi - lo) / x <= 6) || 5000;
    hi = Math.ceil(hi / step) * step; lo = Math.floor(lo / step) * step;
    const x = (i) => L + (W - L - R) * i / 143, y = (v) => T + (H - T - B) * (hi - v) / (hi - lo);
    let g = '';
    for (let v = lo; v <= hi; v += step) g += `<line class=ax x1="${L}" x2="${W - R}" y1="${y(v)}" y2="${y(v)}"/><text x="${L - 6}" y="${y(v) + 4}" text-anchor=end>${v}</text>`;
    const now = Date.now(), off = 144 - n;
    for (let h = 0; h <= 24; h += 4) {
      const i = h * 6, t = new Date(now - (143 - i) * 600000);
      g += `<text x="${x(i)}" y="${H - 6}" text-anchor=${h === 24 ? "end" : h === 0 ? "start" : "middle"}>${pad(t.getHours(), 2)}:${pad(t.getMinutes(), 2)}</text>`;
    }
    const lines = ser.map(([d, c]) => d.length ? `<polyline stroke="${c}" points="${d.map((v, i) => x(off + i - (n - d.length)).toFixed(1) + ',' + y(v).toFixed(1)).join(' ')}"/>` : '').join('');
    $('#chart').innerHTML = `<svg viewBox="0 0 ${W} ${H}">${g}<line class=ax x1="${L}" x2="${W - R}" y1="${y(0)}" y2="${y(0)}" style="stroke:var(--mute)"/>${lines}</svg>`;
  }


  const dur = (sec) => { sec = Math.max(0, Math.round(sec)); const h = Math.floor(sec / 3600), m = Math.round(sec % 3600 / 60);
    return h ? (m ? `${h} ч ${m} мин` : `${h} ч`) : `${m} мин`; };
  const when = (ts) => { const d = new Date(ts * 1000);
    return `${pad(d.getDate(), 2)}.${pad(d.getMonth() + 1, 2)} ${pad(d.getHours(), 2)}:${pad(d.getMinutes(), 2)}`; };
  let outSync = null;  // [время платы, время браузера] — чтобы текущее отключение тикало между обновлениями
  function renderOutages() {
    let o; try { o = JSON.parse(S.outages || ''); } catch (e) { return; }
    if (!outSync || outSync[0] !== o.now) outSync = [o.now, Date.now()];
    const now = o.now ? o.now + (Date.now() - outSync[1]) / 1000 : 0;
    const big = $('#out-now'), since = $('#out-since');
    if (o.cur) { big.textContent = 'Нет сети'; big.style.color = 'var(--bad)'; since.textContent = `уже ${dur(now - o.cur)}, с ${when(o.cur)}`; }
    else { big.textContent = 'Сеть есть'; big.style.color = 'var(--ok)'; since.textContent = o.log.length ? `последнее отключение закончилось ${when(o.log[0] + o.log[1])}` : 'отключений ещё не было'; }
    const c = o.c, row = (t, n, s) => `<tr><td>${t}</td><td>${n} раз</td><td>${dur(s)}</td></tr>`;
    $('#out-stat').innerHTML = row('Сегодня', c[0], c[1]) + row('Вчера', c[2], c[3]) + row('Месяц', c[4], c[5]);
    const items = [];
    for (let i = 0; i + 1 < o.log.length; i += 2) items.push(`<li>${when(o.log[i])} — ${dur(o.log[i + 1])}</li>`);
    $('#out-log').innerHTML = items.join('');
  }
  setInterval(() => { if (S.outages) renderOutages(); }, 30000);

  function render() {
    document.querySelectorAll('[data-k]').forEach((e) => {
      const u = e.dataset.u, v = fmt(e.dataset.k, e.dataset.d && +e.dataset.d);
      e.innerHTML = v === '—' ? '—' : (e.classList.contains('big') ? `${v}<small>${u}</small>` : `${v} ${u}`);
    });
    document.querySelectorAll('[data-bar]').forEach((e) => { e.style.width = Math.max(0, Math.min(100, S[e.dataset.bar] || 0)) + '%'; });
    document.querySelectorAll('[data-f]').forEach((e) => { const v = S[e.dataset.f]; e.textContent = v === undefined ? '—' : v ? 'да' : 'нет'; });
    document.querySelectorAll('[data-sw]').forEach((b) => b.classList.toggle('on', S[b.dataset.sw] === true));
    document.querySelectorAll('[data-tg]').forEach((b) => { const v = S[b.dataset.tg]; b.classList.toggle('on', v === true); b.textContent = v === undefined ? '—' : v ? 'вкл' : 'выкл'; });
    renderFlow(); renderSettings(); renderStats(); renderChart(); renderOutages();
    $('#mode').textContent = S.device_mode ? (MODES[S.device_mode] || S.device_mode) : '—';
    $('#flags').innerHTML = Object.entries(FLAGS).filter(([k]) => k in S).map(([k, t]) => `<span class="flag${S[k] ? ' on' : ''}">${t}</span>`).join('');
    const sc = S.wifi_scan;
    $('#scanres').innerHTML = sc ? sc.split(';').map((x) => x.trim()).filter(Boolean).map((x) => {
      const m = x.match(/^(.*) ([0-9A-F:]{17}) ch(\d+) (-?\d+)dBm$/), name = m ? m[1] : x;
      return `<li data-ssid="${name.replace(/"/g, '&quot;')}" style="cursor:pointer">${m ? `<b>${name.replace(/</g, '&lt;') || '(скрытая)'}</b> · ${m[4]} дБм · канал ${m[3]}` : x.replace(/</g, '&lt;')}</li>`;
    }).join('') : '';
    document.querySelectorAll('#scanres li[data-ssid]').forEach((li) => li.onclick = () => { $('#w-ssid').value = li.dataset.ssid; $('#w-pass').focus(); });
    document.querySelectorAll('[data-t]').forEach((e) => { const v = S[e.dataset.t]; if (v !== undefined && e.textContent !== v) e.textContent = v || '—'; });
  }

  async function post(path) {
    const r = await fetch(path, {method: 'POST'});
    if (!r.ok) throw new Error('HTTP ' + r.status);
  }

  document.querySelectorAll('[data-sw]').forEach((b) => b.onclick = async () => {
    if (b.classList.contains('on')) return;
    if (!confirm(`Переключить приоритет инвертора на «${b.textContent}»? Это меняет режим реального инвертора.`)) return;
    document.querySelectorAll('[data-sw]').forEach((x) => x.disabled = true);
    try { await post(`/switch/${encodeURIComponent(P + b.dataset.sw)}/turn_on`); toast('Команда отправлена, жду подтверждения от инвертора'); }
    catch (e) { toast('Не получилось: ' + e.message); }
    setTimeout(() => document.querySelectorAll('[data-sw]').forEach((x) => x.disabled = false), 4000);
  });

  $('#w-go').onclick = async () => {
    const ssid = $('#w-ssid').value.trim(), pass = $('#w-pass').value;
    if (!ssid) return toast('Укажи имя сети');
    if (!confirm(`Подключить плату к сети «${ssid}»? Если не получится за 30 секунд, она вернётся к текущей.`)) return;
    try { await post(`/text/${encodeURIComponent(P + 'wifi connect')}/set?value=${encodeURIComponent(ssid + '\t' + pass)}`); $('#w-pass').value = ''; toast('Подключаюсь…'); }
    catch (e) { toast('Не получилось: ' + e.message); }
  };
  $('#scan').onclick = async () => {
    try { await post(`/button/${encodeURIComponent(P + 'wifi scan start')}/press`); toast('Сканирую, результат через несколько секунд'); }
    catch (e) { toast('Не получилось: ' + e.message); }
  };

  // Снимок при подключении несёт name, обновления — только id вида "sensor/inverter grid_voltage".
  let last = 0;
  function connect() {
    const es = new EventSource('/events');
    es.addEventListener('state', (ev) => {
      const d = JSON.parse(ev.data);
      const slash = (d.id || '').indexOf('/');
      if (slash < 0) return;
      const domain = d.id.slice(0, slash), name = d.id.slice(slash + 1);
      if (!name.startsWith(P)) return;
      const k = name.slice(P.length).replace(/ /g, '_');
      S[k] = domain === 'text_sensor' ? d.state : d.value;
      last = Date.now(); render();
    });
    es.addEventListener('ping', (ev) => {
      last = Date.now();
      try { const d = JSON.parse(ev.data); if (d.uptime) S.uptime = d.uptime; } catch (e) {}
    });
    es.onerror = () => { es.close(); setTimeout(connect, 3000); };
  }
  setInterval(() => {
    const live = Date.now() - last < 30000;
    $('#dot').classList.toggle('on', live);
    const up = S.uptime ? `работает ${Math.floor(S.uptime / 3600)} ч ${Math.floor(S.uptime % 3600 / 60)} мин` : '';
    $('#hdr').textContent = live ? up || 'на связи' : 'нет связи';
  }, 1000);
  function page() {
    const p = (location.hash || '#main').slice(1);
    document.querySelectorAll('main[data-page]').forEach((m) => m.hidden = m.dataset.page !== p);
    document.querySelectorAll('nav a').forEach((a) => a.classList.toggle('on', a.getAttribute('href') === '#' + p));
  }
  window.addEventListener('hashchange', page);
  page(); render(); connect();
})();
