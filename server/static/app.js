// Dashboard controller: polls /api/latest for the "now" cards, lets you pick
// metrics + a time range, and draws one history chart per selected metric.

// Known metric keys → label / unit / colour / optional flag rendering.
// Unknown keys still work (raw key, no unit). Keep in sync with firmware POST.
const META = {
  // inverter
  ac_out_w:  { label: "Выход AC", unit: "Вт", color: "#4ea3ff" },
  pv_w:      { label: "Солнце", unit: "Вт", color: "#e3b341" },
  grid_v:    { label: "Сеть", unit: "В", color: "#8b98a8" },
  out_v:     { label: "Выход", unit: "В", color: "#58a6ff" },
  load_pct:  { label: "Нагрузка", unit: "%", color: "#db61a2" },
  batt_v:    { label: "АКБ (инв.)", unit: "В", color: "#3fb950" },
  temp:      { label: "Темп. инв.", unit: "°C", color: "#f0883e" },
  grid_hz:   { label: "Частота", unit: "Гц", color: "#8b98a8" },
  outages:   { label: "Отключений", unit: "", color: "#f85149" },
  // bms
  soc:       { label: "Заряд", unit: "%", color: "#3fb950" },
  bms_v:     { label: "Напряжение", unit: "В", color: "#58a6ff" },
  bms_a:     { label: "Ток", unit: "А", color: "#e3b341" },
  bms_w:     { label: "Мощность", unit: "Вт", color: "#4ea3ff" },
  cap_ah:    { label: "Остаток", unit: "Ач", color: "#3fb950" },
  t1:        { label: "Темп. 1", unit: "°C", color: "#f0883e" },
  t2:        { label: "Темп. 2", unit: "°C", color: "#f0883e" },
  t_mos:     { label: "Темп. MOS", unit: "°C", color: "#f85149" },
  cell_min:  { label: "Ячейка min", unit: "В", color: "#8b98a8" },
  cell_max:  { label: "Ячейка max", unit: "В", color: "#58a6ff" },
  cell_delta:{ label: "Разброс", unit: "В", color: "#db61a2" },
  chg:       { label: "Заряд", flag: true },
  dsg:       { label: "Разряд", flag: true },
  bal:       { label: "Баланс", flag: true },
};

const DEV_LABEL = { inverter: "Инвертор", bms: "АКБ / BMS" };

const RANGES = [
  { label: "1ч", sec: 3600 },
  { label: "6ч", sec: 6 * 3600 },
  { label: "24ч", sec: 24 * 3600 },
  { label: "7д", sec: 7 * 86400 },
  { label: "30д", sec: 30 * 86400 },
];

const state = {
  range: 6 * 3600,
  selected: new Set(),   // "device/metric"
  catalog: [],           // [{device, metric, ts}]
  initialised: false,
};

const $ = (s) => document.querySelector(s);
const meta = (m) => META[m] || { label: m, unit: "", color: "#4ea3ff" };
const key = (d, m) => `${d}/${m}`;

async function jget(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(r.status);
  return r.json();
}

function fmt(v, unit) {
  const a = Math.abs(v);
  const s = a >= 100 ? v.toFixed(0) : a >= 10 ? v.toFixed(1) : v.toFixed(2);
  return unit ? `${s}<span class="u">${unit}</span>` : s;
}

// ---- now cards ------------------------------------------------------------

async function refreshNow() {
  let data;
  try {
    data = await jget("/api/latest");
  } catch (e) {
    setStatus("down", "сервер недоступен");
    return;
  }
  const now = Date.now() / 1000;
  let newest = 0;
  const cards = [];
  for (const dev of Object.keys(data).sort()) {
    for (const m of Object.keys(data[dev])) {
      const { v, ts } = data[dev][m];
      if (ts > newest) newest = ts;
      const mt = meta(m);
      const age = now - ts;
      const old = age > 120 ? " old" : "";
      let valHtml;
      if (mt.flag) {
        const on = v >= 0.5;
        valHtml = `<span class="${on ? "flag-on" : "flag-off"}">${on ? "да" : "нет"}</span>`;
      } else {
        valHtml = fmt(v, mt.unit);
      }
      cards.push(`<div class="card${old}">
        <div class="dev">${DEV_LABEL[dev] || dev}</div>
        <div class="lbl">${mt.label}</div>
        <div class="val">${valHtml}</div>
      </div>`);
    }
  }
  $("#now").innerHTML = cards.join("") || `<div class="empty">Пока нет данных. Платы ещё ничего не прислали на /api/ingest.</div>`;

  const age = now - newest;
  if (!newest) setStatus("down", "нет данных");
  else if (age < 120) setStatus("live", `обновлено ${Math.round(age)} с назад`);
  else setStatus("stale", `молчит ${Math.round(age / 60)} мин`);
}

