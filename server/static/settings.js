// Settings: what the container costs the router (RAM, CPU, flash), database
// state and storage policy, live UI overrides (with reset) and the config.

(() => {
  const { $, esc, num, dur, api, toast, requireAdmin, page } = SB;
  let timer = null;

  const MB = (b) => (b === undefined || b === null ? '—' : b >= 1048576 ? (b / 1048576).toFixed(1) + ' МБ' : (b / 1024).toFixed(0) + ' КБ');
  const KBm = (k) => (k === undefined || k === null ? '—' : k >= 10240 ? (k / 1024).toFixed(0) + ' МБ' : (k / 1024).toFixed(1) + ' МБ');
  const pct = (x) => (x === undefined || x === null ? '—' : x.toFixed(x < 10 ? 1 : 0) + ' %');
  const bar = (part, total) => {
    const p = total ? Math.min(100, (part / total) * 100) : 0;
    return `<span class="sbar"><i style="width:${p.toFixed(1)}%"></i></span>`;
  };
  const facts = (rows) => rows.map(([k, x, extra]) => `<div><span>${k}</span><b>${x}</b>${extra || ''}</div>`).join('');
  const span = (s) => (s < 3600 ? Math.round(s / 60) + ' мин' : s < 86400 ? Math.round(s / 3600) + ' ч' : Math.round(s / 86400) + ' д');

  async function system() {
    let s;
    try { s = await api('/api/system'); } catch (e) { $('#set-res').innerHTML = '<div class="empty">Сервер недоступен</div>'; return; }
    const p = s.process, h = s.host, st = s.storage;
    const used = h.mem_total_kb && h.mem_avail_kb ? h.mem_total_kb - h.mem_avail_kb : undefined;
    $('#set-res').innerHTML = facts([
      ['ОЗУ контейнера', KBm(p.rss_kb), `<small>пик ${KBm(p.rss_peak_kb)}${h.mem_total_kb ? ' · ' + ((p.rss_kb / h.mem_total_kb) * 100).toFixed(2) + '% от роутера' : ''}</small>`],
      ['ЦП контейнера', pct(p.cpu_pct), '<small>от одного ядра, за 5 с</small>'],
      ['Флеш: база', MB(st.db_bytes + st.wal_bytes), `<small>файл ${MB(st.db_bytes)} + WAL ${MB(st.wal_bytes)}</small>`],
      ['Флеш: программа', MB(p.binary_bytes), `<small>правки интерфейса ${MB(st.ui_override_bytes)} · конфиг ${MB(st.config_bytes)}</small>`],
      ['ОЗУ роутера', used !== undefined ? KBm(used) + ' / ' + KBm(h.mem_total_kb) : '—', bar(used, h.mem_total_kb)],
      ['ЦП роутера', pct(h.cpu_pct), `<small>${h.cores || '?'} ядра · load ${h.load1 ?? '—'}</small>`],
      ['Флеш роутера', st.fs_total ? MB(st.fs_total - st.fs_free) + ' / ' + MB(st.fs_total) : '—', bar(st.fs_total - st.fs_free, st.fs_total) + `<small>свободно ${MB(st.fs_free)}</small>`],
      ['Сервер работает', dur(s.uptime_s), `<small>версия ${esc(s.version)}</small>`],
    ]);
    const d = s.db, pol = s.storage_policy;
    $('#set-db').innerHTML = facts([
      ['Записей в истории', d ? d.rows.toLocaleString('ru') : '—'],
      ['Устройств', d ? d.devices : '—'],
      ['История с', d && d.oldest_ts ? new Date(d.oldest_ts * 1000).toLocaleString('ru') : '—'],
      ['Удаление старого', pol.retention_days ? 'через ' + pol.retention_days + ' дн' : 'не удаляется, только прореживается'],
    ]) ;
    $('#set-tiers').innerHTML = pol.tiers.map((t) => `<span class="tier">${t.min_age_s ? 'старше ' + span(t.min_age_s) : 'сегодня'} → раз в ${span(t.res_s)}</span>`).join('');
  }

  async function ui() {
    let f, c;
    try { [f, c] = await Promise.all([api('/api/files'), api('/api/config')]); } catch (e) { return; }
    $('#set-files').innerHTML = `<p class="muted small">Файлы в <code>${esc(f.dir)}</code> подменяют встроенные. «Вернуть» удаляет правку.</p>` +
      f.files.map((x) => `<div class="frow"><code>${esc(x.name)}</code><span class="${x.override ? 'ok' : 'muted'}">${x.override ? 'изменён' : 'встроенный'}</span>` +
        (x.override ? `<button class="btn adm" data-reset="${esc(x.name)}">Вернуть</button>` : '<span></span>') + '</div>').join('');
    document.querySelectorAll('[data-reset]').forEach((b) => (b.onclick = async () => {
      if (!requireAdmin() || !(await SB.ask(`Вернуть встроенную версию ${b.dataset.reset}?`))) return;
      try { await api('/api/files/' + encodeURIComponent(b.dataset.reset), { method: 'DELETE' }); toast('Возвращено — обнови страницу'); ui(); }
      catch (e) { toast('Не получилось: ' + e.message, true); }
    }));
    $('#set-config').innerHTML = (c.load_error ? `<p class="bad">config.json с ошибкой, работает прежний: ${esc(c.load_error)}</p>` : '') +
      `<p class="muted small">Плат: ${c.boards.length} · правил: ${c.actions.length}</p>` +
      `<details><summary>config.json</summary><pre>${esc(JSON.stringify(Object.fromEntries(Object.entries(c).filter(([k]) => !['load_error', 'admin'].includes(k))), null, 2))}</pre></details>`;
  }

  // ---- tariffs: periods from a date, optional night zone; stored in config.json → money ----
  async function moneyPanel() {
    const el = $('#set-money');
    let cfg; try { cfg = await api('/api/config'); } catch (e) { el.textContent = e.message; return; }
    const m = cfg.money || { currency: 'грн', tariffs: [] };
    let rows = (m.tariffs || []).map((t) => ({ ...t })).sort((a, b) => a.from.localeCompare(b.from));
    const hours = (sel) => [...Array(24).keys()].map((h) => `<option value="${h}"${h === sel ? ' selected' : ''}>${String(h).padStart(2, '0')}:00</option>`).join('');
    const draw = () => {
      el.innerHTML = `<p class="muted small">Цена за кВт·ч из сети. Когда тариф меняется — добавь строку с даты изменения: старые дни считаются по старой цене. Ночная цена — для двухзонного тарифа (оставь пустой, если тариф один). Суммы — оценка: инвертор не меряет сеть, её потребление считается по балансу.</p>
        ${rows.length ? `<table class="ttab"><tr class="muted"><td>с даты</td><td>цена</td><td>ночью</td><td>ночь с</td><td>до</td><td></td></tr>` + rows.map((t, i) => `<tr>
          <td><input type="date" data-t="${i}" data-f="from" value="${esc(t.from)}"></td>
          <td><input type="number" step="0.01" min="0" data-t="${i}" data-f="price" value="${t.price ?? ''}"></td>
          <td><input type="number" step="0.01" min="0" data-t="${i}" data-f="night_price" value="${t.night_price ?? ''}" placeholder="—"></td>
          <td><select data-t="${i}" data-f="night_from">${hours(t.night_from ?? 23)}</select></td>
          <td><select data-t="${i}" data-f="night_to">${hours(t.night_to ?? 7)}</select></td>
          <td><button class="btn xs adm" data-tdel="${i}">✕</button></td></tr>`).join('') + '</table>'
          : '<p class="muted small">Тариф не задан — деньги не считаются.</p>'}
        <div class="row"><label class="small">Валюта <input id="m-cur" value="${esc(m.currency || 'грн')}" style="width:70px"></label>
          <button class="btn adm" id="m-add">+ Тариф с даты</button><button class="btn on adm" id="m-save">Сохранить</button></div>`;
      el.querySelectorAll('[data-t]').forEach((inp) => (inp.onchange = () => {
        const t = rows[+inp.dataset.t], f = inp.dataset.f;
        t[f] = f === 'from' ? inp.value : inp.value === '' ? null : +inp.value;
      }));
      el.querySelectorAll('[data-tdel]').forEach((b) => (b.onclick = () => { if (requireAdmin()) { rows.splice(+b.dataset.tdel, 1); draw(); } }));
      $('#m-add').onclick = () => {
        if (!requireAdmin()) return;
        const last = rows[rows.length - 1];
        const d = new Date(); const today = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
        rows.push({ from: today, price: last?.price ?? null, night_price: last?.night_price ?? null, night_from: 23, night_to: 7 });
        draw();
      };
      $('#m-save').onclick = async () => {
        if (!requireAdmin()) return;
        const bad = rows.find((t) => !/^\d{4}-\d{2}-\d{2}$/.test(t.from || '') || !(t.price > 0));
        if (bad) return toast('У каждой строки нужны дата и цена', true);
        if (new Set(rows.map((t) => t.from)).size !== rows.length) return toast('Две строки с одной датой', true);
        const tariffs = rows.map((t) => ({ from: t.from, price: +t.price, ...(t.night_price ? { night_price: +t.night_price } : {}), night_from: +(t.night_from ?? 23), night_to: +(t.night_to ?? 7) }));
        const fresh = await api('/api/config');
        const out = Object.fromEntries(Object.entries(fresh).filter(([k]) => !['load_error', 'admin'].includes(k)));
        out.money = { currency: ($('#m-cur').value || 'грн').trim(), tariffs };
        try { await api('/api/config', { method: 'PUT', json: out }); toast('Тариф сохранён'); moneyPanel(); }
        catch (e) { toast('Не сохранилось: ' + e.message, true); }
      };
    };
    draw();
  }

  async function tg() {
    const el = $('#set-tg');
    if (!SB.isAdmin) { el.innerHTML = '<p class="muted small">Настройка Telegram — после входа («Войти» в меню).</p>'; return; }
    let t; try { t = await api('/api/telegram'); } catch (e) { el.textContent = e.message; return; }
    const who = (c) => `<b>${esc(c.name || 'без имени')}</b>${c.username ? ` <span class="muted">@${esc(c.username)}</span>` : ''} <span class="muted small">· ${esc(c.id)}</span>`;
    el.innerHTML = `
      <p class="small">${t.token_set ? `✔ бот ${t.bot ? '<b>@' + esc(t.bot) + '</b>' : 'подключён'} · команды /status, /help, /id зарегистрированы` : '✘ бот не подключён'}</p>
      <div class="row"><input type="password" id="tg-token" placeholder="${t.token_set ? 'новый токен (оставь пустым — не менять)' : 'токен от @BotFather, 123456:ABC…'}" autocomplete="off">
        <button class="btn on" id="tg-save">${t.token_set ? 'Сменить токен' : 'Подключить'}</button></div>
      ${t.token_set ? `
      <label class="chk"><input type="checkbox" id="tg-notify"${t.notify_changes ? ' checked' : ''}> Сообщать об изменениях настроек
        <span class="muted small">— инвертор, BMS, прошивки плат, автоматизации, конфигурация, получатели</span></label>
      <label class="chk"><input type="checkbox" id="tg-access"${t.notify_access ? ' checked' : ''}> Сообщать о входах и доступе
        <span class="muted small">— каждый вход, серии неверных паролей, доступ из интернета открыт/закрыт, смена внешнего IP</span></label>
      <h3>Получатели</h3>
      ${t.chats.length ? t.chats.map((c) => `<div class="frow"><span>${who(c)}</span><span></span><button class="btn xs" data-tdel="${esc(c.id)}">✕</button></div>`).join('')
        : '<p class="muted small">Пока никого. Напиши своему боту <code>/start</code> — появишься ниже в «Хотят доступ».</p>'}
      ${t.pending.length ? `<h3>Хотят доступ</h3>` + t.pending.map((c) => `<div class="frow"><span>${who(c)}</span><span></span><button class="btn xs on" data-tadd="${esc(c.id)}">Добавить</button></div>`).join('') : ''}
      <p class="muted small">Чтобы добавить человека: пусть напишет боту${t.bot ? ' <b>@' + esc(t.bot) + '</b>' : ''} <code>/start</code>, обнови эту страницу и нажми «Добавить». Уведомления приходят всем получателям; бот отвечает только им.</p>
      <div class="row"><input id="tg-chat" placeholder="или chat id вручную"><button class="btn" id="tg-add">Добавить</button>
        <button class="btn" id="tg-test">Отправить тест всем</button><button class="btn" id="tg-off">Отключить бота</button></div>` : ''}`;
    $('#tg-save').onclick = async () => {
      const token = $('#tg-token').value.trim();
      if (!token) return toast('Вставь токен бота', true);
      try { const r = await api('/api/telegram', { method: 'PUT', json: { token } }); toast('Бот @' + r.bot + ' подключён, команды зарегистрированы'); tg(); }
      catch (e) { toast(e.message, true); }
    };
    if (!t.token_set) return;
    $('#tg-notify').onchange = async (e) => {
      try { await api('/api/telegram', { method: 'PUT', json: { notify_changes: e.target.checked } }); toast(e.target.checked ? 'Изменения будут приходить в Telegram' : 'Изменения больше не присылаются'); }
      catch (err) { e.target.checked = !e.target.checked; toast(err.message, true); }
    };
    $('#tg-access').onchange = async (e) => {
      try { await api('/api/telegram', { method: 'PUT', json: { notify_access: e.target.checked } }); toast(e.target.checked ? 'Входы будут приходить в Telegram' : 'Входы больше не присылаются'); }
      catch (err) { e.target.checked = !e.target.checked; toast(err.message, true); }
    };
    const add = async (id) => { try { await api('/api/telegram/chats', { method: 'POST', json: { id } }); toast('Добавлен'); tg(); } catch (e) { toast(e.message, true); } };
    el.querySelectorAll('[data-tadd]').forEach((b) => (b.onclick = () => add(b.dataset.tadd)));
    $('#tg-add').onclick = () => { const id = $('#tg-chat').value.trim(); if (id) add(id); };
    el.querySelectorAll('[data-tdel]').forEach((b) => (b.onclick = async () => {
      if (!(await SB.ask('Убрать этого получателя? Он перестанет получать уведомления и пользоваться ботом.'))) return;
      try { await api('/api/telegram/chats/' + encodeURIComponent(b.dataset.tdel), { method: 'DELETE' }); toast('Убран'); tg(); } catch (e) { toast(e.message, true); }
    }));
    $('#tg-test').onclick = async () => { try { await api('/api/telegram/test', { method: 'POST' }); toast('Отправлено — проверь Telegram'); } catch (e) { toast(e.message, true); } };
    $('#tg-off').onclick = async () => {
      if (!(await SB.ask('Отключить бота и забыть токен и получателей?'))) return;
      try { await api('/api/telegram', { method: 'PUT', json: { disable: true } }); toast('Отключено'); tg(); } catch (e) { toast(e.message, true); }
    };
  }

  page('settings', {
    show() { system(); ui(); tg(); moneyPanel(); timer = setInterval(system, 5000); },
    hide() { clearInterval(timer); },
  });
})();
