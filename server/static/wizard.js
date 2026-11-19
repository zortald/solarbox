// Setup wizard: welcome → equipment (whatever is connected is both shown and
// controlled) → boards (recommended, or from what the owner already has) →
// site → assembly (buy, solder, flash, adopt) → battery → inverter → tariff →
// Telegram → review → done.
// Nothing is applied while stepping through: the answers are a draft on the
// server (PUT /api/wizard), the review screen shows the server's plan (old →
// new) and «Применить» carries it out. `?demo`: boards and BMS are simulated,
// nothing is saved or sent.

(() => {
  const { $, esc, toast, api } = SB;
  const DEMO = new URLSearchParams(location.search).has('demo');
  const RELAY = 'https://get.solarbox.workers.dev';

  // Board chips: what each can carry. bleInv = BMS slots left when the same
  // board also talks to the inverter. `tested` = runs on a real site;
  // everything else is an estimate until a combined image is built.
  // Pin names are the board's own silkscreen (gpio in brackets where it differs).
  const CHIPS = {
    // L/R = pin headers top to bottom, board seen from the component side, USB at `usb`.
    esp32: { t: 'ESP32 DevKit V1', buy: 'ESP32 DevKit V1 (30 пинов, ESP32-WROOM-32)', inv: true, ble: 3, bleInv: 3, v5: 'VIN', gnd: 'GND', v33: '3V3', tx: 'TX2', rx: 'RX2', gpio: 'GPIO17 / GPIO16',
      mod: 'ESP32-WROOM-32', usb: 'bottom',
      L: ['EN', 'VP', 'VN', 'D34', 'D35', 'D32', 'D33', 'D25', 'D26', 'D27', 'D14', 'D12', 'D13', 'GND', 'VIN'],
      R: ['D23', 'D22', 'TX0', 'RX0', 'D21', 'D19', 'D18', 'D5', 'TX2', 'RX2', 'D4', 'D2', 'D15', 'GND', '3V3'] },
    s3: { t: 'ESP32-S3 DevKitC-1', buy: 'ESP32-S3 DevKitC-1', inv: true, ble: 3, bleInv: 3, v5: '5V', gnd: 'G', v33: '3V3', tx: '17', rx: '18', gpio: 'GPIO17 / GPIO18',
      mod: 'ESP32-S3-WROOM-1', usb: 'bottom',
      L: ['3V3', '3V3', 'RST', '4', '5', '6', '7', '15', '16', '17', '18', '8', '3', '46', '9', '10', '11', '12', '13', '14', '5V', 'G'],
      R: ['G', 'TX', 'RX', '1', '2', '42', '41', '40', '39', '38', '37', '36', '35', '0', '45', '48', '47', '21', '20', '19', 'G', 'G'] },
    c3: { t: 'ESP32-C3 SuperMini', buy: 'ESP32-C3 SuperMini', inv: true, ble: 2, bleInv: 1, v5: '5V', gnd: 'G', v33: '3.3', tx: '21', rx: '20', gpio: 'GPIO21 / GPIO20',
      mod: 'ESP32-C3', usb: 'top',
      L: ['5V', 'G', '3.3', '4', '3', '2', '1', '0'], R: ['5', '6', '7', '8', '9', '10', '20', '21'] },
    d1: { t: 'Wemos D1 mini', buy: 'Wemos D1 mini (ESP8266)', inv: true, ble: 0, bleInv: 0, v5: '5V', gnd: 'G', v33: '3V3', tx: 'TX', rx: 'RX', gpio: 'GPIO1 / GPIO3', noble: true,
      mod: 'ESP-12F', usb: 'bottom',
      L: ['RST', 'A0', 'D0', 'D5', 'D6', 'D7', 'D8', '3V3'], R: ['TX', 'RX', 'D1', 'D2', 'D3', 'D4', 'G', '5V'] },
  };
  // DC-DC from the battery to 5 V: input range must cover the full-charge voltage.
  const DCDC = {
    12: { m: 'LM2596 (вход до 40 В)', max: 14.6 },
    24: { m: 'LM2596 (вход до 40 В)', max: 29.2 },
    48: { m: 'XL7015 (вход 5–80 В)', max: 58.4 },
  };
  // Runs on a real site: a C3 with one JK BMS (the BMS board of site 1).
  const tested = (b) => b.chip === 'c3' && !b.inv && b.nb === 1;
  // ESP8266 (D1 mini) is not supported: no Bluetooth, and the flasher speaks ESP32 only.
  const KINDS = ['esp32', 's3', 'c3'];
  const can = (chip, inv, nb) => { const c = CHIPS[chip]; return (!inv || c.inv) && nb <= (inv ? c.bleInv : c.ble); };

  const W = {
    i: 0,
    want: { invType: 'pi30', bmsType: 'jkble', nbms: 1, volt: 24 },   // type 'none' = not connected
    // Board power: 'inv' = +12 V from the inverter's RJ45 (pin vpin), 'batt' = battery via DC-DC, 'usb' = USB charger.
    power: { src: 'inv', vpin: 4 },
    use: {},             // drivers needed, derived from want
    have: { esp32: 0, s3: 0, c3: 0, d1: 0, max: false },     // boards the owner already has
    plan: [],            // [{ chip, inv, nb, own }] — fixed when leaving «Платы»
    boards: {},          // plan index → { found: {...}, adopted: bool }
    site: { name: '', tz: 'Europe/Kyiv' },
    scan: null,          // BLE scan results (null = not scanned yet)
    packs: [],           // chosen MACs
    batt: { chem: 'lfp', cells: 8, cap: 280, soc100: 3.45, soc0: 2.95, read: false },
    inv: null,
    tg: { token: '' },
    money: { day: '', night: false, nday: '', from: 23, to: 7 },
    usbSkip: {},         // USB ports of the router the owner said not to use
    wifi: { ssid: '' },  // network for boards flashed from the router (password not kept)
  };

  function derive() {
    W.use = { pi30: W.want.invType === 'pi30', jkble: W.want.bmsType === 'jkble' };
  }
  const need = () => ({ inv: !!W.use.pi30, nb: W.use.jkble ? W.want.nbms : 0 });
  const ctlInv = () => W.use.pi30;
  const ctlBms = () => W.use.jkble;

  const STEPS = [
    { id: 'welcome', t: 'Начало' },
    { id: 'goals', t: 'Оборудование' },
    { id: 'pick', t: 'Платы' },
    { id: 'site', t: 'Объект' },
    { id: 'build', t: 'Сборка' },
    { id: 'battery', t: 'Батарея', skip: () => !W.use.jkble && !ctlInv() },
    { id: 'inverter', t: 'Инвертор', skip: () => !ctlInv() },
    { id: 'tariff', t: 'Тариф', skip: () => !W.use.pi30 },
    { id: 'telegram', t: 'Telegram' },
    { id: 'review', t: 'Проверка' },
    { id: 'done', t: 'Готово' },
  ];
  const steps = () => STEPS.filter((s) => !s.skip?.());
  const cur = () => steps()[W.i];

  const get = (path) => path.split('.').reduce((o, k) => o?.[k], W);
  function set(path, val) {
    const ks = path.split('.'), last = ks.pop();
    ks.reduce((o, k) => o[k], W)[last] = val;
  }
  const field = (label, path, attrs = '', hint = '') =>
    `<label class="wzf"><span>${label}</span><input data-k="${path}" value="${esc(get(path))}" ${attrs}>${hint ? `<small class="muted">${hint}</small>` : ''}</label>`;

  // Per-cell voltages → pack, for the bank's nominal 12/24/48 V.
  const nominal = () => ({ 4: 12, 8: 24, 16: 48 }[W.batt.cells] || Math.round(W.batt.cells * 3.2));
  const pack = (cell) => +(cell * W.batt.cells).toFixed(1);

  // ---- boards: recommendation, and a plan from what the owner has ----
  // Missing parts are bought: inverter + up to 3 BMS → one ESP32; otherwise
  // ESP32-C3 for the inverter, ESP32-C3 per 2 BMS (ESP32 for an odd 3).
  function buyFor(inv, nb) {
    const out = [];
    if (inv && nb && nb <= 3) return [{ chip: 'esp32', inv: true, nb }];
    if (inv) out.push({ chip: 'c3', inv: true, nb: 0 });
    while (nb > 0) { const k = nb === 3 ? 3 : Math.min(2, nb); out.push({ chip: k === 3 ? 'esp32' : 'c3', inv: false, nb: k }); nb -= k; }
    return out;
  }
  // Boards at hand = the ones plugged into the router's USB (used first,
  // each tied to its port) + the ones the owner says are in a drawer.
  function fromStock() {
    let { inv, nb } = need();
    const plugged = usbBoards(), left = {}, out = [];
    for (const k of KINDS) left[k] = (W.have[k] || 0) + plugged.filter((x) => x.key === k).length;
    const take = (chip, i, n) => {
      left[chip]--;
      const u = plugged.findIndex((x) => x.key === chip);
      out.push(Object.assign({ chip, inv: i, nb: n, own: true }, u >= 0 ? { usb: plugged.splice(u, 1)[0].port } : {}));
    };
    // One board for everything, if any of them can — the smallest that does.
    for (const chip of ['c3', 'esp32', 's3']) if (left[chip] > 0 && can(chip, inv, nb)) { take(chip, inv, nb); return { boards: out, one: true }; }
    // Inverter: a board that also takes as many BMS as it can.
    if (inv) {
      const chip = ['s3', 'esp32', 'c3'].find((c) => left[c] > 0);
      if (chip) { const n = Math.min(nb, CHIPS[chip].bleInv); take(chip, true, n); nb -= n; inv = false; }
    }
    for (const chip of ['esp32', 's3', 'c3']) while (nb > 0 && left[chip] > 0) { const n = Math.min(nb, CHIPS[chip].ble); take(chip, false, n); nb -= n; }
    return { boards: out.concat(buyFor(inv, nb)), one: false };
  }
  const anyStock = () => KINDS.some((c) => W.have[c] > 0) || usbBoards().length > 0;
  const roles = (b) => [b.inv && 'инвертор', b.nb && (b.nb === 1 ? '1 BMS' : b.nb + ' BMS')].filter(Boolean).join(' + ');

  // What one board of this chip can do for the chosen equipment.
  function verdict(chip) {
    const { inv, nb } = need(), c = CHIPS[chip];
    if (can(chip, inv, nb)) return ['ok', `Хватит одной этой платы на всё${tested({ chip, inv, nb }) ? '' : ' (оценка)'}`];
    const parts = [];
    if (inv) parts.push(c.bleInv && nb ? `инвертор + ${c.bleInv} BMS` : 'инвертор');
    if (nb && c.ble) parts.push(`или до ${c.ble} BMS`);
    if (!parts.length) return ['warn', 'Не подходит: нет Bluetooth для BMS'];
    return ['warn', `Одной не хватит: потянет ${parts.join(' ')}${c.noble && nb ? ' — нет Bluetooth для BMS' : ''}`];
  }

  function boardRow(b, i) {
    return `<div class="wzbrow"><span class="wznum">${i + 1}</span><span><b>${CHIPS[b.chip].t}</b>
      <small class="muted">${roles(b)}${b.own ? ' · есть' : ' · купить'}${tested(b) ? ' · проверено на объекте' : ' · оценка'}</small></span></div>`;
  }

  // ---- steps ----
  const VIEW = {
    welcome: () => `<div class="panel wzhero">
      <img src="/icon-192.png" alt="" width="72" height="72">
      <h2 class="wzh">Вас приветствует Solarbox</h2>
      <p>Сначала — какое оборудование подключаем: его Solarbox будет показывать и им же управлять. Потом подберём платы — в том числе из тех, что уже есть.</p>
      <p class="muted">Займёт ≈ 15–30 минут, если платы уже спаяны. Прерваться можно в любой момент — сделанное сохранится.</p>
    </div>`,

    goals: () => {
      const w = W.want;
      const warn = [];
      if (w.invType === 'deye') warn.push('Deye пока не поддерживается — инвертор пропустим, добавите позже.');
      if (w.invType === 'other') warn.push('Модель неизвестна: посмотрите в инструкции протокол (PI30 / Voltronic) или пришлите фото шильдика.');
      if (w.bmsType === 'other') warn.push('Другие BMS (JK по RS485, Pace, Seplos) — позже.');
      return `<div class="panel wzform">
        <h2>Инвертор</h2>
        <label class="wzf"><span>Модель</span><select data-k="want.invType">
          <option value="pi30" ${w.invType === 'pi30' ? 'selected' : ''}>Voltronic / PowMr / MPP / EASun (PI30)</option>
          <option value="deye" ${w.invType === 'deye' ? 'selected' : ''}>Deye / Sunsynk (пока нет)</option>
          <option value="other" ${w.invType === 'other' ? 'selected' : ''}>Другой / не знаю</option>
          <option value="none" ${w.invType === 'none' ? 'selected' : ''}>Не подключаем</option></select></label>
      </div>
      <div class="panel wzform">
        <h2>Батарея (BMS)</h2>
        <div class="wzrow">
          <label class="wzf"><span>BMS</span><select data-k="want.bmsType">
            <option value="jkble" ${w.bmsType === 'jkble' ? 'selected' : ''}>JK с Bluetooth</option>
            <option value="other" ${w.bmsType === 'other' ? 'selected' : ''}>Другая / не знаю</option>
            <option value="none" ${w.bmsType === 'none' ? 'selected' : ''}>Не подключаем</option></select></label>
          ${w.bmsType !== 'none' ? `<label class="wzf"><span>Сколько BMS</span><select data-k="want.nbms">
            ${[1, 2, 3, 4].map((n) => `<option value="${n}" ${w.nbms === n ? 'selected' : ''}>${n}</option>`).join('')}</select>
            <small class="muted">Батарей параллельно, у каждой своя BMS</small></label>` : ''}
        </div>
      </div>
      <div class="panel wzform">
        <h2>Напряжение системы</h2>
        <label class="wzf"><span>Батарея</span><select data-k="want.volt">
          ${[12, 24, 48].map((v) => `<option value="${v}" ${w.volt === v ? 'selected' : ''}>${v} В</option>`).join('')}</select>
          <small class="muted">Как на шильдике инвертора. От батареи через DC-DC питаются платы.</small></label>
      </div>` + warn.map((t) => `<p class="warn small">${t}</p>`).join('');
    },

    pick: () => {
      const rec = buyFor(need().inv, need().nb);
      const st = anyStock() ? fromStock() : null;
      const stepper = (k) => `<span class="wzstep"><button class="btn" data-have="${k}" data-d="-1" ${W.have[k] ? '' : 'disabled'}>−</button><b>${W.have[k]}</b><button class="btn" data-have="${k}" data-d="1">+</button></span>`;
      return usbPanel() + `<div class="panel">
        <h2>Рекомендуем</h2>
        ${rec.map(boardRow).join('')}
        ${st ? '' : kitList(rec)}
        ${rec.length === 1 && rec[0].inv && rec[0].nb ? `<p class="muted small">Одна плата на всё — если инвертор и батарея рядом: к инвертору кабель RJ45, до BMS — Bluetooth, ≈ 5–10 м.<br>
          Проверено на объекте: две платы — D1 mini + MAX3232 для инвертора и ESP32-C3 для BMS.</p>` : ''}
      </div>
      <div class="panel">
        <h2>Питание платы</h2>
        ${[
          need().inv && ['inv', 'От инвертора, по тому же кабелю', 'На разъёме RJ45 есть +12 В (пин 4, у части моделей 7) — от него работает фирменный Wi-Fi-модуль. Ничего не откручивать.'],
          ['usb', 'USB-блок питания в розетку', 'Розетка от выхода инвертора. Проще всего, но плата выключится вместе с выходом.'],
          ['batt', 'От батареи через DC-DC', 'Работает всегда, но нужно подключиться к шине или клеммам батареи — с предохранителем.'],
        ].filter(Boolean).map(([v, t, sub]) => `<label class="wzpick"><input type="radio" name="pwr" data-pwr="${v}" ${W.power.src === v || (v === 'usb' && W.power.src === 'inv' && !need().inv) ? 'checked' : ''}>
          <span><b>${t}${v === 'inv' ? ' — рекомендуем' : ''}</b><small class="muted">${sub}</small></span></label>`).join('')}
        ${W.power.src === 'inv' && need().nb && !(rec.length === 1) ? '<p class="muted small">Платы только для BMS к инвертору не подключены — им USB-блок питания.</p>' : ''}
      </div>
      <div class="panel">
        <h2>Что ещё есть</h2>
        <p class="muted small">Помимо воткнутых в USB роутера — платы в ящике.</p>
        ${KINDS.map((k) => {
          const v = W.have[k] ? verdict(k) : null;
          return `<div class="wzhave"><span><b>${CHIPS[k].t}</b>${v ? `<small class="${v[0]}">${v[1]}</small>` : ''}</span>${stepper(k)}</div>`;
        }).join('')}
        ${need().inv ? `<label class="wzpick"><input type="checkbox" data-k="have.max" ${W.have.max ? 'checked' : ''}><span><b>Модуль MAX3232</b><small class="muted">RS232 ↔ 3,3 В, нужен для инвертора</small></span></label>` : ''}
      </div>
      ${st ? `<div class="panel">
        <h2>Ваша сборка</h2>
        <p class="${st.one ? 'ok' : 'muted'} small">${st.one ? 'Всё на одной плате из тех, что есть.' : 'Одной из имеющихся плат не хватит — вот как разложить:'}</p>
        ${st.boards.map(boardRow).join('')}
        ${kitList(st.boards)}
      </div>` : ''}`;
    },

    site: () => `<div class="panel wzform">
      <h2>Объект</h2>
      ${field('Название', 'site.name', 'placeholder="Например: Дача" autocomplete="off"', 'Видно в заголовке и в сообщениях Telegram')}
      <label class="wzf"><span>Часовой пояс</span><select data-k="site.tz"><option value="Europe/Kyiv">Киев (UTC+2, летом UTC+3)</option></select>
        <small class="muted">По нему считаются сутки, ночной тариф и статистика</small></label>
      <p class="muted small">Входить в Solarbox — логином и паролем вашего роутера, отдельный пароль не нужен.</p>
    </div>`,

    build: () => W.plan.map(buildCard).join(''),

    battery: () => {
      const b = W.batt, bms = W.use.jkble;
      const ready = W.plan.some((p, i) => p.nb && W.boards[i]?.adopted);
      let scan = '';
      if (bms) {
        scan = `<div class="panel"><div class="ph"><h2>BMS рядом с платой</h2><button class="btn" id="wz-rescan">Искать ещё раз</button></div>` +
          (!ready ? '<p class="warn">Сначала подключите плату BMS на шаге «Сборка».</p>'
            : W.scan === null ? '<p class="muted"><span class="spin"></span>Ищу по Bluetooth…</p>'
              : !W.scan.length ? '<p class="warn small">Рядом с платой не видно ни одной JK BMS. Проверьте, что BMS включена и рядом (≈5–10 м), и нажмите «Искать ещё раз». Можно идти дальше и выбрать BMS позже.</p>'
              : W.scan.map((s) => `<label class="wzpick ${s.jk ? '' : 'dim'}"><input type="checkbox" data-pack="${s.mac}" ${W.packs.includes(s.mac) ? 'checked' : ''} ${s.jk ? '' : 'disabled'}>
                <span><b>${esc(s.name)}</b><small class="muted">${s.mac} · ${s.rssi} дБм${s.jk ? ' · ' + s.proto : ' · не JK BMS'}</small></span></label>`).join('') +
                `<p class="muted small">Отметьте свои (${W.want.nbms}) — сигнал сильнее у той, что ближе.</p>`) + '</div>';
      }
      return scan + `<div class="panel wzform">
        <h2>Батарея</h2>
        ${b.read ? '<p class="ok small">Ячейки и ёмкость прочитаны из BMS.</p>' : bms ? '' : '<p class="muted small">Без BMS — введите вручную.</p>'}
        <label class="wzf"><span>Химия</span><select data-k="batt.chem"><option value="lfp">LiFePO₄ (LFP)</option></select></label>
        <div class="wzrow">
          ${field('Ячеек последовательно', 'batt.cells', 'type="number" min="4" max="24" step="1" inputmode="numeric"')}
          ${field('Ёмкость, А·ч', 'batt.cap', 'type="number" min="10" max="2000" step="1" inputmode="numeric"')}
        </div>
        <p class="muted small">Батарея ${nominal()} В · ≈ ${(nominal() * b.cap / 1000).toFixed(1)} кВт·ч (номинал)</p>
        ${ctlBms() ? `<h2 class="wzgap">Пороги заряда в BMS</h2>
        <p class="muted small">По ним BMS ставит 100 % и 0 %. Предложены типовые для LFP — поменяйте, если знаете свои.</p>
        <div class="wzrow">
          ${field('SOC 100 %, В/ячейку', 'batt.soc100', 'type="number" min="3.3" max="3.65" step="0.005" inputmode="decimal"', `≈ ${pack(b.soc100)} В на батарею`)}
          ${field('SOC 0 %, В/ячейку', 'batt.soc0', 'type="number" min="2.5" max="3.2" step="0.005" inputmode="decimal"', `≈ ${pack(b.soc0)} В на батарею`)}
        </div>
        <p class="muted small">Мастер пока не записывает пороги в BMS сам: на шаге «Проверка» будет список значений, чтобы задать их в приложении JK.</p>` : ''}
      </div>`;
    },

    inverter: () => {
      const p = W.inv || (W.inv = proposeInv());
      const chk = checkInv(p);
      return `<div class="panel wzform">
        <h2>Инвертор — предложение по батарее</h2>
        <p class="muted small">Посчитано из шага «Батарея» (${W.batt.cells} ячеек LFP, SOC 100 % при ${W.batt.soc100} В). Сейчас в инверторе: <b>прочитаю после подключения платы</b>.</p>
        <label class="wzf"><span>Тип батареи</span><select data-k="inv.type">
          <option value="USE" ${p.type === 'USE' ? 'selected' : ''}>USE — напряжения задаются вручную</option>
          <option value="LIb" ${p.type === 'LIb' ? 'selected' : ''}>LIb — литий по протоколу BMS</option></select></label>
        <div class="wzrow">
          ${field('Заряд (bulk), В', 'inv.bulk', 'type="number" step="0.1" inputmode="decimal"', `${(p.bulk / W.batt.cells).toFixed(3)} В/ячейку`)}
          ${field('Поддержка (float), В', 'inv.float', 'type="number" step="0.1" inputmode="decimal"', `${(p.float / W.batt.cells).toFixed(3)} В/ячейку`)}
        </div>
        ${field('Отключение нагрузки, В', 'inv.cut', 'type="number" step="0.1" inputmode="decimal"', `${(p.cut / W.batt.cells).toFixed(3)} В/ячейку — защита до BMS`)}
        <div class="wzchecks">${chk.map(([ok, t]) => `<div class="${ok ? 'ok' : 'warn'}">${ok ? '✓' : '!'} ${t}</div>`).join('')}</div>
        <p class="muted small">Это предложение, не измерение. Мастер пока не записывает в инвертор сам: значения будут в списке на шаге «Проверка».</p>
      </div>`;
    },

    tariff: () => `<div class="panel wzform">
      <h2>Тариф</h2>
      <p class="muted small">Стоимость считается по энергии, взятой из сети. Счётчика сети в инверторе нет — это оценка по балансу (≈).</p>
      <p class="muted small">Можно оставить пустым и задать позже в «Настройках» — тогда статистика будет без денег.</p>
      ${field('Цена, грн за кВт·ч', 'money.day', 'type="number" min="0" step="0.01" inputmode="decimal"')}
      <label class="wzpick"><input type="checkbox" data-k="money.night" ${W.money.night ? 'checked' : ''}><span><b>Ночной тариф</b><small class="muted">Двухзонный счётчик</small></span></label>
      ${W.money.night ? `<div class="wzrow">
        ${field('Ночью, грн', 'money.nday', 'type="number" min="0" step="0.01" inputmode="decimal"')}
        <label class="wzf"><span>Ночь, часы</span><span class="row"><input data-k="money.from" type="number" min="0" max="23" value="${W.money.from}" style="width:70px"> — <input data-k="money.to" type="number" min="0" max="23" value="${W.money.to}" style="width:70px"></span></label>
      </div>` : ''}
    </div>`,

    telegram: () => `<div class="panel wzform">
      <h2>Уведомления в Telegram</h2>
      <p class="muted small">Необязательно — можно пропустить и подключить позже в «Настройках».</p>
      <ol class="wzlist small">
        <li>В Telegram откройте <b>@BotFather</b> → <code>/newbot</code>, придумайте имя.</li>
        <li>Скопируйте токен сюда.</li>
        <li>После «Применить» напишите своему боту <code>/start</code> и подтвердите себя в «Настройках → Telegram».</li></ol>
      ${field('Токен бота', 'tg.token', 'placeholder="123456:ABC…" autocomplete="off"')}
    </div>`,

    review: () => {
      const r = W.review;
      if (!r) return '<div class="panel"><p class="muted"><span class="spin"></span>Считаю, что изменится…</p></div>';
      const res = W.results || {};
      const groups = {};
      for (const it of r.items) (groups[it.group] = groups[it.group] || []).push(it);
      const row = (it) => {
        const x = res[it.id];
        const mark = !x ? '' : x.ok ? '<span class="ok">✓</span>' : `<span class="warn">✗ ${esc(x.error || '')}</span>`;
        return `<div class="wzitem"><div class="wzit">${esc(it.title)}${mark ? ` <span class="wzmark">${mark}</span>` : ''}</div>
          <div class="small"><span class="muted">${esc(it.old || 'не задано')}</span> → <b>${esc(it.new)}</b></div></div>`;
      };
      const manual = manualItems();
      return (r.items.length ? Object.entries(groups).map(([g, items]) => `<div class="panel"><h2>${esc(g)}</h2>
          ${items.map(row).join('')}</div>`).join('')
        : `<div class="panel"><p class="ok">${W.applied ? 'Всё применено.' : 'Менять нечего — всё уже так.'}</p></div>`) +
        (manual.length ? `<div class="panel"><h2>Вручную</h2>
          <p class="muted small">Запись в BMS и инвертор из мастера ещё не сделана. Задайте эти значения сами (приложение JK BMS, меню инвертора):</p>
          <ul class="wzlist">${manual.map((t) => `<li>${t}</li>`).join('')}</ul></div>` : '') +
        (DEMO ? '<p class="muted small">Тестовый режим: «Применить» ничего не меняет.</p>' : '');
    },


    done: () => `<div class="panel wzhero">
      <h2 class="wzh">Готово</h2>
      ${W.emergency ? `<p>Аварийный пароль Solarbox — для входа с пустым логином, если роутер недоступен: <code>${esc(W.emergency)}</code><br>
        <span class="muted small">Запишите его. Он показан один раз здесь и лежит на роутере в /data/admin-password.</span></p>` : ''}
      <div class="facts wzsum">
        <div><span>Объект</span><b>${esc(W.site.name || '—')}</b></div>
        <div><span>Инвертор</span><b>${W.use.pi30 ? 'PI30' : '—'}</b></div>
        <div><span>BMS</span><b>${W.use.jkble ? W.packs.length + ' × JK' : '—'}</b></div>
        <div><span>Платы</span><b>${W.plan.map((b) => CHIPS[b.chip].t.split(' ')[0]).join(' + ')}</b></div>
        <div><span>Telegram</span><b>${W.tg.token ? 'бот подключён' : 'не подключён'}</b></div>
      </div>
      <p class="muted">Мастер всегда можно открыть снова в «Настройках» — он не перезапишет работающую настройку без вопроса.</p>
    </div>`,
  };

  // Everything to buy for the plan: boards, then power and the inverter cable.
  function kit(boards) {
    const d = DCDC[W.want.volt], out = [], src = pwrSrc;
    for (const b of boards) out.push([CHIPS[b.chip].buy, '1 шт', b.own ? 'есть' : '']);
    const inv = boards.some((b) => b.inv);
    if (inv) out.push(['Модуль MAX3232 (RS232 ↔ 3,3 В)', '1 шт', W.have.max ? 'есть' : '']);
    const by = (s) => boards.filter((b) => src(b) === s).length;
    if (by('inv')) out.push(['DC-DC понижающий MP1584 → 5 В', by('inv') + ' шт', 'от +12 В инвертора']);
    if (by('batt')) {
      out.push([`DC-DC понижающий ${d.m} → 5 В`, by('batt') + ' шт', `батарея до ${d.max} В`]);
      out.push(['Держатель предохранителя + предохранитель 1 А', by('batt') + ' шт', 'у батареи']);
      out.push(['Провод 0,5–0,75 мм², красный и чёрный', '≈ 2 м', 'батарея → DC-DC → плата']);
    }
    if (by('usb')) out.push(['USB-блок питания 5 В, 1 А и кабель к плате', by('usb') + ' шт', '']);
    if (inv) {
      out.push(['Витая пара Cat5e', 'от платы до инвертора', 'жилы 0,2 мм² (24 AWG)']);
      out.push(['Коннектор RJ45 (8P8C) + обжимка', '1 шт', 'или готовый патч-корд, отрезать']);
    }
    out.push(['Провода Dupont мама–мама, термоусадка', '', inv ? 'плата ↔ MAX3232, DC-DC' : '']);
    return out;
  }
  // Where a board takes its power: only the inverter board can use the inverter's +12 V.
  const pwrSrc = (b) => (W.power.src === 'inv' && !b.inv ? 'usb' : W.power.src);
  function kitList(boards) {
    return `<h3>Комплект</h3><table class="wzkit">${kit(boards).map(([t, q, n]) =>
      `<tr><td>${t}${n ? `<small class="${n === 'есть' ? 'ok' : 'muted'}">${n}</small>` : ''}</td><td>${q}</td></tr>`).join('')}</table>`;
  }

  // ---- assembly: per planned board — parts, wiring, flashing, adoption ----
  function buildCard(b, i) {
    const c = CHIPS[b.chip], st = W.boards[i] || {};
    const parts = kit([b]), sec = 2 + (b.inv ? 1 : 0) + (pwrSrc(b) !== 'inv' ? 1 : 0);
    const t = fwTarget(b), have = FIRMWARE.includes(t);
    // The image comes first: without it the board never shows up below.
    // A board in the router's USB is flashed from there; the download is the
    // spare way («прошить с компьютера»).
    const fwBox = b.usb && usbTarget(b) && !pcFlash[i] ? usbBox(b, i) : have
      ? `<div class="wzfw"><div><b>Прошивка для этой платы</b><small class="muted"><code>solarbox-firmware-${t}.bin</code> — собирается один раз для всех таких плат, потом сразу</small>
          ${b.usb && usbTarget(b) ? `<small><a href="#" data-routerflash="${i}">← прошить с роутера</a> — плата воткнута в его USB</small>` : ''}</div>
          <button class="btn on" data-fw="${i}">Скачать прошивку</button></div>`
      : `<div class="wzfw warn"><div><b>Прошивки для такой платы пока нет</b><small>${b.inv ? 'Драйвер инвертора PI30 ещё в работе — плата для инвертора пока не прошивается из Solarbox.' : 'Такое сочетание платы и числа BMS не собирается.'}
          Можно пройти мастер дальше без неё («Подключу позже») и подключить её, когда прошивка появится.</small></div></div>`;
    const flash = `<ol class="wzlist">
        <li>Скачайте прошивку (кнопка выше) — на <b>компьютер</b> с Chrome или Edge: с телефона плату не прошить.</li>
        <li>Откройте <a href="https://web.esphome.io" target="_blank" rel="noopener">web.esphome.io</a>, подключите плату по USB → <b>Connect</b> → <b>Install</b> → выберите файл.</li>
        <li>После прошивки там же <b>Configure Wi-Fi</b> — сеть, в которой стоит роутер с Solarbox.</li>
        <li>Без компьютера под рукой: плата сама поднимает точку <code>solarbox-XXXX</code> — подключитесь с телефона и выберите Wi-Fi там.</li>
      </ol>`;
    let wait;
    if (st.adopted) wait = `<div class="ok"><span class="dot ok"></span>${st.already ? 'Уже подключена' : 'Подключится при «Применить»'}: ${esc(st.found.ip)} · ${esc(st.found.mac)} · прошивка ${esc(st.found.fw)}</div>`;
    else if (st.found) wait = `<div class="wznew"><div><b>Новая плата</b> <span class="muted small">${esc(c.t)} · ${esc(st.found.ip)} · ${esc(st.found.mac)} · прошивка ${esc(st.found.fw)}</span></div>
        <button class="btn on" data-adopt="${i}">Подключить</button></div>`;
    else if (st.later) wait = `<div class="muted">Подключите позже: «Настройки → Мастер настройки». <button class="btn xs" data-later="${i}">Вернуть</button></div>`;
    else wait = `<div class="wznew"><div class="muted">${have ? '<span class="spin"></span>Когда плата прошита и в Wi-Fi, она появится здесь сама.' : 'Плата появится здесь, когда будет прошита.'}${DEMO ? ` <button class="btn xs" data-sim="${i}">имитировать</button>` : ''}</div>
        <button class="btn" data-later="${i}">Подключу позже</button></div>`;
    const open = st.adopted ? '' : 'open';
    return `<div class="panel wzboard">
      <div class="ph"><h2>Плата ${i + 1} · ${c.t}</h2><span class="muted small">${roles(b)}</span></div>
      ${tested(b) ? '' : '<p class="muted small">Такая сборка ещё не проверена на объекте — оценка.</p>'}
      ${fwBox}
      ${b.nb ? '<p class="small">BMS плата слушает по Bluetooth — к ним ничего не паяется. Ставить ближе к батарее (≈ 5–10 м), не в металлический шкаф.</p>' : ''}
      <details ${open}><summary>1. Детали</summary><ul class="wzlist">${parts.map(([t, q, n]) => `<li>${t}${q ? ` — ${q}` : ''}${n ? ` <span class="muted small">(${n})</span>` : ''}</li>`).join('')}</ul></details>
      <details ${open}><summary>2. Плата и выводы</summary>${boardSvg(b)}</details>
      ${b.inv ? `<details ${open}><summary>3. Монтаж: плата, MAX3232${pwrSrc(b) === 'inv' ? ', DC-DC' : ''} и RJ45</summary>${assembly(b)}</details>` : ''}
      ${pwrSrc(b) !== 'inv' ? `<details ${open}><summary>${b.inv ? 4 : 3}. Питание</summary>${power(b)}</details>` : ''}
      <details ${open}><summary>${sec + 1}. Прошивка и Wi-Fi</summary>${flash}</details>
      <h3>${sec + 2}. Плата в сети</h3>${wait}
    </div>`;
  }

  // The board as it is: both pin headers in their real order with the
  // silkscreen labels; the pins we use are coloured and say where they go.
  // Pins this board uses: label → [wire class, where it goes].
  function uses(b) {
    const c = CHIPS[b.chip], src = pwrSrc(b);
    const from = { inv: 'DC-DC OUT', batt: 'DC-DC OUT', usb: '' }[src];
    const u = {};
    if (src !== 'usb') u[c.v5] = ['pwr', '+5 В ' + from + '+'];
    if (src !== 'usb' || b.inv) u[c.gnd] = ['gnd', [src !== 'usb' && from + '−', b.inv && 'MAX3232 GND'].filter(Boolean).join(', ')];
    if (b.inv) Object.assign(u, { [c.v33]: ['pwr', 'MAX3232 VCC'], [c.tx]: ['tx', 'MAX3232 T1IN'], [c.rx]: ['rx', 'MAX3232 R1OUT'] });
    return u;
  }

  // The board as it is: pin headers in their real order with silkscreen labels
  // (inward, as printed). Returns svg and the positions of the used pins.
  function drawBoard(b, bx, top, tags) {
    const c = CHIPS[b.chip], use = uses(b), n = Math.max(c.L.length, c.R.length), step = n > 16 ? 16 : 19, bw = 120;
    const usbTop = c.usb === 'top', mid = b.chip === 'c3', pinTop = top + (usbTop ? 34 : 70);
    const bottom = pinTop + n * step + (usbTop ? 18 : 40), shown = new Set(), at = {};
    const col = (pins, x, right) => pins.map((t, i) => {
      const y = pinTop + i * step, u = !shown.has(t) && use[t];
      if (u) { shown.add(t); at[t] = { x, y, right }; } // one GND is enough
      return `<circle cx="${x}" cy="${y}" r="${u ? 4.5 : 3.5}" class="bpad ${u ? 'w-' + u[0] : ''}"/>` +
        `<text x="${right ? x - 9 : x + 9}" y="${y + 4}" class="blab ${u ? 'on w-' + u[0] : ''}" text-anchor="${right ? 'end' : 'start'}">${t}</text>` +
        (u && tags ? `<polyline points="${right ? `${x + 5},${y} ${x + 16},${y}` : `${x - 5},${y} ${x - 16},${y}`}" class="wire w-${u[0]}"/>` +
          `<text x="${right ? x + 20 : x - 20}" y="${y + 4}" class="btag" text-anchor="${right ? 'start' : 'end'}">${u[1]}</text>` : '');
    }).join('');
    const modW = mid ? 48 : bw - 30, modH = mid ? 30 : 52;
    const modY = mid ? pinTop + (n - 1) * step / 2 - modH / 2 : top + 8, usbY = usbTop ? top - 6 : bottom - 22;
    const svg = `<rect x="${bx}" y="${top}" width="${bw}" height="${bottom - top}" rx="6" class="bpcb"/>
      <rect x="${bx + (bw - modW) / 2}" y="${modY}" width="${modW}" height="${modH}" rx="3" class="bmod"/>
      <text x="${bx + bw / 2}" y="${modY + modH / 2 + 3}" class="bmodt" text-anchor="middle">${c.mod}</text>
      <rect x="${bx + bw / 2 - 16}" y="${usbY}" width="32" height="18" rx="3" class="busb"/>
      <text x="${bx + bw / 2}" y="${usbTop ? usbY - 4 : usbY + 30}" class="blab" text-anchor="middle">USB</text>
      ${col(c.L, bx + 9, false)}${col(c.R, bx + bw - 9, true)}`;
    return { svg, at, bottom, use };
  }
  const BOARD_NOTE = '<p class="muted small">Вид со стороны деталей, подписи — как на плате. У клонов порядок бывает другим — ориентируйтесь по подписям, а не по месту.</p>';
  function boardSvg(b) {
    const d = drawBoard(b, 140, 24, true);
    return `<svg viewBox="0 0 400 ${d.bottom + 10}" class="wsvg bsvg">${d.svg}</svg>` + BOARD_NOTE;
  }

  // Assembly drawing: the real board, MAX3232 module, DC-DC (power from the
  // inverter) and the RJ45 plug with T568B colours; wires routed around.
  const T568B = { 1: ['#f0883e', '#fff', 'бело-оранжевый'], 2: ['#f0883e', null, 'оранжевый'], 3: ['#3fb950', '#fff', 'бело-зелёный'], 4: ['#4ea3ff', null, 'синий'],
    5: ['#4ea3ff', '#fff', 'бело-синий'], 6: ['#3fb950', null, 'зелёный'], 7: ['#a0672e', '#fff', 'бело-коричневый'], 8: ['#a0672e', null, 'коричневый'] };
  function assembly(b) {
    const c = CHIPS[b.chip], viaInv = pwrSrc(b) === 'inv', vp = W.power.vpin;
    const d = drawBoard(b, 46, 30, false), wires = [];
    // MAX3232 module: TTL side (left) and RS232 side (right).
    const mx = 214, my = 60, mt = ['VCC', 'GND', 'T1IN', 'R1OUT'], mr = ['T1OUT', 'R1IN', 'GND'];
    const mpin = (i) => ({ x: mx, y: my + 22 + i * 18 }), rpin = (i) => ({ x: mx + 86, y: my + 31 + i * 18 });
    // DC-DC: OUT on the left (to the board), IN on the right (from RJ45).
    const dx = 214, dy = Math.max(my + 120, d.bottom - 110), dout = (i) => ({ x: dx, y: dy + 24 + i * 22 }), din = (i) => ({ x: dx + 86, y: dy + 24 + i * 22 });
    // RJ45 plug, contacts 1…8 top to bottom.
    const rx = 336, ry = 50, rj = (n) => ({ x: rx, y: ry + 16 + (n - 1) * 16 });
    // Board pins → targets. Left-column pins go round under the board.
    let lane = 0;
    const route = (p, t, cls) => {
      const k = lane++;
      if (p.right) {
        const x1 = 186 + k * 4;
        wires.push(`<polyline points="${p.x + 4},${p.y} ${x1},${p.y} ${x1},${t.y} ${t.x},${t.y}" class="wire ${cls}"/>`);
      } else {
        const x0 = 34 - k * 5, yb = d.bottom + 12 + k * 6, x1 = 186 + k * 4;
        wires.push(`<polyline points="${p.x - 4},${p.y} ${x0},${p.y} ${x0},${yb} ${x1},${yb} ${x1},${t.y} ${t.x},${t.y}" class="wire ${cls}"/>`);
      }
    };
    route(d.at[c.v33], mpin(0), 'w-pwr');
    route(d.at[c.gnd], mpin(1), 'w-gnd');
    route(d.at[c.tx], mpin(2), 'w-tx');
    route(d.at[c.rx], mpin(3), 'w-rx');
    if (viaInv) { route(d.at[c.v5], dout(0), 'w-pwr'); route(d.at[c.gnd], dout(1), 'w-gnd'); }
    // RS232 side → RJ45 (twisted pair colours).
    const pair = (from, n, k) => {
      const t = rj(n), x1 = 308 + k * 5, [col, stripe] = T568B[n];
      const pts = `${from.x + 4},${from.y} ${x1},${from.y} ${x1},${t.y} ${t.x - 4},${t.y}`;
      wires.push(`<polyline points="${pts}" class="wire tp" style="stroke:${col}"/>` + (stripe ? `<polyline points="${pts}" class="wire tps"/>` : ''));
    };
    pair(rpin(1), 1, 0); pair(rpin(0), 2, 1); pair(rpin(2), 8, 2);
    if (viaInv) { pair(din(0), vp, 3); pair(din(1), 8, 4); }
    const box = (x, y, w, h, t) => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="5" class="bpcb"/><text x="${x + w / 2}" y="${y - 6}" class="wtit" text-anchor="middle">${t}</text>`;
    const pad = (p, t, right, cls) => `<circle cx="${p.x}" cy="${p.y}" r="4" class="bpad ${cls || ''}"/><text x="${right ? p.x - 8 : p.x + 8}" y="${p.y + 3.5}" class="blab" text-anchor="${right ? 'end' : 'start'}">${t}</text>`;
    const h = Math.max(d.bottom + 12 + lane * 6 + 10, dy + 90, ry + 150);
    const svg = `<svg viewBox="0 0 400 ${h}" class="wsvg">
      ${wires.join('')}
      ${d.svg}
      ${box(mx, my, 86, 90, 'MAX3232')}
      ${mt.map((t, i) => pad(mpin(i), t, false)).join('')}${mr.map((t, i) => pad(rpin(i), t, true)).join('')}
      ${viaInv ? box(dx, dy, 86, 70, 'DC-DC') + pad(dout(0), 'OUT+', false, 'w-pwr') + pad(dout(1), 'OUT−', false) + pad(din(0), 'IN+', true, 'w-pwr') + pad(din(1), 'IN−', true) +
        `<text x="${dx + 43}" y="${dy + 82}" class="blab" text-anchor="middle">→ 5,0 В</text>` : ''}
      <rect x="${rx}" y="${ry}" width="44" height="${16 * 8 + 4}" rx="4" class="rj"/><text x="${rx + 22}" y="${ry - 6}" class="wtit" text-anchor="middle">RJ45</text>
      ${[1, 2, 3, 4, 5, 6, 7, 8].map((n) => { const p = rj(n), on = [1, 2, 8].includes(n) || (viaInv && n === vp); return `<rect x="${rx + 3}" y="${p.y - 5}" width="18" height="10" rx="2" style="fill:${T568B[n][0]};opacity:${on ? 1 : 0.25}"/>` +
        (T568B[n][1] ? `<rect x="${rx + 3}" y="${p.y - 1.5}" width="18" height="3" style="fill:#fff;opacity:${on ? 0.9 : 0.25}"/>` : '') +
        `<text x="${rx + 26}" y="${p.y + 3.5}" class="blab ${on ? 'on' : ''}">${n}</text>`; }).join('')}
      <text x="${rx + 22}" y="${ry + 16 * 8 + 18}" class="blab" text-anchor="middle">T568B</text>
    </svg>`;
    const rows = [
      [`плата ${c.v33}`, 'MAX3232 VCC', 'Dupont'], [`плата ${c.gnd}`, 'MAX3232 GND', 'Dupont'],
      [`плата ${c.tx}`, 'MAX3232 T1IN', 'Dupont'], [`плата ${c.rx}`, 'MAX3232 R1OUT', 'Dupont'],
      ['MAX3232 R1IN', 'RJ45 1 — TX инвертора', tp(1)], ['MAX3232 T1OUT', 'RJ45 2 — RX инвертора', tp(2)], ['MAX3232 GND', 'RJ45 8 — GND', tp(8)],
    ];
    if (viaInv) rows.push(['RJ45 ' + vp + ' — +12 В инвертора', 'DC-DC IN+', tp(vp)], ['RJ45 8 — GND', 'DC-DC IN−', tp(8) + ' (вторая жила к той же 8 или общий провод)'],
      ['DC-DC OUT+', `плата ${c.v5}`, 'Dupont'], ['DC-DC OUT−', `плата ${c.gnd}`, 'Dupont']);
    return svg + conn(rows) + `
    <p class="small"><b>Проверка мультиметром на разъёме инвертора</b> (щуп «−» на пин 8):</p>
    <ul class="wzlist small">
      <li>пин 1 → ≈ −5…−12 В: это TX инвертора, всё верно.</li>
      ${viaInv ? `<li>пин 4 или 7 → ≈ +12 В: здесь питание. Выберите, где оно у вас:
        <span class="seg two wzvp"><button class="btn ${vp === 4 ? 'on' : ''}" data-vpin="4">пин 4 (синий)</button><button class="btn ${vp === 7 ? 'on' : ''}" data-vpin="7">пин 7 (бело-коричн.)</button></span></li>
      <li>Нет напряжения ни на 4, ни на 7 — питание от этого инвертора не взять: вернитесь к «Платы» и выберите USB-блок или батарею.</li>` : ''}
    </ul>
    ${viaInv ? '<p class="warn small">На DC-DC до подключения платы выставьте 5,0 В подстроечником, по мультиметру.</p>' : ''}
    <p class="muted small">Витая пара — жилы 0,2 мм² (24 AWG), RS232 на 2400 бод спокойно идёт на 10–15 м. Неиспользуемые жилы не подключать.
    ${viaInv ? 'Сколько тока даёт +12 В разъёма, производители не пишут; плата берёт ≈ 0,05–0,25 А от 12 В (оценка) — фирменный Wi-Fi-модуль питается так же.' : ''}
    Подписи на модулях MAX3232 бывают разные — на схеме названия выводов микросхемы. Если данных нет — поменяйте местами ${c.tx} и ${c.rx}: перепутать не опасно.<br>
    ${b.chip === 'd1' ? 'D1 mini прошивайте <b>до</b> пайки к MAX3232: TX/RX — те же выводы, что у USB.' : `${c.tx}/${c.rx} — это ${c.gpio}, предложение распиновки Solarbox, ещё не проверено на плате.`}</p>`;
  }
  const tp = (n) => `витая пара, <span class="wcol" style="--c:${T568B[n][0]};--s:${T568B[n][1] || T568B[n][0]}"></span>${T568B[n][2]}`;

  // Connection table: from → to, wire, cross-section.
  const conn = (rows) => `<table class="wzconn"><tr><th>Откуда</th><th>Куда</th><th>Провод</th></tr>${rows.map(([a, b, w]) =>
    `<tr><td>${a}</td><td>${b}</td><td>${w}</td></tr>`).join('')}</table>`;

  // Battery → fuse → DC-DC → board 5 V, or a USB charger.
  function power(b) {
    const c = CHIPS[b.chip], d = DCDC[W.want.volt];
    if (pwrSrc(b) === 'usb') return `<p class="small">USB-блок питания 5 В, 1 А в розетку и кабель в USB платы. Розетка — от выхода инвертора, так что при отключении выхода плата тоже выключится.</p>`;
    return `<svg viewBox="0 0 360 150" class="wsvg">
      <rect x="6" y="30" width="70" height="90" rx="8" class="wbox"/><text x="41" y="22" class="wtit" text-anchor="middle">АКБ ${W.want.volt} В</text>
      <rect x="104" y="40" width="40" height="18" rx="4" class="wbox"/><text x="124" y="53" class="wlab" text-anchor="middle">1 А</text>
      <rect x="168" y="30" width="86" height="90" rx="8" class="wbox"/><text x="211" y="22" class="wtit" text-anchor="middle">DC-DC</text>
      <rect x="284" y="30" width="70" height="90" rx="8" class="wbox"/><text x="319" y="22" class="wtit" text-anchor="middle">плата</text>
      ${wire('76,49 104,49', 'pwr')}${wire('144,49 168,49', 'pwr')}${wire('76,101 168,101', 'gnd')}
      ${wire('254,49 284,49', 'pwr')}${wire('254,101 284,101', 'gnd')}
      ${pin(76, 49, '+', true)}${pin(76, 101, '−', true)}
      ${pin(168, 49, 'IN+')}${pin(168, 101, 'IN−')}${pin(254, 49, 'OUT+', true)}${pin(254, 101, 'OUT−', true)}
      ${pin(284, 49, c.v5)}${pin(284, 101, c.gnd)}
      <text x="211" y="80" class="wlab" text-anchor="middle">→ 5,0 В</text>
      <text x="122" y="140" class="wlab" text-anchor="middle">0,5–0,75 мм²</text><text x="270" y="140" class="wlab" text-anchor="middle">0,5 мм²</text>
    </svg>
    ${conn([
      ['АКБ + (шина или клемма инвертора)', 'предохранитель 1 А → DC-DC IN+', '0,5–0,75 мм², красный'],
      ['АКБ −', 'DC-DC IN−', '0,5–0,75 мм², чёрный'],
      ['DC-DC OUT+', `плата ${c.v5}`, '0,5 мм², красный'],
      ['DC-DC OUT−', `плата ${c.gnd}`, '0,5 мм², чёрный'],
    ])}
    <p class="warn small">До подключения платы выставьте на DC-DC 5,0 В подстроечником (по мультиметру) — новые модули часто выдают сколько угодно.</p>
    <p class="muted small">DC-DC: ${d.m} — батарея ${W.want.volt} В при полном заряде до ${d.max} В. Предохранитель ставить у батареи, а не у платы: он защищает провод.</p>`;
  }

  const pin = (x, y, t, right) => `<circle cx="${x}" cy="${y}" r="3.2" class="wpin"/><text x="${right ? x - 7 : x + 7}" y="${y + 4}" class="wlab" text-anchor="${right ? 'end' : 'start'}">${t}</text>`;
  const wire = (pts, cls) => `<polyline points="${pts}" class="wire ${cls}"/>`;

  // ---- inverter proposal from the battery ----
  function proposeInv() {
    const b = W.batt;
    return { type: 'USE', bulk: pack(b.soc100 + 0.03), float: pack(3.375), cut: pack(Math.max(2.9, b.soc0)) };
  }
  function checkInv(p) {
    const b = W.batt, out = [];
    out.push(p.bulk / b.cells > b.soc100 ? [1, `Заряд ${p.bulk} В выше порога SOC 100 % (${pack(b.soc100)} В) — BMS дойдёт до 100 %`]
      : [0, `Заряд ${p.bulk} В не выше порога SOC 100 % (${pack(b.soc100)} В) — BMS никогда не покажет 100 %`]);
    out.push(p.bulk / b.cells <= 3.55 ? [1, 'Заряд не выше 3,55 В на ячейку'] : [0, `Заряд ${(p.bulk / b.cells).toFixed(2)} В на ячейку — много для LFP`]);
    out.push(p.float < p.bulk ? [1, 'Поддержка ниже заряда'] : [0, 'Поддержка должна быть ниже заряда']);
    out.push(p.cut / b.cells >= b.soc0 - 0.05 ? [1, 'Отключение около порога SOC 0 %'] : [0, 'Отключение заметно ниже SOC 0 % — сработает BMS, а не инвертор']);
    return out;
  }

  // ---- simulated hardware (test mode) ----
  const sims = {};
  function simulate(i, now) {
    if (W.boards[i]?.found || sims[i]) return;
    sims[i] = setTimeout(() => {
      const n = 21 + i;
      W.boards[i] = { found: { fw: '2026.10.1', mac: `02:00:00:00:00:${n}`, ip: `192.168.88.${n}` } };
      if (cur().id === 'build') render();
      toast(`Найдена новая плата: ${CHIPS[W.plan[i].chip].t}`);
    }, now ? 300 : 6000 + i * 2000);
  }
  function scanBle() {
    W.scan = null; render();
    setTimeout(() => {
      W.scan = [
        { name: 'JK_B2A8S20P', mac: '02:00:00:00:B1:01', rssi: -61, jk: true, proto: 'JK02_32S' },
        { name: 'JK_B2A8S20P', mac: '02:00:00:00:B1:02', rssi: -74, jk: true, proto: 'JK02_32S' },
        { name: 'JK_B2A16S20P', mac: '02:00:00:00:B1:03', rssi: -79, jk: true, proto: 'JK02_32S' },
        { name: 'Mi Smart Band', mac: '02:00:00:00:C3:55', rssi: -88, jk: false },
      ];
      if (cur().id === 'battery') render();
    }, 2500);
  }

  // ---- the server: draft, discovered boards, plan ----
  const KEEP = ['i', 'want', 'power', 'have', 'plan', 'boards', 'site', 'packs', 'batt', 'inv', 'tg', 'money', 'usbSkip', 'wifi'];
  // Device names in Solarbox, in order over all BMS boards: bms, bms2, bms3…
  function draft() {
    const ui = {};
    for (const k of KEEP) ui[k] = W[k];
    const adopt = [];
    let nbms = 0, left = W.packs.slice();
    W.plan.forEach((b, i) => {
      const f = W.boards[i]?.found;
      if (!f || !W.boards[i].adopted) return;
      const firstBms = b.nb && !adopt.some((a) => a.slots.length);
      const slots = left.splice(0, b.nb).map((mac, k) => {
        nbms++;
        const s = W.scan?.find((x) => x.mac === mac);
        return { slot: k + 1, bms_mac: mac, proto: s?.proto || 'JK02_32S', device: nbms === 1 ? 'bms' : 'bms' + nbms };
      });
      // Named by what it runs now (a board meant for both may have the BMS part only yet).
      const inv = b.inv && (f.types || [f.type]).includes('pi30');
      adopt.push({ mac: f.mac, ip: f.ip, chip: f.chip || CHIPS[b.chip].t, title: inv ? 'Инвертор' : 'АКБ · JK BMS',
        name: inv ? 'inverter' : firstBms ? 'bms' : 'bms-board' + (i + 1), slots });
    });
    return { ui, site: { name: W.site.name }, money: { day: +W.money.day || 0, night: !!W.money.night, nday: +W.money.nday || 0, from: +W.money.from, to: +W.money.to },
      tg: { token: W.tg.token.trim() }, adopt };
  }
  // Debounced: every render asks to save; whoever waits gets the answer of
  // the save that actually went out after their call.
  let saving, waiting = [];
  function save() {
    if (DEMO) return Promise.resolve();
    clearTimeout(saving);
    return new Promise((res) => {
      waiting.push(res);
      saving = setTimeout(() => {
        const done = waiting; waiting = [];
        api('/api/wizard', { method: 'PUT', json: { draft: draft() } })
          .catch((e) => toast('Черновик не сохранился: ' + e.message, true))
          .then(() => done.forEach((f) => f()));
      }, 300);
    });
  }

  // Boards that announced themselves → the planned ones, by what they run.
  let disc = [];
  async function poll() {
    if (DEMO || !['build', 'battery'].includes(cur().id)) return;
    try { disc = await api('/api/discovered'); } catch (e) { return; }
    const claimed = new Set(Object.values(W.boards).map((x) => x.found?.mac));
    let changed = false;
    W.plan.forEach((b, i) => {
      if (W.boards[i]?.found) {
        const d = disc.find((x) => x.mac === W.boards[i].found.mac);
        if (d && d.ip !== W.boards[i].found.ip) { W.boards[i].found.ip = d.ip; changed = true; }
        return;
      }
      const want = b.inv ? 'pi30' : 'jk-ble';
      const um = b.usb && (() => { const p = usb.ports.find((x) => x.port === b.usb); return (p?.board?.mac || p?.serial || '').toUpperCase(); })();
      const d = um ? disc.find((x) => x.mac === um && x.age < 120)
        : disc.find((x) => (x.types || [x.type]).includes(want) && !claimed.has(x.mac) && x.age < 120);
      if (!d) return;
      claimed.add(d.mac);
      W.boards[i] = { found: { mac: d.mac, ip: d.ip, fw: d.fw || '?', type: d.type, types: d.types, chip: d.chip || '' }, adopted: !!d.adopted_as, already: d.adopted_as };
      toast(`Найдена новая плата: ${d.ip}`);
      changed = true;
    });
    // BLE scan results of our BMS boards.
    const scan = [];
    W.plan.forEach((b, i) => {
      const d = b.nb && disc.find((x) => x.mac === W.boards[i]?.found?.mac);
      for (const f of d?.found || []) if (!scan.some((s) => s.mac === f.mac)) scan.push({ name: f.name, mac: f.mac, rssi: f.rssi, jk: /^JK/i.test(f.name), proto: 'JK02_32S' });
    });
    if (scan.length || W.plan.some((b, i) => b.nb && W.boards[i]?.found)) {
      scan.sort((a, b) => b.rssi - a.rssi);
      if (JSON.stringify(scan) !== JSON.stringify(W.scan)) { W.scan = scan; changed = true; }
    }
    if (changed) render();
  }
  setInterval(poll, 3000);

  // Firmware for a planned board: built on demand by the relay, reused after.
  // Library images (relay /v1/firmware/…): JK BMS boards, with or without the PI30 inverter.
  const FIRMWARE = ['c3-jk1', 'c3-jk2', 's3-jk1', 's3-jk2', 's3-jk3', 'esp32-jk1', 'esp32-jk2', 'esp32-jk3',
    'c3-pi30-jk1', 's3-pi30-jk1', 's3-pi30-jk2', 's3-pi30-jk3', 'esp32-pi30-jk1', 'esp32-pi30-jk2', 'esp32-pi30-jk3'];
  const fwTarget = (b) => `${b.chip}${b.inv ? '-pi30' : ''}${b.nb ? '-jk' + b.nb : ''}`;
  // A board in the router's USB: the full image if the library has it, else
  // its BMS part now (the inverter comes later over the air).
  const usbTarget = (b) => (FIRMWARE.includes(fwTarget(b)) ? fwTarget(b) : b.nb && FIRMWARE.includes(`${b.chip}-jk${b.nb}`) ? `${b.chip}-jk${b.nb}` : null);
  async function firmware(b, btn) {
    const t = fwTarget(b);
    btn.disabled = true;
    try {
      for (let k = 0; k < 80; k++) {
        const r = await fetch(`${RELAY}/v1/firmware/${t}`).then((x) => x.json());
        if (r.status === 'ready') { location.href = r.files.bin.url; btn.textContent = 'Скачано'; return; }
        if (r.status !== 'building') throw new Error(r.error || 'сборка не удалась');
        btn.textContent = 'Собирается… (≈3–5 мин, один раз для всех)';
        await new Promise((ok) => setTimeout(ok, 15000));
      }
      throw new Error('сборка идёт слишком долго');
    } catch (e) {
      toast('Прошивка: ' + e.message, true);
      btn.textContent = 'Скачать прошивку';
    } finally { btn.disabled = false; }
  }

  function manualItems() {
    const out = [];
    if (ctlBms()) out.push(`BMS: SOC 100 % при ${W.batt.soc100} В/ячейку (${pack(W.batt.soc100)} В), SOC 0 % при ${W.batt.soc0} В/ячейку (${pack(W.batt.soc0)} В)`);
    if (ctlInv() && W.inv) out.push(`Инвертор: тип батареи ${W.inv.type}, заряд ${W.inv.bulk} В, поддержка ${W.inv.float} В, отключение нагрузки ${W.inv.cut} В`);
    return out;
  }

  async function loadReview() {
    W.review = null; render();
    if (DEMO) {
      W.review = { items: [{ id: 'site', group: 'Solarbox', title: 'Название объекта', old: '', new: W.site.name || '—' }] };
    } else {
      await save();
      try { W.review = await api('/api/wizard/plan'); } catch (e) { toast(e.message, true); W.review = { items: [] }; }
    }
    render();
  }

  async function applyAll(only) {
    if (DEMO) { toast('Тестовый режим: ничего не применяется'); W.applied = true; return true; }
    const r = await api('/api/wizard/apply', { json: only ? { only } : {} });
    W.results = Object.assign(W.results || {}, Object.fromEntries(r.results.map((x) => [x.id, x])));
    W.applied = r.left === 0;
    const failed = r.results.filter((x) => !x.ok).length;
    toast(failed ? `Не получилось: ${failed}. Остальное применено — можно повторить.` : 'Применено', !!failed);
    render();
    return W.applied;
  }

  // ---- boards in the router's USB ----
  // Plugged and supported = used (Q: «воткнул — значит, используешь»). The
  // card's state is worked out from facts every time — what is in the USB,
  // what the server's flash job says, what talks to Solarbox on the network —
  // so a reload, a Solarbox restart or a pulled cable can't confuse it.
  const CHIP_KEY = { esp32s3: 's3', esp32c3: 'c3', esp32: 'esp32' };
  const pcFlash = {}; // plan index → owner chose to flash it from a computer
  const T = { identify: 20e3, flash: 120e3, wifi: 60e3, appear: 300e3 }; // «застряло» after this
  let usb = { ports: [] }, usbBusy = false, restarting = false, wifiPw = '';
  const nets = {};      // port → networks the board sees (its own scan) | 'scanning'
  const since = {};     // port → when identifying / flashing progress / Wi-Fi was last seen moving
  const wifiFails = {}; // port → failed attempts (2 → offer the board's own access point)
  const wifiDone = {};  // port → when the board took the Wi-Fi
  let wantRestart = false;

  // Supported, identified, not set aside: [{port, key, mac}]
  function usbBoards() {
    return usb.ports.filter((p) => p.board && CHIP_KEY[p.board.chip] && !W.usbSkip[p.port])
      .map((p) => ({ port: p.port, key: CHIP_KEY[p.board.chip], mac: p.board.mac }));
  }
  const portMac = (p) => (p?.board?.mac || p?.serial || '').toUpperCase();
  const online = (mac) => mac && disc.find((d) => d.mac === mac && d.age < 120);

  async function pollUsb() {
    if (DEMO || !['pick', 'build'].includes(cur().id) || usbBusy) return;
    usbBusy = true;
    try {
      const fresh = await api('/api/usb');
      restarting = false;
      for (const p of fresh.ports) { const o = usb.ports.find((x) => x.port === p.port); if (o) { p.tried = o.tried; p.error = p.board ? null : o.error; } }
      usb = fresh;
      // A board plugged in after Solarbox started: it restarts to reach it (≈10 s).
      if (usb.restart_needed && !wantRestart) {
        wantRestart = restarting = true;
        await api('/api/usb/restart', { method: 'POST' }).catch(() => {});
      } else if (!usb.restart_needed) {
        wantRestart = false;
        const p = usb.ports.find((x) => x.node && !x.board && !x.tried);
        if (p) {
          p.tried = Date.now(); render();
          try { p.board = (await api('/api/usb/identify', { json: { port: p.port } })).board; }
          catch (e) { p.error = e.message; }
        }
      }
    } catch (e) {
      if (!wantRestart) usb.error = e.message; // during the restart the server is away for a moment
      else restarting = true;
    } finally { usbBusy = false; }
    bindPlugged();
    if (!document.activeElement || !document.activeElement.matches('input, select')) render();
  }
  setInterval(pollUsb, 3000);

  // Plugged in on «Сборка»: take the planned board of that chip that has no board yet.
  function bindPlugged() {
    if (cur().id !== 'build') return;
    const used = new Set(W.plan.map((b) => b.usb).filter(Boolean));
    for (const u of usbBoards()) {
      if (used.has(u.port)) continue;
      const b = W.plan.find((x) => !x.usb && x.chip === u.key && !W.boards[W.plan.indexOf(x)]?.found);
      if (b) { b.usb = u.port; b.own = true; used.add(u.port); }
    }
  }

  // «Платы»: what is in the router's USB and what each one is for.
  function usbPanel() {
    if (DEMO) return '';
    const plan = anyStock() ? fromStock().boards : [];
    const rows = usb.ports.map((p) => {
      const b = p.board, key = b && CHIP_KEY[b.chip], k = plan.findIndex((x) => x.usb === p.port), sb = p.solarbox;
      const what = b ? `${CHIPS[key]?.t || b.chip.toUpperCase()} · ${b.flash} · MAC ${esc(b.mac)}` : esc(p.kind) + (p.serial ? ` · ${esc(p.serial)}` : '');
      let note, btn = '';
      if (restarting && !p.node) note = '<span class="spin"></span>подключаюсь к новой плате (≈10 с)…';
      else if (p.error) note = `<span class="warn">${esc(p.error)}</span> <button class="btn xs" data-usbretry="${p.port}">Повторить</button>`;
      else if (!b) note = Date.now() - (p.tried || Date.now()) > T.identify
        ? '<span class="warn">не отвечает — другой кабель или порт; не помогло — зажмите на плате BOOT и нажмите «Повторить»</span>' : '<span class="spin"></span>опознаю…';
      else if (!key) note = '<span class="warn">не поддерживается — нужна ESP32-C3, ESP32-S3 или ESP32</span>';
      else if (W.usbSkip[p.port]) { note = 'не используется'; btn = `<button class="btn xs" data-usbskip="${p.port}">Использовать</button>`; }
      else {
        note = k >= 0 ? `плата ${k + 1}: ${roles(plan[k])}` : 'лишняя — для плана хватает других';
        btn = `<button class="btn xs" data-usbskip="${p.port}">Не использовать</button>`;
      }
      if (sb && key) note += ` · уже с прошивкой Solarbox ${esc(sb.fw)}${sb.age < 120 ? ', в сети' : ''}`;
      if (!p.tested) note += ' · <span class="muted">такой USB ещё не проверен</span>';
      return `<div class="wzbrow"><span class="wznum">USB</span><span><b>${what}</b><small class="muted">${note}</small></span>${btn}</div>`;
    }).join('');
    return `<div class="panel"><h2>Воткнуто в USB роутера</h2>
      ${rows || '<p class="muted small">Ничего. Воткните плату в USB роутера — Solarbox сам её опознает, прошьёт и даст ей Wi-Fi; потом её можно отключить и нести на место.</p>'}
      ${rows ? '<p class="muted small">Воткнутые платы идут в план первыми. Остальное — из «что уже есть» или купить.</p>' : ''}</div>`;
  }

  // «Сборка»: the board's way from the USB to the network, one main button at a time.
  function usbBox(b, i) {
    const p = usb.ports.find((x) => x.port === b.usb), t = usbTarget(b), j = p?.job, st = W.boards[i] || {};
    const mac = portMac(p) || st.found?.mac, live = online(mac);
    const head = (s) => `<div class="wzfw"><div style="flex:1"><b>Плата в USB роутера</b><small class="muted">${esc(mac || b.usb)} · прошивка <code>${t}</code></small>${partial}${s}</div></div>`;
    const partial = t !== fwTarget(b) ? '<p class="small warn">Прошивки с инвертором для неё нет — прошьётся для BMS, инвертор добавится обновлением по воздуху.</p>' : '';
    if (!p) {
      if (live) return ''; // carried away and on the network: the card below takes over
      if (wifiDone[b.usb] || j?.state === 'done') return head(Date.now() - (wifiDone[b.usb] || Date.now()) > T.appear
        ? '<p class="warn small">Плата не появилась в сети за 5 минут. Включена ли она, ловит ли Wi-Fi на месте? Можно вернуть её в USB роутера и дать Wi-Fi заново.</p>'
        : '<div class="muted"><span class="spin"></span>Плату вынули — жду её в сети…</div>');
      return head('<p class="warn small">Плату вынули из USB роутера. Воткните её обратно — продолжим с того же места.</p>');
    }
    const flashBtn = (label) => `<button class="btn on" data-usbflash="${i}">${label}</button>`;
    // Running Solarbox already and on the network: nothing to do but carry it.
    if (p.solarbox && live && !(j && ['fetching', 'building', 'flashing', 'verifying'].includes(j.state)))
      return head(`<div class="ok">✓ Уже с прошивкой Solarbox ${esc(p.solarbox.fw)} и в сети (${esc(live.ip)}). Можно отключать и нести на место.</div>
        <p class="small"><button class="btn xs" data-usbflash="${i}">Прошить заново</button> <span class="muted">обычно не нужно: обновляется по воздуху</span></p>`);
    if (j && ['fetching', 'building', 'flashing', 'verifying'].includes(j.state)) {
      const key = j.state + (j.pct || 0);
      if (since[b.usb]?.key !== key) since[b.usb] = { key, at: Date.now() };
      const stuck = j.state !== 'building' && Date.now() - since[b.usb].at > T.flash;
      return head(stuck ? `<p class="warn small">Прошивка не двигается 2 минуты — похоже, оборвалась.</p>${flashBtn('Повторить')}`
        : `<div class="muted"><span class="spin"></span>${{ fetching: 'Получаю прошивку…', building: 'Прошивка собирается (≈3–5 мин, один раз для всех таких плат)…', flashing: `Прошиваю… ${j.pct || 0} %`, verifying: 'Проверяю записанное…' }[j.state]}</div>`);
    }
    if (j?.state === 'failed') return head(`<p class="warn small">Прошивка не закончилась: ${esc(j.error)}. Воткните плату и нажмите ещё раз — испортить её так нельзя.</p>${flashBtn('Повторить')}`);
    if (j?.state === 'done' || (p.solarbox && !live)) {
      if (wifiDone[b.usb]) return head(live ? `<div class="ok">✓ Прошита и в Wi-Fi (${esc(live.ip)}). Можно отключать от роутера и нести на место.</div>`
        : '<div class="muted"><span class="spin"></span>Плата подключается к Wi-Fi…</div>');
      return head(`<div class="ok">✓ Прошита${j?.version ? `: ${esc(j.target)} ${esc(j.version)}` : ''}.</div>${wifiForm(b, i)}`);
    }
    return head(`${flashBtn('Прошить')}
      <p class="small muted">Или <a href="#" data-pcflash="${i}">прошить с компьютера</a>.</p>`);
  }

  // The board scans the air itself; a list, not a text field — browsers took
  // «network + password» for a login form and filled in the router's.
  function wifiForm(b, i) {
    const n = nets[b.usb];
    if (!n) scanNets(b.usb);
    if (!n || n === 'scanning') return '<div class="muted"><span class="spin"></span>Плата ищет сети Wi-Fi вокруг себя…</div>';
    const other = !!W.wifi.other;
    if (!other && W.wifi.ssid && !n.some((x) => x.ssid === W.wifi.ssid)) W.wifi.ssid = '';
    const none = !n.length && !n.error
      ? `<p class="small warn">Плата не видит ни одной сети. Если на ней раньше была своя прошивка с сохранённым Wi-Fi, она не сканирует эфир — <a href="#" data-usbflashlink="${i}">прошить заново</a> (настройки платы сотрутся). Или введите сеть вручную: «Другая сеть…».</p>` : '';
    const ap = (wifiFails[b.usb] || 0) >= 2
      ? '<p class="small warn">Не выходит дважды. Другой способ: плата раздаёт точку <code>solarbox-XXXX</code> — подключитесь к ней с телефона и выберите сеть там; плата появится ниже сама.</p>' : '';
    return `<div class="wzrow"><label class="wzf"><span>Wi-Fi для платы — сети, которые видит сама плата: ${n.length}</span><select id="wz-wifissid">
        <option value="">— выберите сеть —</option>
        ${n.map((x) => `<option value="${esc(x.ssid)}" ${!other && x.ssid === W.wifi.ssid ? 'selected' : ''}>${esc(x.ssid)} · ${x.rssi} дБм${x.secured ? '' : ' · открытая'}</option>`).join('')}
        <option value="\u0000other" ${other ? 'selected' : ''}>Другая сеть…</option></select>
        ${n.error ? `<small class="warn">${esc(n.error)}</small>` : ''}</label>
      ${other ? `<label class="wzf"><span>Имя сети</span><input id="wz-wifiother" value="${esc(W.wifi.ssid)}" autocomplete="off" data-lpignore="true" data-1p-ignore></label>` : ''}
      <label class="wzf"><span>Пароль Wi-Fi</span><input id="wz-wifipw" type="password" autocomplete="new-password" data-lpignore="true" data-1p-ignore value="${esc(wifiPw)}"></label></div>
      <div class="row"><button class="btn on" data-usbwifi="${i}">Дать плате Wi-Fi</button><button class="btn" data-rescan="${b.usb}">Искать ещё раз</button></div>
      ${none}<p class="muted small">Плата видит сети здесь, у роутера; на месте нужна та, что ловится там. Пароль уйдёт в плату, Solarbox его не хранит.</p>${ap}`;
  }

  async function scanNets(port) {
    if (nets[port] === 'scanning') return;
    nets[port] = 'scanning';
    try { nets[port] = (await api('/api/usb/scan', { json: { port } })).networks; }
    catch (e) { nets[port] = Object.assign([], { error: e.message }); }
    render();
  }
  async function usbFlash(i) {
    const b = W.plan[i];
    delete wifiDone[b.usb]; delete nets[b.usb];
    try {
      await api('/api/usb/flash', { json: { port: b.usb, target: usbTarget(b) } });
      // While the library builds the image, ask again until it is there.
      for (;;) {
        await new Promise((ok) => setTimeout(ok, 4000));
        const j = (await api('/api/usb')).ports.find((x) => x.port === b.usb)?.job;
        if (j?.state === 'building') { await new Promise((ok) => setTimeout(ok, 15000)); return usbFlash(i); }
        if (!j || !['fetching', 'flashing', 'verifying'].includes(j.state)) break;
      }
    } catch (e) { toast('Прошивка: ' + e.message, true); }
    render();
  }
  async function usbWifi(i, btn) {
    const port = W.plan[i].usb, ssid = (W.wifi.ssid || '').trim();
    if (!ssid) return toast('Выберите сеть Wi-Fi', true);
    btn.disabled = true; btn.textContent = 'Плата подключается… (до минуты)';
    try {
      // The board reaches Solarbox at this router's address, plain HTTP, port 8080.
      await api('/api/usb/wifi', { json: { port, ssid, password: wifiPw, solarbox: `http://${location.hostname}:8080` } });
      wifiPw = ''; wifiDone[port] = Date.now(); wifiFails[port] = 0;
      toast('Плата в Wi-Fi');
    } catch (e) { wifiFails[port] = (wifiFails[port] || 0) + 1; toast(e.message, true); }
    render();
  }

  // ---- validation per step ----
  function invalid(id) {
    if (id === 'goals' && !W.use.pi30 && !W.use.jkble)
      return W.want.invType === 'none' && W.want.bmsType === 'none' ? 'Выберите, что подключаем' : 'Пока нечего подключать: выбранное оборудование не поддерживается';
    if (id === 'site' && !W.site.name.trim()) return 'Введите название объекта';
    if (id === 'build') {
      const left = W.plan.map((b, i) => (W.boards[i]?.adopted || W.boards[i]?.later ? 0 : i + 1)).filter(Boolean);
      if (left.length) return 'Ещё не подключена плата ' + left.join(', ');
    }
    if (id === 'battery') {
      // None in range yet (another room, BMS asleep): go on, pick them later.
      const jk = (W.scan || []).filter((x) => x.jk).length;
      if (W.use.jkble && jk && W.packs.length !== Math.min(W.want.nbms, jk)) return `Отметьте свои BMS: ${Math.min(W.want.nbms, jk)}`;
      if (!(W.batt.cells >= 4 && W.batt.cap > 0)) return 'Проверьте ячейки и ёмкость';
      if (ctlBms() && !(W.batt.soc100 > W.batt.soc0)) return 'SOC 100 % должен быть выше SOC 0 %';
    }
    if (id === 'telegram' && W.tg.token.trim() && !/^\d+:[\w-]{20,}$/.test(W.tg.token.trim())) return 'Токен выглядит не так: цифры, двоеточие, буквы';
    return '';
  }

  // Leaving «Платы»: fix the plan; a changed plan forgets adopted boards.
  function fixPlan() {
    const p = anyStock() ? fromStock().boards : buyFor(need().inv, need().nb);
    if (JSON.stringify(p) !== JSON.stringify(W.plan)) { W.plan = p; W.boards = {}; for (const k in sims) { clearTimeout(sims[k]); delete sims[k]; } }
  }

  // ---- render and events ----
  function render() {
    derive();
    const list = steps(), s = cur();
    $('#wz-steps').innerHTML = `<div class="wzbar">${list.map((x, i) => `<span class="${i < W.i ? 'done' : i === W.i ? 'on' : ''}" title="${x.t}"></span>`).join('')}</div>` +
      `<div class="wzcap">Шаг ${W.i + 1} из ${list.length} · <b>${s.t}</b>${list[W.i + 1] ? ` <span class="muted">→ ${list[W.i + 1].t}</span>` : ''}</div>`;
    $('#wz-body').innerHTML = VIEW[s.id]();
    $('#wz-back').hidden = W.i === 0;
    $('#wz-next').textContent = s.id === 'welcome' ? 'Начать' : s.id === 'done' ? 'Открыть Solarbox'
      : s.id === 'review' && W.review?.items.length && !W.applied ? 'Применить' : 'Далее';
    $('#wz-skip').hidden = s.id !== 'welcome' || DEMO;
    bind();
    if (DEMO && s.id === 'build') W.plan.forEach((_, i) => simulate(i));
    if (DEMO && s.id === 'battery' && W.use.jkble && W.plan.some((p, i) => p.nb && W.boards[i]?.adopted) && W.scan === null && !scanBle.started) { scanBle.started = true; scanBle(); }
    save();
  }

  function bind() {
    const body = $('#wz-body');
    body.querySelectorAll('[data-k]').forEach((el) => {
      const k = el.dataset.k;
      const upd = () => {
        const v = el.type === 'checkbox' ? el.checked : el.type === 'number' ? parseFloat(el.value) : el.value;
        set(k, el.type === 'number' && !isFinite(v) ? 0 : v);
        if (k === 'want.nbms' || k === 'want.volt') W.want[k.slice(5)] = +el.value;
        if (k === 'want.volt') W.batt.cells = { 12: 4, 24: 8, 48: 16 }[+el.value];
        if (k.startsWith('batt.')) W.inv = null; // re-propose the inverter from the new battery
      };
      el.oninput = upd;
      // Re-render on commit where derived text depends on the value (not while typing — keeps focus).
      el.onchange = () => { upd(); if (el.type === 'checkbox' || el.tagName === 'SELECT' || /^(batt|inv)\./.test(k)) render(); };
    });
    body.querySelectorAll('[data-pwr]').forEach((el) => (el.onchange = () => { W.power.src = el.dataset.pwr; render(); }));
    body.querySelectorAll('[data-vpin]').forEach((el) => (el.onclick = () => { W.power.vpin = +el.dataset.vpin; render(); }));
    body.querySelectorAll('[data-have]').forEach((el) => (el.onclick = () => {
      const k = el.dataset.have; W.have[k] = Math.max(0, Math.min(9, W.have[k] + +el.dataset.d)); render();
    }));
    body.querySelectorAll('[data-sim]').forEach((el) => (el.onclick = () => { const i = el.dataset.sim; clearTimeout(sims[i]); sims[i] = 0; simulate(+i, true); }));
    const wp = $('#wz-wifipw'); if (wp) wp.oninput = () => { wifiPw = wp.value; };
    const ws = $('#wz-wifissid');
    if (ws) ws.onchange = () => { W.wifi.other = ws.value === '\u0000other'; if (!W.wifi.other) W.wifi.ssid = ws.value; render(); };
    const wo = $('#wz-wifiother'); if (wo) wo.oninput = () => { W.wifi.ssid = wo.value; };
    body.querySelectorAll('[data-rescan]').forEach((el) => (el.onclick = () => { delete nets[el.dataset.rescan]; render(); }));
    body.querySelectorAll('[data-usbskip]').forEach((el) => (el.onclick = () => {
      const port = el.dataset.usbskip;
      if (W.usbSkip[port]) delete W.usbSkip[port]; else W.usbSkip[port] = true;
      render();
    }));
    body.querySelectorAll('[data-usbretry]').forEach((el) => (el.onclick = () => {
      const p = usb.ports.find((x) => x.port === el.dataset.usbretry);
      if (p) { p.tried = 0; p.error = null; }
      pollUsb();
    }));
    body.querySelectorAll('[data-pcflash]').forEach((el) => (el.onclick = (e) => { e.preventDefault(); pcFlash[+el.dataset.pcflash] = true; render(); }));
    body.querySelectorAll('[data-routerflash]').forEach((el) => (el.onclick = (e) => { e.preventDefault(); delete pcFlash[+el.dataset.routerflash]; render(); }));
    body.querySelectorAll('[data-usbflash]').forEach((el) => (el.onclick = () => usbFlash(+el.dataset.usbflash)));
    body.querySelectorAll('[data-usbflashlink]').forEach((el) => (el.onclick = (e) => { e.preventDefault(); usbFlash(+el.dataset.usbflashlink); }));
    body.querySelectorAll('[data-usbwifi]').forEach((el) => (el.onclick = () => usbWifi(+el.dataset.usbwifi, el)));
    body.querySelectorAll('[data-later]').forEach((el) => (el.onclick = () => { const i = +el.dataset.later; W.boards[i] = Object.assign(W.boards[i] || {}, { later: !W.boards[i]?.later }); render(); }));
    body.querySelectorAll('[data-adopt]').forEach((el) => (el.onclick = () => { W.boards[el.dataset.adopt].adopted = true; toast('Плата будет подключена при «Применить»'); render(); }));
    body.querySelectorAll('[data-fw]').forEach((el) => (el.onclick = () => (DEMO ? toast('Тестовый режим: прошивка не скачивается') : firmware(W.plan[+el.dataset.fw], el))));
    body.querySelectorAll('[data-pack]').forEach((el) => (el.onchange = () => {
      const m = el.dataset.pack;
      W.packs = el.checked ? [...W.packs, m] : W.packs.filter((x) => x !== m);
      if (W.packs.length > W.want.nbms) { W.packs.shift(); toast(`Выбрано BMS на шаге «Оборудование»: ${W.want.nbms}`); }
      // Test mode: as if the BMS reported cells and capacity.
      if (DEMO && W.packs.length && !W.batt.read) Object.assign(W.batt, { cells: 8, cap: 280, read: true }), (W.inv = null);
      render();
    }));
    const rs = $('#wz-rescan'); if (rs) rs.onclick = () => (DEMO ? W.plan.some((p, i) => p.nb && W.boards[i]?.adopted) && scanBle() : (W.scan = null, render(), poll()));
  }

  $('#wz-next').onclick = async () => {
    const s = cur(), err = invalid(s.id);
    if (err) return toast(err, true);
    if (s.id === 'pick') fixPlan();
    if (s.id === 'done') { location.href = '/'; return; }
    if (s.id === 'review' && !W.review) return toast('Подождите — считаю, что изменится', true);
    if (s.id === 'review' && W.review.items.length && !W.applied) {
      const failed = Object.values(W.results || {}).filter((x) => !x.ok).map((x) => x.id);
      $('#wz-next').disabled = true;
      try { await applyAll(failed.length ? failed : null); } catch (e) { toast(e.message, true); } finally { $('#wz-next').disabled = false; }
      if (!W.applied) return;
    }
    W.i++;
    if (cur().id === 'review') { W.results = null; W.applied = false; loadReview(); }
    if (cur().id === 'done' && !DEMO) api('/api/wizard/emergency').then((r) => { W.emergency = r.password; render(); }, () => {});
    render(); scrollTo(0, 0);
  };
  $('#wz-back').onclick = () => { if (W.i > 0) { W.i--; if (cur().id === 'review') loadReview(); render(); scrollTo(0, 0); } };
  $('#wz-skip').onclick = async () => {
    if (!(await SB.ask('Пропустить мастер? Его можно открыть позже в «Настройках».', { ok: 'Пропустить', danger: false }))) return;
    try { await api('/api/wizard/skip', { method: 'POST' }); } catch (e) { /* the dashboard opens anyway */ }
    location.href = '/';
  };

  (async () => {
    $('#wz-demo').hidden = !DEMO;
    if (!DEMO) {
      try {
        const w = await api('/api/wizard');
        if (w.draft?.ui) for (const k of KEEP) if (w.draft.ui[k] !== undefined) W[k] = w.draft.ui[k];
      } catch (e) { toast('Черновик не загрузился: ' + e.message, true); }
      W.i = Math.min(W.i, steps().length - 1);
      if (cur().id === 'review') loadReview();
    }
    render();
    poll();
  })();
})();