function setStatus(cls, text) {
  const el = $("#status");
  el.className = "status " + cls;
  el.textContent = text;
}

// ---- controls -------------------------------------------------------------

function renderRanges() {
  $("#ranges").innerHTML = RANGES.map(
    (r) => `<button data-sec="${r.sec}"${r.sec === state.range ? ' class="active"' : ""}>${r.label}</button>`
  ).join("");
  $("#ranges").querySelectorAll("button").forEach((b) => {
    b.onclick = () => {
      state.range = +b.dataset.sec;
      renderRanges();
      refreshCharts();
    };
  });
}

function renderMetricPicker() {
  const byDev = {};
  for (const it of state.catalog) (byDev[it.device] ||= []).push(it.metric);
  const groups = Object.keys(byDev).sort().map((dev) => {
    const items = byDev[dev]
      .filter((m) => !meta(m).flag)        // flags shown as cards only, not charted
      .map((m) => {
        const k = key(dev, m);
        const mt = meta(m);
        const on = state.selected.has(k);
        return `<label><input type="checkbox" data-k="${k}"${on ? " checked" : ""}>
          <span class="swatch" style="background:${mt.color}"></span>${mt.label}</label>`;
      }).join("");
    return `<div class="grp"><span class="gname">${DEV_LABEL[dev] || dev}</span>${items}</div>`;
  });
  $("#metrics").innerHTML = groups.join("");
  $("#metrics").querySelectorAll("input").forEach((inp) => {
    inp.onchange = () => {
      inp.checked ? state.selected.add(inp.dataset.k) : state.selected.delete(inp.dataset.k);
      refreshCharts();
    };
  });
}

// ---- charts ---------------------------------------------------------------

async function refreshCharts() {
  const host = $("#charts");
  const keys = [...state.selected];
  if (!keys.length) {
    host.innerHTML = `<div class="empty">Выберите метрики выше, чтобы увидеть историю.</div>`;
    return;
  }
  const to = Math.floor(Date.now() / 1000);
  const from = to - state.range;

  // Ensure a card per selected metric (preserve order of catalog).
  host.innerHTML = keys.map((k) => {
    const [, m] = k.split("/");
    const mt = meta(m);
    return `<div class="chart" data-k="${k}">
      <div class="head"><span class="title">${mt.label}</span><span class="cur"></span></div>
      <canvas></canvas>
    </div>`;
  }).join("");

  for (const k of keys) {
    const [dev, m] = k.split("/");
    const mt = meta(m);
    const el = host.querySelector(`.chart[data-k="${CSS.escape(k)}"]`);
    try {
      const d = await jget(`/api/series?device=${encodeURIComponent(dev)}&metric=${encodeURIComponent(m)}&from=${from}&to=${to}`);
      Plot(el.querySelector("canvas"), { series: [{ ts: d.ts, v: d.v, color: mt.color, label: mt.label }], unit: mt.unit });
      if (d.v.length) {
        const last = d.v[d.v.length - 1];
        el.querySelector(".cur").innerHTML = fmt(last, mt.unit);
      }
    } catch (e) {
      el.querySelector(".cur").textContent = "ошибка";
    }
  }
}

// ---- boot -----------------------------------------------------------------

async function loadCatalog() {
  try {
    const c = await jget("/api/catalog");
    state.catalog = c.items || [];
  } catch (e) {
    state.catalog = [];
  }
  if (!state.initialised && state.catalog.length) {
    // Default selection: a couple of headline metrics if present.
    const want = ["bms/soc", "inverter/ac_out_w", "inverter/pv_w", "bms/bms_w"];
    const have = new Set(state.catalog.map((i) => key(i.device, i.metric)));
    want.filter((k) => have.has(k)).forEach((k) => state.selected.add(k));
    state.initialised = true;
  }
  renderMetricPicker();
}

async function boot() {
  renderRanges();
  await loadCatalog();
  await refreshNow();
  await refreshCharts();

  setInterval(refreshNow, 5000);
  setInterval(async () => { await loadCatalog(); refreshCharts(); }, 30000);
  // Redraw charts on resize (canvas is sized from CSS box).
  let rt;
  window.addEventListener("resize", () => { clearTimeout(rt); rt = setTimeout(refreshCharts, 200); });
}

boot();
