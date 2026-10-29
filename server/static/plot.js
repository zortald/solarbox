// Minimal dependency-free time-series chart on a <canvas>.
//
// Plot(canvas, { series, unit, from, to, bands })
//   bands: [{ from, to, color? }] — shaded time ranges (e.g. power outages)
//   series: [{ ts:[], v:[], color, label, unit?, axis?: 'right', fixed?: [min, max] }]
//   from/to: the selected period — the x-axis always spans it, so short history
//            sits where it belongs instead of being stretched.
// Left-axis series share one scale; 'right' series get their own (labels on the
// right). Lines break across gaps in the data. Hi-DPI aware, hover tooltip.

(function () {
  const PAD = { l: 46, t: 8, b: 20 };

  function niceStep(span, target) {
    const raw = span / target;
    const pow = Math.pow(10, Math.floor(Math.log10(raw)));
    const n = raw / pow;
    return (n < 1.5 ? 1 : n < 3 ? 2 : n < 7 ? 5 : 10) * pow;
  }
  const p2 = (x) => String(x).padStart(2, '0');
  function fmtTime(sec, span) {
    const d = new Date(sec * 1000);
    if (span > 3 * 86400) return `${p2(d.getDate())}.${p2(d.getMonth() + 1)}`;
    if (span > 86400 + 3600) return `${p2(d.getDate())}.${p2(d.getMonth() + 1)} ${p2(d.getHours())}:00`;
    return `${p2(d.getHours())}:${p2(d.getMinutes())}`;
  }
  function fmtVal(v) {
    const a = Math.abs(v);
    return a >= 100 ? v.toFixed(0) : a >= 10 ? v.toFixed(1) : v.toFixed(2);
  }
  // Nice hour/day-aligned x ticks in local time.
  function xTicks(t0, t1) {
    const span = t1 - t0;
    const steps = [300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200, 86400, 172800, 604800];
    const step = steps.find((s) => span / s <= 6) || 604800;
    const off = new Date(t0 * 1000).getTimezoneOffset() * 60; // align to local midnight
    const out = [];
    // local = t - off (off = getTimezoneOffset in s), so ticks sit on local hours/midnight
    for (let t = Math.ceil((t0 - off) / step) * step + off; t <= t1; t += step) out.push(t);
    return out;
  }
  // A gap is anything much longer than the series' usual step (and > 5 min).
  function gapLimit(s) {
    if (s._gap !== undefined) return s._gap;
    const d = [];
    for (let i = 1; i < s.ts.length; i++) d.push(s.ts[i] - s.ts[i - 1]);
    d.sort((a, b) => a - b);
    return (s._gap = Math.max(300, (d.length ? d[d.length >> 1] : 60) * 5));
  }
  function range(list) {
    let lo = Infinity, hi = -Infinity;
    for (const s of list) {
      if (s.fixed) { lo = Math.min(lo, s.fixed[0]); hi = Math.max(hi, s.fixed[1]); continue; }
      for (const v of s.v || []) { if (v < lo) lo = v; if (v > hi) hi = v; }
    }
    if (!isFinite(lo)) { lo = 0; hi = 1; }
    if (lo === hi) { lo -= 1; hi += 1; }
    if (!list.some((s) => s.fixed)) { const p = (hi - lo) * 0.08; lo -= p; hi += p; }
    return [lo, hi];
  }

  window.Plot = function (canvas, opts) {
    const all = opts.series || [];
    const series = all.filter((s) => s.ts && s.ts.length);
    const dpr = window.devicePixelRatio || 1;
    const W = canvas.clientWidth || 360, H = canvas.clientHeight || 170;
    canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr);
    const ctx = canvas.getContext('2d');
    const css = getComputedStyle(document.body);
    const colLine = css.getPropertyValue('--line').trim() || '#2a3542';
    const colMuted = css.getPropertyValue('--muted').trim() || '#8b98a8';

    let t0 = opts.from, t1 = opts.to;
    if (!(t0 && t1)) {
      if (!series.length) {
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, W, H);
        ctx.fillStyle = colMuted; ctx.font = '12px sans-serif'; ctx.textAlign = 'center';
        ctx.fillText('нет данных', W / 2, H / 2);
        return;
      }
      t0 = Math.min(...series.map((s) => s.ts[0])); t1 = Math.max(...series.map((s) => s.ts[s.ts.length - 1]));
    }
    if (t0 === t1) t1 = t0 + 1;

    const left = series.filter((s) => s.axis !== 'right'), right = series.filter((s) => s.axis === 'right');
    const hasRight = right.length > 0;
    const padR = hasRight ? 40 : 10;
    const [lMin, lMax] = range(left);
    const [rMin, rMax] = range(right);
    const plotW = W - PAD.l - padR, plotH = H - PAD.t - PAD.b;
    const x = (t) => PAD.l + ((t - t0) / (t1 - t0)) * plotW;
    const yL = (v) => PAD.t + plotH - ((v - lMin) / (lMax - lMin)) * plotH;
    const yR = (v) => PAD.t + plotH - ((v - rMin) / (rMax - rMin)) * plotH;
    const yOf = (s) => (s.axis === 'right' ? yR : yL);
    const unitOf = (s) => s.unit ?? opts.unit ?? '';
    const leftColor = left.length === 1 ? left[0].color : colMuted;
    const rightColor = right.length === 1 ? right[0].color : colMuted;

    function render() {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);
      ctx.font = '10px sans-serif'; ctx.lineWidth = 1;
      // left scale + grid
      const ls = niceStep(lMax - lMin, 4);
      ctx.textBaseline = 'middle';
      for (let v = Math.ceil(lMin / ls) * ls; v <= lMax; v += ls) {
        const py = yL(v);
        ctx.strokeStyle = Math.abs(v) < ls / 1e6 && lMin < 0 ? colMuted : colLine; // zero line stands out
        ctx.beginPath(); ctx.moveTo(PAD.l, py); ctx.lineTo(W - padR, py); ctx.stroke();
        if (left.length) { ctx.fillStyle = leftColor; ctx.textAlign = 'right'; ctx.fillText(fmtVal(v), PAD.l - 5, py); }
      }
      // right scale (labels only)
      if (hasRight) {
        const rs = niceStep(rMax - rMin, 4);
        ctx.fillStyle = rightColor; ctx.textAlign = 'left';
        for (let v = Math.ceil(rMin / rs) * rs; v <= rMax; v += rs) ctx.fillText(fmtVal(v), W - padR + 5, yR(v));
      }
      // x ticks
      ctx.textAlign = 'center'; ctx.textBaseline = 'top';
      for (const t of xTicks(t0, t1)) {
        ctx.strokeStyle = colLine; ctx.globalAlpha = 0.35;
        ctx.beginPath(); ctx.moveTo(x(t), PAD.t); ctx.lineTo(x(t), PAD.t + plotH); ctx.stroke();
        ctx.globalAlpha = 1;
        ctx.fillStyle = colMuted; ctx.fillText(fmtTime(t, t1 - t0), x(t), H - PAD.b + 4);
      }
      // "now" marker when the period extends into the future (e.g. «Сегодня»)
      const now = Date.now() / 1000;
      if (now > t0 && now < t1 - 60) {
        ctx.strokeStyle = colMuted; ctx.setLineDash([3, 4]);
        ctx.beginPath(); ctx.moveTo(x(now), PAD.t); ctx.lineTo(x(now), PAD.t + plotH); ctx.stroke();
        ctx.setLineDash([]);
      }
      // lines (and shaded bands under them)
      ctx.save();
      ctx.beginPath(); ctx.rect(PAD.l, PAD.t - 2, plotW, plotH + 4); ctx.clip();
      for (const b of opts.bands || []) {
        const a = x(Math.max(b.from, t0)), z = x(Math.min(b.to, t1));
        if (z > a) { ctx.fillStyle = b.color || 'rgba(248,81,73,0.14)'; ctx.fillRect(a, PAD.t, Math.max(z - a, 1.5), plotH); }
      }
      for (const s of series) {
        const y = yOf(s), gap = gapLimit(s);
        ctx.strokeStyle = s.color || '#4ea3ff'; ctx.lineWidth = 1.6; ctx.lineJoin = 'round';
        ctx.beginPath();
        for (let i = 0; i < s.ts.length; i++) {
          const px = x(s.ts[i]), py = y(s.v[i]);
          i && s.ts[i] - s.ts[i - 1] <= gap ? ctx.lineTo(px, py) : ctx.moveTo(px, py);
        }
        ctx.stroke();
      }
      ctx.restore();
      if (!series.length) {
        ctx.fillStyle = colMuted; ctx.font = '12px sans-serif'; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
        ctx.fillText(all.length ? 'нет данных за период' : 'ничего не выбрано', PAD.l + plotW / 2, PAD.t + plotH / 2);
      }
    }

    function hover(mx) {
      render();
      if (mx == null || mx < PAD.l || mx > W - padR || !series.length) return;
      const t = t0 + ((mx - PAD.l) / plotW) * (t1 - t0);
      const rows = [];
      for (const s of series) {
        let j = 0;
        for (let i = 1; i < s.ts.length; i++) if (Math.abs(s.ts[i] - t) < Math.abs(s.ts[j] - t)) j = i;
        if (Math.abs(s.ts[j] - t) > gapLimit(s)) continue; // no data near the cursor
        ctx.fillStyle = s.color || '#4ea3ff';
        ctx.beginPath(); ctx.arc(x(s.ts[j]), yOf(s)(s.v[j]), 3, 0, Math.PI * 2); ctx.fill();
        rows.push({ color: s.color, text: `${s.label ? s.label + ': ' : ''}${fmtVal(s.v[j])} ${unitOf(s)}`, ts: s.ts[j] });
      }
      ctx.strokeStyle = colMuted; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(mx, PAD.t); ctx.lineTo(mx, PAD.t + plotH); ctx.stroke();
      if (!rows.length) return;
      ctx.font = '11px sans-serif';
      const head = fmtTime(rows[0].ts, Math.min(t1 - t0, 86400));
      const w = Math.max(ctx.measureText(head).width, ...rows.map((r) => ctx.measureText(r.text).width)) + 12;
      const h = 16 + rows.length * 14;
      let bx = mx + 10; if (bx + w > W) bx = mx - w - 10;
      ctx.fillStyle = 'rgba(10,16,22,0.92)'; ctx.strokeStyle = colLine;
      ctx.fillRect(bx, PAD.t + 2, w, h); ctx.strokeRect(bx, PAD.t + 2, w, h);
      ctx.textAlign = 'left'; ctx.textBaseline = 'top';
      ctx.fillStyle = colMuted; ctx.fillText(head, bx + 6, PAD.t + 5);
      rows.forEach((r, i) => { ctx.fillStyle = r.color || '#e6edf3'; ctx.fillText(r.text, bx + 6, PAD.t + 19 + i * 14); });
    }

    canvas.onmousemove = (e) => hover(e.clientX - canvas.getBoundingClientRect().left);
    canvas.ontouchmove = (e) => { if (e.touches[0]) hover(e.touches[0].clientX - canvas.getBoundingClientRect().left); };
    canvas.onmouseleave = () => hover(null);
    render();
  };

  // Shared period presets: key → [label, () => [from, to]]. «Сегодня» = local 00:00–24:00.
  const now = () => Math.floor(Date.now() / 1000);
  const midnight = () => { const d = new Date(); d.setHours(0, 0, 0, 0); return Math.floor(d / 1000); };
  window.PERIODS = {
    '1h': ['1ч', () => [now() - 3600, now()]],
    '6h': ['6ч', () => [now() - 21600, now()]],
    'today': ['Сегодня', () => [midnight(), midnight() + 86400]],
    '24h': ['24ч', () => [now() - 86400, now()]],
    '7d': ['7д', () => [now() - 604800, now()]],
    '30d': ['30д', () => [now() - 2592000, now()]],
  };
})();
