// «Доступ»: opening Solarbox to the internet through the router (port forward
// + IP Cloud name), the certificate fingerprint to check on a new device, and
// who is logged in. Router changes are confirmed with the router password,
// which goes to the router once and is not kept.

(() => {
  const { $, esc, api, toast, page, ask, askText } = SB;
  let d = null;

  const when = (t) => {
    const x = new Date(t * 1000), p = (n) => String(n).padStart(2, '0');
    return `${p(x.getDate())}.${p(x.getMonth() + 1)} ${p(x.getHours())}:${p(x.getMinutes())}`;
  };
  const who = (u) => u ? esc(u) : '<span class="muted">аварийный пароль</span>';

  async function load() {
    try { d = await api('/api/access'); } catch (e) { $('#access').innerHTML = `<div class="panel bad">${esc(e.message)}</div>`; return; }
    render();
  }

  function render() {
    const s = d.status, port = s?.open_port;
    const urls = s && port ? [s.public_ip && `https://${s.public_ip}:${port}`, s.ddns_name && `https://${s.ddns_name}:${port}`].filter(Boolean) : [];
    let state;
    if (!s) state = `<p class="bad">${esc(d.status_error || 'роутер недоступен')}</p>`;
    else if (port) state = `<p><span class="dot ok"></span><b>Открыт</b> — порт ${port}</p>` +
      urls.map((u) => `<div class="frow"><code>${esc(u)}</code><span></span><button class="btn xs" data-copy="${esc(u)}">Копировать</button></div>`).join('') +
      (s.ddns_name ? '<p class="muted small">Если IP у провайдера меняется, заходите по имени — оно не меняется.</p>' : '');
    else if (s.grey) state = `<p><span class="dot bad"></span><b>Невозможен</b></p><p class="small">У роутера нет белого адреса (WAN: ${esc(s.wan.join(', '))}): провайдер или роутер выше по сети делает NAT, и проброс отсюда не сработает. Попросите у провайдера белый IP или заходите через MikroTik Back To Home.</p>`;
    else state = `<p><span class="dot"></span><b>Закрыт</b>${s.public_ip ? ` — внешний адрес ${esc(s.public_ip)}` : ''}</p>`;

    $('#access').innerHTML = `
      <div class="panel"><h2>Доступ из интернета</h2>${state}
        <div class="row">
          ${s && !port && !s.grey ? '<button class="btn on" id="ac-open">Открыть доступ</button>' : ''}
          ${s && port ? '<button class="btn danger" id="ac-close">Закрыть доступ</button>' : ''}
          ${!d.service ? '<button class="btn" id="ac-svc">Подключить к роутеру</button>' : ''}
        </div>
        <p class="muted small">Изменения на роутере подтверждаются паролем роутера — он уходит роутеру один раз и нигде не сохраняется.
          Открытие включает IP Cloud (имя вида *.sn.mynetname.net) и добавляет одно правило NAT с комментарием <code>solarbox-access</code>; закрытие удаляет только его.</p>
        ${d.cmd_open && !s?.grey ? `<details><summary class="small">Команды RouterOS, если кнопки не работают</summary>
          <p class="small">Открыть:</p><pre>${esc(d.cmd_open)}</pre><p class="small">Закрыть:</p><pre>${esc(d.cmd_close)}</pre></details>` : ''}
        ${s ? `<div class="facts wide" style="margin-top:10px;word-break:break-all">
          <div><span>WAN</span><b class="small">${esc(s.wan.join(', ') || '—')}</b></div>
          <div><span>IP Cloud</span><b class="small">${s.ddns_enabled ? esc(s.ddns_name || 'включён') : 'выключен'}</b></div>
          <div><span>Роутер</span><b class="small">${esc(d.router || '—')}</b></div></div>` : ''}
      </div>
      <div class="panel"><h2>Сертификат</h2>
        <p class="small">Сертификат solarbox самоподписанный, поэтому браузер на новом устройстве предупредит «соединение не защищено». Перед тем как согласиться, откройте в браузере сведения о сертификате и сверьте отпечаток SHA-256 с этим:</p>
        <pre>${esc(d.fingerprint.slice(0, 47))}\n${esc(d.fingerprint.slice(48))}</pre>
        <p class="muted small">Совпадает — принимайте, это solarbox. Не совпадает — кто-то подменяет соединение, пароль не вводите. Отпечаток не меняется никогда, он же приходит в Telegram при открытии доступа.</p>
        <details><summary class="small">Для скриптов</summary><pre>SB_PIN=${esc(d.pin)}</pre></details>
      </div>
      <div class="panel"><h2>Кто вошёл</h2>
        ${d.sessions.sort((a, b) => b.seen - a.seen).map((x) => `<div class="frow"><span>${who(x.user)} · ${esc(x.browser)}<br><span class="muted small">${esc(x.ip)} · вход ${when(x.created)}</span></span><span class="muted small">${when(x.seen)}</span><span></span></div>`).join('')}
        <div class="row" style="margin-top:8px"><button class="btn" id="ac-all">Выйти везде</button></div>
        <p class="muted small">Сессия живёт 30 дней с последнего захода. Раз в сутки solarbox проверяет, что пользователь ещё есть на роутере и не отключён, иначе закрывает его сессии.</p>
      </div>`;

    $('#access').querySelectorAll('[data-copy]').forEach((b) => (b.onclick = async () => {
      try { await navigator.clipboard.writeText(b.dataset.copy); toast('Скопировано'); } catch (e) { toast(b.dataset.copy); }
    }));
    if ($('#ac-open')) $('#ac-open').onclick = () => change('open', 'Открыть доступ к solarbox из интернета?');
    if ($('#ac-close')) $('#ac-close').onclick = () => change('close', 'Закрыть доступ из интернета?');
    if ($('#ac-svc')) $('#ac-svc').onclick = () => change('service', null);
    $('#ac-all').onclick = async () => {
      if (!(await ask('Закрыть все сессии на всех устройствах, включая эту?', { ok: 'Выйти везде', danger: true }))) return;
      try { await api('/api/logout-all', { method: 'POST' }); } catch (e) { /* login page anyway */ }
      location.href = '/login.html';
    };
  }

  // Router credentials for one change: the logged-in router user, or ask for
  // the login too after an emergency-password login.
  async function change(what, question) {
    if (question && !(await ask(question))) return;
    let user = d.user;
    if (!user) { user = await askText('Логин роутера', { ok: 'Дальше', plain: true }); if (!user) return; }
    const password = await askText(`Пароль роутера (${user})`, { ok: 'Подтвердить' });
    if (!password) return;
    try {
      const r = await api('/api/access/' + what, { json: { user, password } });
      toast(what === 'open' ? `Доступ открыт, порт ${r.port}` : what === 'close' ? 'Доступ закрыт' : 'solarbox подключён к роутеру');
    } catch (e) { toast(e.message, true); }
    load();
  }

  page('access', { show: load });
})();
