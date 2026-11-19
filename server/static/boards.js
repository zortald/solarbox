// Boards: status of each board, restart, and firmware upload (native ESPHome
// OTA through the server). Control needs the admin login.

(() => {
  const { $, esc, BOARDS, v, age, ago, dur, num, api, entity, act, toast, requireAdmin, page } = SB;
  let list = [], extra = {}, built = false, flashing = false;

  function card(b) {
    const ui = BOARDS[b.name] || { title: b.name, chip: '' };
    return `<div class="board" id="bd-${b.name}">
      <div class="bh"><b>${esc(ui.title)}</b><span class="muted">${esc(ui.chip)} · ${esc(b.host)}</span><span class="bst" data-st></span></div>
      <div class="facts" data-facts></div>
      <div class="bact">
        <button class="btn" data-ping>Проверить связь</button>
        <button class="btn adm" data-restart ${ui.restart ? '' : 'disabled'}>Перезагрузить</button>
        <a class="btn" href="http://${esc(b.host)}${b.web_port !== 80 ? ':' + b.web_port : ''}/" target="_blank" rel="noopener">Веб платы ↗</a>
      </div>
      <div class="ota adm-only">
        <button class="btn" data-lib>Обновить прошивку</button>
        <span class="muted small">свежая сборка Solarbox для этой платы, подпись проверяется</span>
        <label>Или своим файлом (.bin из <code>esphome compile</code>, <code>firmware.bin</code> / <code>*.ota.bin</code>):</label>
        <input type="file" accept=".bin" data-file>
        <button class="btn" data-flash>Прошить</button>
        <div class="muted" data-otamsg></div>
      </div>
    </div>`;
  }

  function render() {
    for (const b of list) {
      const el = document.getElementById('bd-' + b.name); if (!el) continue;
      const a = age(b.name), up = v(b.name, 'uptime'), rssi = v(b.name, 'rssi'), x = extra[b.name] || {};
      const ok = a < 90;
      el.querySelector('[data-st]').innerHTML = `<span class="dot ${ok ? 'ok' : 'bad'}"></span>${ok ? 'данные идут' : 'молчит'}`;
      const rows = [
        ['Последний пакет', ago(a)],
        ['Работает', up !== undefined ? dur(up) : '—'],
        ['Wi-Fi', rssi !== undefined ? num(rssi, 0) + ' дБм' : '—'],
        ['Прошивка', x.version ?? '—'],
        ['Связь с сервером', x.ping ?? '—'],
      ];
      el.querySelector('[data-facts]').innerHTML = rows.map(([k, t]) => `<div><span>${k}</span><b>${esc(t)}</b></div>`).join('');
    }
  }

  async function probe(name) {
    const x = (extra[name] = extra[name] || {});
    try {
      const p = await api(`/api/board/${name}/ping`);
      x.ping = p.ok ? `есть · ${p.ms} мс` : 'нет: ' + (p.error || 'HTTP ' + p.status);
    } catch (e) { x.ping = 'нет: ' + e.message; }
    const ui = BOARDS[name];
    if (ui?.version) { try { x.version = (await entity(name, ui.version)).value; } catch (e) { x.version = '—'; } }
    render();
  }

  async function restart(name) {
    if (!requireAdmin()) return;
    const ui = BOARDS[name];
    if (!(await SB.ask(`Перезагрузить плату «${ui.title}»? Данные с неё пропадут примерно на полминуты.`))) return;
    try { await act(name, ui.restart, 'press'); toast('Плата перезагружается'); }
    catch (e) { toast('Не получилось: ' + e.message, true); }
  }

  async function flash(name, el) {
    if (!requireAdmin()) return;
    if (flashing) return toast('Уже идёт прошивка другой платы', true);
    const f = el.querySelector('[data-file]').files[0];
    if (!f) return toast('Выбери файл прошивки', true);
    const ui = BOARDS[name];
    if (!(await SB.ask(`Прошить «${ui.title}» файлом ${f.name} (${Math.round(f.size / 1024)} КБ)?\n\nЕсли файл от другой платы, она может не загрузиться — проверь имя.`))) return;
    const msg = el.querySelector('[data-otamsg]');
    flashing = true;
    el.querySelectorAll('button').forEach((b) => (b.disabled = true));
    msg.textContent = 'Загружаю прошивку на плату… это занимает до минуты, не закрывай страницу.';
    const before = SB.T[name] || 0;
    try {
      const r = await api(`/api/board/${name}/ota`, { method: 'POST', body: await f.arrayBuffer(), headers: { 'Content-Type': 'application/octet-stream' } });
      msg.textContent = '✔ ' + r.message + '. Жду, когда плата вернётся…';
      toast('Прошивка залита');
      // Success = the board pushes again after the reboot.
      for (let i = 0; i < 40; i++) {
        await new Promise((res) => setTimeout(res, 3000));
        if ((SB.T[name] || 0) > before + 5) { msg.textContent = '✔ ' + r.message + '. Плата вернулась и шлёт данные.'; probe(name); break; }
        if (i === 39) msg.textContent = '✔ Прошивка залита, но плата пока не прислала данных — проверь через минуту.';
      }
    } catch (e) {
      msg.textContent = '✘ ' + e.message;
      toast('Прошивка не удалась: ' + e.message, true);
    } finally {
      flashing = false;
      el.querySelectorAll('button').forEach((b) => (b.disabled = false));
    }
  }

  // Firmware from the release library: Solarbox fetches the board's image
  // (built on demand the first time), checks the signature and flashes it.
  async function fromLibrary(name, el) {
    if (!requireAdmin()) return;
    if (flashing) return toast('Уже идёт прошивка другой платы', true);
    const ui = BOARDS[name];
    if (!(await SB.ask(`Обновить прошивку «${ui.title}» до свежей сборки Solarbox?\n\nПлата перезагрузится, настройки (Wi-Fi, BMS) сохранятся.`, { ok: 'Обновить' }))) return;
    const msg = el.querySelector('[data-otamsg]');
    flashing = true;
    el.querySelectorAll('button').forEach((b) => (b.disabled = true));
    const before = SB.T[name] || 0;
    try {
      let r;
      for (let k = 0; k < 80; k++) {
        msg.textContent = k ? 'Прошивка собирается (≈3–5 мин, один раз для всех таких плат)…' : 'Получаю прошивку и загружаю на плату… до минуты, не закрывайте страницу.';
        r = await api(`/api/board/${name}/update`, { json: {} });
        if (r.status !== 'building') break;
        await new Promise((res) => setTimeout(res, 15000));
      }
      if (!r.ok) throw new Error('сборка идёт слишком долго');
      msg.textContent = `✔ Прошивка ${r.target} ${r.version} залита. Жду, когда плата вернётся…`;
      for (let i = 0; i < 40; i++) {
        await new Promise((res) => setTimeout(res, 3000));
        if ((SB.T[name] || 0) > before + 5) { msg.textContent = `✔ ${r.target} ${r.version}: плата вернулась и шлёт данные.`; probe(name); break; }
        if (i === 39) msg.textContent = '✔ Прошивка залита, но плата пока не прислала данных — проверьте через минуту.';
      }
    } catch (e) {
      msg.textContent = '✘ ' + e.message;
      toast('Обновление не удалось: ' + e.message, true);
    } finally {
      flashing = false;
      el.querySelectorAll('button').forEach((b) => (b.disabled = false));
    }
  }

  async function load() {
    try { list = (await api('/api/boards')).boards; } catch (e) { list = []; }
    $('#boards').innerHTML = list.length ? list.map(card).join('')
      : '<div class="empty">Плат пока нет — подключите их в <a href="/wizard.html">мастере настройки</a>.</div>';
    for (const b of list) {
      const el = document.getElementById('bd-' + b.name);
      el.querySelector('[data-ping]').onclick = () => probe(b.name);
      el.querySelector('[data-restart]').onclick = () => restart(b.name);
      el.querySelector('[data-flash]').onclick = () => flash(b.name, el);
      el.querySelector('[data-lib]').onclick = () => fromLibrary(b.name, el);
      probe(b.name);
    }
    built = true;
    render();
  }

  page('boards', { show() { if (!built) load(); else list.forEach((b) => probe(b.name)); }, tick: render });
})();
