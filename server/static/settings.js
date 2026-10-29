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
      `<p class="muted small">Управление: ${c.admin ? 'включено (по паролю)' : 'выключено'} · плат: ${c.boards.length} · правил: ${c.actions.length}</p>` +
      `<details><summary>config.json</summary><pre>${esc(JSON.stringify(Object.fromEntries(Object.entries(c).filter(([k]) => !['load_error', 'admin'].includes(k))), null, 2))}</pre></details>`;
  }

  async function tg() {
    const el = $('#set-tg');
    if (!SB.isAdmin) { el.innerHTML = '<p class="muted small">Настройка Telegram — после входа («Войти» в меню).</p>'; return; }
    let t; try { t = await api('/api/telegram'); } catch (e) { el.textContent = e.message; return; }
    el.innerHTML = `<p class="small">${t.token_set ? '✔ токен бота задан' : '✘ токен не задан'} · ${t.chat_id ? 'чат ' + esc(t.chat_id) : 'чат не указан'}</p>
      <p class="muted small">1) Создай бота у @BotFather и вставь токен. 2) Напиши своему боту <code>/id</code> — он ответит номером чата, вставь его. 3) «Отправить тест». Потом в «Автоматизациях» появится действие «Сообщение в Telegram», а боту можно писать <code>/status</code>.</p>
      <div class="row"><input type="password" id="tg-token" placeholder="${t.token_set ? 'токен (оставь пустым — не менять)' : '123456:ABC… токен бота'}" autocomplete="off">
        <input id="tg-chat" placeholder="chat id" value="${esc(t.chat_id || '')}"></div>
      <div class="row"><button class="btn on" id="tg-save">Сохранить</button><button class="btn" id="tg-test">Отправить тест</button>${t.token_set ? '<button class="btn" id="tg-off">Отключить</button>' : ''}</div>`;
    $('#tg-save').onclick = async () => {
      try { await api('/api/telegram', { method: 'PUT', json: { token: $('#tg-token').value, chat_id: $('#tg-chat').value } }); toast('Сохранено'); tg(); }
      catch (e) { toast(e.message, true); }
    };
    $('#tg-test').onclick = async () => { try { await api('/api/telegram/test', { method: 'POST' }); toast('Отправлено — проверь Telegram'); } catch (e) { toast(e.message, true); } };
    if ($('#tg-off')) $('#tg-off').onclick = async () => {
      if (!(await SB.ask('Отключить Telegram и забыть токен?'))) return;
      try { await api('/api/telegram', { method: 'PUT', json: { disable: true } }); toast('Отключено'); tg(); } catch (e) { toast(e.message, true); }
    };
  }

  page('settings', {
    show() { system(); ui(); tg(); timer = setInterval(system, 5000); },
    hide() { clearInterval(timer); },
  });
})();
