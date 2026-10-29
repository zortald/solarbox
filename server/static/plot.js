// Minimal dependency-free time-series line chart on a <canvas>.
// Plot(canvas, { series: [{ts:[], v:[], color, label}], unit }) — one shared
// y-scale across the given series (so pass same-unit series only). Hi-DPI aware,
// with a hover crosshair + tooltip.

(function () {
  const PAD = { l: 44, r: 10, t: 8, b: 20 };

  function niceStep(span, target) {
    const raw = span / target;
    const pow = Math.pow(10, Math.floor(Math.log10(raw)));
    const n = raw / pow;
    const step = n < 1.5 ? 1 : n < 3 ? 2 : n < 7 ? 5 : 10;
    return step * pow;
  }

  function fmtTime(sec, span) {
    const d = new Date(sec * 1000);
    const p = (x) => String(x).padStart(2, "0");
    if (span > 3 * 86400) return `${p(d.getDate())}.${p(d.getMonth() + 1)}`;
    if (span > 86400) return `${p(d.getDate())}.${p(d.getMonth() + 1)} ${p(d.getHours())}:00`;
    return `${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  function fmtVal(v) {
    const a = Math.abs(v);
    if (a >= 1000) return v.toFixed(0);
    if (a >= 100) return v.toFixed(0);
    if (a >= 10) return v.toFixed(1);
    return v.toFixed(2);
  }

  // A gap is anything much longer than the series' usual step (and > 5 min).
  function gapLimit(s) {
    if (s._gap !== undefined) return s._gap;
    const d = [];
    for (let i = 1; i < s.ts.length; i++) d.push(s.ts[i] - s.ts[i - 1]);
    d.sort((a, b) => a - b);
    const med = d.length ? d[d.length >> 1] : 60;
    return (s._gap = Math.max(300, med * 5));
  }

  window.Plot = function (canvas, opts) {
    const series = (opts.series || []).filter((s) => s.ts && s.ts.length);
    const dpr = window.devicePixelRatio || 1;
    const cssW = canvas.clientWidth || 360;
    const cssH = canvas.clientHeight || 170;
    canvas.width = Math.round(cssW * dpr);
    canvas.height = Math.round(cssH * dpr);
    const ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);

    const css = getComputedStyle(document.body);
    const colLine = css.getPropertyValue("--line").trim() || "#2a3542";
    const colMuted = css.getPropertyValue("--muted").trim() || "#8b98a8";

    if (!series.length) {
      ctx.fillStyle = colMuted;
      ctx.font = "12px sans-serif";
      ctx.textAlign = "center";
      ctx.fillText("нет данных", cssW / 2, cssH / 2);
      return;
    }

    let tMin = Infinity, tMax = -Infinity, vMin = Infinity, vMax = -Infinity;
    for (const s of series) {
      for (let i = 0; i < s.ts.length; i++) {
        if (s.ts[i] < tMin) tMin = s.ts[i];
        if (s.ts[i] > tMax) tMax = s.ts[i];
        if (s.v[i] < vMin) vMin = s.v[i];
        if (s.v[i] > vMax) vMax = s.v[i];
      }
    }
    if (tMin === tMax) tMax = tMin + 1;
    if (vMin === vMax) { vMin -= 1; vMax += 1; }
    const vPad = (vMax - vMin) * 0.08;
    vMin -= vPad; vMax += vPad;

    const plotW = cssW - PAD.l - PAD.r;
    const plotH = cssH - PAD.t - PAD.b;
    const x = (t) => PAD.l + ((t - tMin) / (tMax - tMin)) * plotW;
    const y = (val) => PAD.t + plotH - ((val - vMin) / (vMax - vMin)) * plotH;

    // y grid + labels
    ctx.font = "10px sans-serif";
    ctx.fillStyle = colMuted;
    ctx.strokeStyle = colLine;
    ctx.lineWidth = 1;
    const yStep = niceStep(vMax - vMin, 4);
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (let gv = Math.ceil(vMin / yStep) * yStep; gv <= vMax; gv += yStep) {
      const py = y(gv);
      ctx.beginPath();
      ctx.moveTo(PAD.l, py);
      ctx.lineTo(cssW - PAD.r, py);
      ctx.stroke();
      ctx.fillText(fmtVal(gv), PAD.l - 5, py);
    }

    // x labels
    const span = tMax - tMin;
    const xStep = niceStep(span, 4);
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    for (let gt = Math.ceil(tMin / xStep) * xStep; gt <= tMax; gt += xStep) {
      ctx.fillText(fmtTime(gt, span), x(gt), cssH - PAD.b + 4);
    }

    // lines
    for (const s of series) {
      ctx.strokeStyle = s.color || "#4ea3ff";
      ctx.lineWidth = 1.6;
      ctx.lineJoin = "round";
      ctx.beginPath();
      for (let i = 0; i < s.ts.length; i++) {
        const px = x(s.ts[i]), py = y(s.v[i]);
        // Break the line across gaps in the data instead of bridging them.
        const gap = i && s.ts[i] - s.ts[i - 1] > gapLimit(s);
        i && !gap ? ctx.lineTo(px, py) : ctx.moveTo(px, py);
      }
      ctx.stroke();
    }

    // hover
    canvas.onmousemove = (e) => {
      const rect = canvas.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      draw(mx);
    };
    canvas.onmouseleave = () => draw(null);

    function draw(mx) {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      // cheap redraw: re-run Plot would reset handlers; instead overlay only.
      // Clear the hover layer by repainting base once, then crosshair.
      render();
      if (mx == null || mx < PAD.l || mx > cssW - PAD.r) return;
      const t = tMin + ((mx - PAD.l) / plotW) * (tMax - tMin);
      ctx.strokeStyle = colMuted;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(mx, PAD.t);
      ctx.lineTo(mx, PAD.t + plotH);
      ctx.stroke();

      const lines = [];
      for (const s of series) {
        let j = 0;
        for (let i = 1; i < s.ts.length; i++) {
          if (Math.abs(s.ts[i] - t) < Math.abs(s.ts[j] - t)) j = i;
        }
        const px = x(s.ts[j]), py = y(s.v[j]);
        ctx.fillStyle = s.color || "#4ea3ff";
        ctx.beginPath();
        ctx.arc(px, py, 3, 0, Math.PI * 2);
        ctx.fill();
        lines.push({ color: s.color, text: `${fmtVal(s.v[j])}${opts.unit ? " " + opts.unit : ""}`, ts: s.ts[j] });
      }
      // tooltip
      const tip = fmtTime(lines[0].ts, span) + (span <= 86400 ? "" : "");
      ctx.font = "11px sans-serif";
      const w = 64;
      let bx = mx + 8;
      if (bx + w > cssW) bx = mx - w - 8;
      const bh = 14 + lines.length * 13;
      ctx.fillStyle = "rgba(10,16,22,0.92)";
      ctx.strokeStyle = colLine;
      ctx.fillRect(bx, PAD.t + 2, w, bh);
      ctx.strokeRect(bx, PAD.t + 2, w, bh);
      ctx.fillStyle = colMuted;
      ctx.textAlign = "left";
      ctx.textBaseline = "top";
      ctx.fillText(tip, bx + 5, PAD.t + 5);
      lines.forEach((ln, i) => {
        ctx.fillStyle = ln.color || "#e6edf3";
        ctx.fillText(ln.text, bx + 5, PAD.t + 18 + i * 13);
      });
    }

    // base render (used by hover redraw)
    function render() {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, cssW, cssH);
      ctx.font = "10px sans-serif";
      ctx.fillStyle = colMuted;
      ctx.strokeStyle = colLine;
      ctx.lineWidth = 1;
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      for (let gv = Math.ceil(vMin / yStep) * yStep; gv <= vMax; gv += yStep) {
        const py = y(gv);
        ctx.beginPath(); ctx.moveTo(PAD.l, py); ctx.lineTo(cssW - PAD.r, py); ctx.stroke();
        ctx.fillStyle = colMuted; ctx.fillText(fmtVal(gv), PAD.l - 5, py);
      }
      ctx.textAlign = "center"; ctx.textBaseline = "top";
      for (let gt = Math.ceil(tMin / xStep) * xStep; gt <= tMax; gt += xStep) {
        ctx.fillText(fmtTime(gt, span), x(gt), cssH - PAD.b + 4);
      }
      for (const s of series) {
        ctx.strokeStyle = s.color || "#4ea3ff";
        ctx.lineWidth = 1.6; ctx.lineJoin = "round";
        ctx.beginPath();
        for (let i = 0; i < s.ts.length; i++) {
          const px = x(s.ts[i]), py = y(s.v[i]);
          // Break the line across gaps in the data instead of bridging them.
        const gap = i && s.ts[i] - s.ts[i - 1] > gapLimit(s);
        i && !gap ? ctx.lineTo(px, py) : ctx.moveTo(px, py);
        }
        ctx.stroke();
      }
    }
  };
})();
