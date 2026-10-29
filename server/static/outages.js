// Power outages: data shared with «Потоки» (grid timer, chart bands) and the
// «Отключения» page (status, totals, full list, remove tests).

(() => {
  const { $, api, page } = SB;
  let out = null, at = 0;
  const midnightTs = () => { const d = new Date(); d.setHours(0, 0, 0, 0); return Math.floor(d / 1000); };
  const span = (s) => s < 60 ? Math.round(s) + ' с' : s < 3600 ? Math.round(s / 60) + ' мин'
    : s < 86400 ? `${Math.floor(s / 3600)} ч ${Math.round((s % 3600) / 60)} мин` : `${Math.floor(s / 86400)} д ${Math.round((s % 86400) / 3600)} ч`;
  const when = (t) => {
    const d = new Date(t * 1000), p = (x) => String(x).padStart(2, '0');
    return (t >= midnightTs() ? '' : `${p(d.getDate())}.${p(d.getMonth() + 1)} `) + `${p(d.getHours())}:${p(d.getMinutes())}`;
  };

  async function refresh(force) {
    if (!force && Date.now() - at < 30000) return;
    at = Date.now();
    try { out = await api(`/api/outages?midnight=${midnightTs()}`); } catch (e) { return; }
    if (!$('#page-outages').hidden) render();
  }

  function render() {
    const el = $('#outages'); if (!el || !out) return;
    const cur = out.current, now = Date.now() / 1000;
    const tile = (k, w) => `<div><span>${k}</span><b>${w.count} раз</b><i>${w.count ? span(w.secs) + (w.longest ? ' · самое долгое ' + span(w.longest) : '') : 'без отключений'}</i></div>`;
    const soc = (a, b) => (a != null ? Math.round(a) + '%' : '—') + ' → ' + (b != null ? Math.round(b) + '%' : '…');
    el.innerHTML = `
      <div class="panel"><div class="ostat">${cur ? `<span class="dot bad"></span><b>Света нет ${span(now - cur.start)}</b><span class="muted">с ${when(cur.start)}</span>`
        : `<span class="dot ok"></span><b>Свет есть</b>${out.items[0] ? `<span class="muted">последнее отключение ${when(out.items[0].start)}</span>` : ''}`}</div>
        <div class="otiles">${tile('Сегодня', out.today)}${tile('7 дней', out.d7)}${tile('30 дней', out.d30)}</div></div>
      <div class="panel"><h2>Отключения за 30 дней</h2>
        ${out.items.length ? `<table class="olist"><tr class="muted"><td>когда</td><td>длилось</td><td>АКБ</td><td class="adm-only"></td></tr>` +
          out.items.map((o) => `<tr><td>${when(o.start)} – ${o.end ? when(o.end) : 'сейчас'}</td><td>${span(o.secs)}</td><td>${soc(o.soc_start, o.soc_end)}</td>` +
          `<td class="adm-only"><button class="btn xs" data-odel="${o.start}" title="Убрать из учёта (например, тест)">✕</button></td></tr>`).join('') + '</table>'
          : '<p class="muted small">Отключений не было.</p>'}
        <p class="muted small">Отключение — сеть ниже ${out.thresholds.low} В дольше ${out.thresholds.hold_s} с; закончилось — выше ${out.thresholds.high} В. Считается на сервере по каждому пакету от инвертора, даже когда дашборд закрыт. ✕ (после входа) убирает запись из учёта — например, тестовое отключение.</p></div>`;
    el.querySelectorAll('[data-odel]').forEach((b) => (b.onclick = async () => {
      if (!SB.requireAdmin() || !(await SB.ask(`Убрать отключение ${when(+b.dataset.odel)} из учёта?`))) return;
      try { await api('/api/outages/' + b.dataset.odel, { method: 'DELETE' }); SB.toast('Убрано'); refresh(true); }
      catch (e) { SB.toast(e.message, true); }
    }));
  }

  SB.outages = { data: () => out, refresh, span };
  page('outages', { show() { refresh(true); }, tick() { refresh(); render(); } });
})();
