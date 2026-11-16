// Penukar markup Remotion (HTML + SVG sebaris, hasil renderToStaticMarkup) -> senarai lukisan NOVA.
//
// Hasil: [{ t: "svg", d: "<svg …>" } | { t: "shader", sksl, uniforms }] dalam susunan lukis.
// SVG sebaris disalin verbatim; kotak HTML (position:absolute, flex ringkas, teks) disusun
// oleh enjin susun atur mini di sini kerana tiada pelayar. Subset CSS yang disokong
// disenaraikan dalam docs/bake-remotion.md; sifat lain dilaporkan sebagai amaran.

const VOID = new Set(["br", "img", "hr", "input", "meta", "link"]);
const INHERIT = ["font-family", "font-size", "font-weight", "color", "letter-spacing", "line-height", "text-transform", "text-align", "text-shadow", "font-style", "white-space"];
const ENT = { "&quot;": '"', "&amp;": "&", "&lt;": "<", "&gt;": ">", "&#x27;": "'", "&#39;": "'" };
const unesc = (s) => (s.includes("&") ? s.replace(/&(quot|amp|lt|gt|#x27|#39);/g, (m) => ENT[m]) : s);
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const n = (v) => (Math.round(v * 1000) / 1000).toString();

// ------------------------------------------------------------------ penghurai
function parseAttrs(src) {
  const out = {};
  const re = /([^\s=/]+)(?:="([^"]*)")?/g;
  let m;
  while ((m = re.exec(src))) out[m[1]] = m[2] === undefined ? "" : unesc(m[2]);
  return out;
}
function parseStyle(s) {
  const out = {};
  if (!s) return out;
  for (const d of s.split(";")) {
    const i = d.indexOf(":");
    if (i > 0) out[d.slice(0, i).trim()] = d.slice(i + 1).trim();
  }
  return out;
}

/** Hurai aras HTML sahaja; setiap <svg> disimpan sebagai julat mentah. */
export function parse(html) {
  const root = { tag: "#root", attrs: {}, style: {}, kids: [] };
  const stack = [root];
  let i = 0;
  const N = html.length;
  while (i < N) {
    const lt = html.indexOf("<", i);
    if (lt < 0) break;
    if (lt > i) {
      const t = unesc(html.slice(i, lt));
      if (t) stack[stack.length - 1].kids.push({ tag: "#text", text: t });
    }
    if (html.startsWith("<!--", lt)) { i = html.indexOf("-->", lt) + 3; continue; }
    const gt = html.indexOf(">", lt);
    if (html[lt + 1] === "/") { stack.pop(); i = gt + 1; continue; }
    const selfClose = html[gt - 1] === "/";
    const head = html.slice(lt + 1, selfClose ? gt - 1 : gt);
    const sp = head.search(/\s/);
    const tag = sp < 0 ? head : head.slice(0, sp);
    const attrs = sp < 0 ? {} : parseAttrs(head.slice(sp + 1));
    const node = { tag, attrs, style: parseStyle(attrs.style), kids: [] };
    stack[stack.length - 1].kids.push(node);
    if (tag === "svg") {
      // cari </svg> sepadan (svg bersarang dikira)
      let depth = 1, p = gt + 1;
      const re = /<(\/?)svg\b/g;
      re.lastIndex = p;
      let m;
      while (depth > 0 && (m = re.exec(html))) { depth += m[1] ? -1 : 1; p = m.index; }
      node.inner = selfClose ? "" : html.slice(gt + 1, p);
      i = selfClose ? gt + 1 : html.indexOf(">", p) + 1;
      continue;
    }
    if (!selfClose && !VOID.has(tag)) stack.push(node);
    i = gt + 1;
  }
  return root;
}

// ------------------------------------------------------------------ nilai CSS
// Bentuk asas CSS `clip-path` (polygon/inset/circle/ellipse) pada kotak (x, y, w, h) -> elemen SVG.
function clipShape(value, x, y, w, h) {
  const m = value.match(/^(polygon|inset|circle|ellipse|path)\((.*)\)$/s);
  if (!m) return null;
  const arg = m[2].trim();
  if (m[1] === "path") {
    // path([evenodd,] "d") — koordinat relatif kepada kotak elemen
    const pm = arg.replace(/&quot;/g, '"').match(/^(?:(nonzero|evenodd)\s*,\s*)?["'](.*)["']$/s);
    if (!pm) return null;
    return `<path d="${pm[2]}"${pm[1] === "evenodd" ? ' clip-rule="evenodd"' : ""}${x || y ? ` transform="translate(${n(x)} ${n(y)})"` : ""}/>`;
  }
  const pos = (str) => { // "at cx cy" (lalai tengah)
    const [a = "50%", b = "50%"] = (str ?? "").trim().split(/\s+/).filter(Boolean)
      .map((t) => ({ center: "50%", left: "0%", top: "0%", right: "100%", bottom: "100%" })[t] ?? t);
    return [x + (px(a, w) ?? w / 2), y + (px(b, h) ?? h / 2)];
  };
  if (m[1] === "polygon") {
    const pts = splitTop(arg.replace(/^(nonzero|evenodd)\s*,/, "")).map((p) => { const [a, b] = p.trim().split(/\s+/); return `${n(x + (px(a, w) ?? 0))},${n(y + (px(b, h) ?? 0))}`; });
    return `<polygon points="${pts.join(" ")}"/>`;
  }
  if (m[1] === "inset") {
    const [box, round] = arg.split(/\s+round\s+/);
    const v = box.trim().split(/\s+/);
    const t = px(v[0], h) ?? 0, r = px(v[1] ?? v[0], w) ?? 0, b = px(v[2] ?? v[0], h) ?? 0, l = px(v[3] ?? v[1] ?? v[0], w) ?? 0;
    const cw = Math.max(0, w - l - r), ch = Math.max(0, h - t - b);
    const rad = round ? Math.min(px(round.trim().split(/[\s/]+/)[0], cw) ?? 0, cw / 2, ch / 2) : 0;
    return `<rect x="${n(x + l)}" y="${n(y + t)}" width="${n(cw)}" height="${n(ch)}"${rad ? ` rx="${n(rad)}"` : ""}/>`;
  }
  const [size, at] = arg.split(/\s*\bat\s+/);
  const [cx, cy] = pos(at);
  const side = (far, ext, c0) => (far ? Math.max(c0, ext - c0) : Math.min(c0, ext - c0));
  const radius = (tok, base, ext, c0) => tok === "farthest-side" ? side(true, ext, c0) : !tok || tok === "closest-side" ? side(false, ext, c0) : px(tok, base) ?? 0;
  const toks = size.trim().split(/\s+/).filter(Boolean);
  if (m[1] === "circle") {
    const r = toks[0] && !toks[0].endsWith("-side") ? px(toks[0], Math.hypot(w, h) / Math.SQRT2) ?? 0
      : Math[toks[0] === "farthest-side" ? "max" : "min"](side(toks[0] === "farthest-side", w, cx - x), side(toks[0] === "farthest-side", h, cy - y));
    return `<circle cx="${n(cx)}" cy="${n(cy)}" r="${n(r)}"/>`;
  }
  return `<ellipse cx="${n(cx)}" cy="${n(cy)}" rx="${n(radius(toks[0], w, w, cx - x))}" ry="${n(radius(toks[1], h, h, cy - y))}"/>`;
}
const px = (v, base) => {
  if (v === undefined || v === "" || v === "auto") return undefined;
  if (v.endsWith("%")) return (parseFloat(v) / 100) * base;
  const f = parseFloat(v);
  return Number.isFinite(f) ? f : undefined;
};
function splitTop(s, sep = ",") {
  const out = [];
  let depth = 0, cur = "";
  for (const ch of s) {
    if (ch === "(") depth++;
    else if (ch === ")") depth--;
    if (ch === sep && depth === 0) { out.push(cur.trim()); cur = ""; } else cur += ch;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}
const NAMED = { transparent: [0, 0, 0, 0], black: [0, 0, 0, 1], white: [255, 255, 255, 1], red: [255, 0, 0, 1] };
export function color(s) {
  s = s.trim().toLowerCase();
  if (NAMED[s]) return NAMED[s];
  if (s[0] === "#") {
    const h = s.slice(1);
    const d = h.length <= 4 ? [...h].map((c) => parseInt(c + c, 16)) : h.match(/../g).map((c) => parseInt(c, 16));
    return [d[0], d[1], d[2], d.length > 3 ? d[3] / 255 : 1];
  }
  const m = s.match(/rgba?\(([^)]+)\)/);
  if (m) {
    const p = m[1].split(/[,\s/]+/).filter(Boolean).map(parseFloat);
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  }
  return [0, 0, 0, 1];
}
const paint = (c, attr = "fill") => `${attr}="rgb(${n(c[0])},${n(c[1])},${n(c[2])})"` + (c[3] < 1 ? ` ${attr}-opacity="${n(c[3])}"` : "");

/** transform CSS -> senarai transform SVG, berputar pada (cx, cy). Pulang "" jika identiti. */
function cssTransform(t, cx, cy) {
  if (!t || t === "none") return "";
  const ops = [];
  let ident = true;
  for (const m of t.matchAll(/([a-zA-Z]+)\(([^)]*)\)/g)) {
    const a = m[2].split(",").map((v) => parseFloat(v));
    switch (m[1]) {
      case "translate": if (a[0] || a[1]) { ops.push(`translate(${n(a[0])} ${n(a[1] || 0)})`); ident = false; } break;
      case "translateX": if (a[0]) { ops.push(`translate(${n(a[0])} 0)`); ident = false; } break;
      case "translateY": if (a[0]) { ops.push(`translate(0 ${n(a[0])})`); ident = false; } break;
      case "scale": if (a[0] !== 1 || (a.length > 1 && a[1] !== 1)) { ops.push(`scale(${a[0]}${a.length > 1 ? " " + a[1] : ""})`); ident = false; } break;
      case "scaleX": if (a[0] !== 1) { ops.push(`scale(${a[0]} 1)`); ident = false; } break;
      case "scaleY": if (a[0] !== 1) { ops.push(`scale(1 ${a[0]})`); ident = false; } break;
      case "rotate": if (a[0]) { ops.push(`rotate(${a[0]})`); ident = false; } break;
      case "matrix": ops.push(`matrix(${a.join(" ")})`); ident = false; break;
      default: throw new Error(`transform CSS tidak disokong: ${m[1]}`);
    }
  }
  if (ident) return "";
  return `translate(${n(cx)} ${n(cy)}) ${ops.join(" ")} translate(${n(-cx)} ${n(-cy)})`;
}

/** buang fungsi filter yang tiada kesan; pulang "" jika semuanya tiada kesan */
function cssFilter(f) {
  if (!f || f === "none") return "";
  if (f.startsWith("url(")) return f;
  const keep = [];
  for (const m of f.matchAll(/([a-z-]+)\(([^)]*)\)/g)) {
    const v = parseFloat(m[2]);
    const noop = ((m[1] === "brightness" || m[1] === "saturate" || m[1] === "contrast" || m[1] === "opacity") && Math.abs(v - 1) < 0.002)
      || ((m[1] === "blur" || m[1] === "sepia" || m[1] === "hue-rotate" || m[1] === "grayscale" || m[1] === "invert") && Math.abs(v) < 0.002);
    if (!noop) keep.push(m[1] === "blur" ? `blur(${n(v)})` : m[0]);
  }
  return keep.join(" ");
}

/**
 * Jejari sudut kotak CSS: `border-radius` (1–4 nilai, pilihan "/ menegak", px atau %) dan
 * `border-<sudut>-radius`. Pulang [[rx, ry] × 4] ikut urutan kiri-atas, kanan-atas, kanan-bawah,
 * kiri-bawah, sudah dikecilkan supaya muat (aturan CSS), atau null jika tiada jejari.
 */
function cornerRadii(st, w, h) {
  const four = (v) => [v[0], v[1] ?? v[0], v[2] ?? v[0], v[3] ?? v[1] ?? v[0]];
  let R = [[0, 0], [0, 0], [0, 0], [0, 0]];
  if (st["border-radius"]) {
    const [hs, vs] = st["border-radius"].split("/").map((p) => p.trim().split(/\s+/));
    const H4 = four(hs), V4 = four(vs ?? hs);
    R = H4.map((v, i) => [px(v, w) ?? 0, px(V4[i], h) ?? 0]);
  }
  ["top-left", "top-right", "bottom-right", "bottom-left"].forEach((c, i) => {
    const v = st[`border-${c}-radius`];
    if (v) { const t = v.trim().split(/\s+/); R[i] = [px(t[0], w) ?? 0, px(t[1] ?? t[0], h) ?? 0]; }
  });
  if (R.every((r) => !(r[0] > 0 && r[1] > 0))) return null;
  const f = Math.min(1, w / (R[0][0] + R[1][0] || 1), w / (R[3][0] + R[2][0] || 1), h / (R[0][1] + R[3][1] || 1), h / (R[1][1] + R[2][1] || 1));
  return R.map((r) => [Math.max(0, r[0] * f), Math.max(0, r[1] * f)]);
}
/** kotak (x, y, w, h) dengan jejari R (daripada cornerRadii, pilihan dikecilkan `inset` px) sebagai elemen SVG */
function boxShape(x, y, w, h, R, attrs = "", inset = 0) {
  if (R && inset) R = R.map((r) => [Math.max(0, r[0] - inset), Math.max(0, r[1] - inset)]);
  if (!R) return `<rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}"${attrs ? " " + attrs : ""}/>`;
  if (R.every((r) => r[0] === R[0][0] && r[1] === R[0][1])) return `<rect x="${n(x)}" y="${n(y)}" width="${n(w)}" height="${n(h)}" rx="${n(R[0][0])}" ry="${n(R[0][1])}"${attrs ? " " + attrs : ""}/>`;
  const [tl, tr, br, bl] = R;
  const arc = (r, ex, ey) => (r[0] > 0 && r[1] > 0 ? `A${n(r[0])} ${n(r[1])} 0 0 1 ${n(ex)} ${n(ey)}` : `L${n(ex)} ${n(ey)}`);
  const d = `M${n(x + tl[0])} ${n(y)}L${n(x + w - tr[0])} ${n(y)}${arc(tr, x + w, y + tr[1])}L${n(x + w)} ${n(y + h - br[1])}${arc(br, x + w - br[0], y + h)}L${n(x + bl[0])} ${n(y + h)}${arc(bl, x, y + h - bl[1])}L${n(x)} ${n(y + tl[1])}${arc(tl, x + tl[0], y)}Z`;
  return `<path d="${d}"${attrs ? " " + attrs : ""}/>`;
}

let uidN = 0;
const uid = (p) => `${p}${(uidN++).toString(36)}`;

/** latar CSS (warna / linear-gradient / radial-gradient, berlapis) -> elemen SVG */
function background(bg, x, y, w, h, R, warn) {
  if (!bg || bg === "none" || bg === "transparent") return "";
  let out = "";
  const shape = (fill) => boxShape(x, y, w, h, R, fill);
  for (const layer of splitTop(bg).reverse()) {
    const g0 = layer.match(/^(repeating-)?(linear|radial)-gradient\((.*)\)$/s);
    const g = g0 && [g0[0], g0[2], g0[3]], repeating = !!g0?.[1];
    if (!g) {
      if (/^[a-z-]+\(/.test(layer) && !/^(rgba?|hsla?)\(/.test(layer)) { warn?.(`latar CSS belum disokong: ${layer.slice(0, 50)}`); continue; }
      const c = color(layer);
      if (c[3] > 0) out += shape(paint(c));
      continue;
    }
    const parts = splitTop(g[2]);
    const id = uid("bg");
    let geom, len = 1, first = 0, line = null;
    if (g[1] === "linear") {
      let deg = 180;
      if (/deg$/.test(parts[0])) { deg = parseFloat(parts[0]); first = 1; }
      else if (parts[0].startsWith("to ")) {
        deg = { "to top": 0, "to right": 90, "to bottom": 180, "to left": 270 }[parts[0]] ?? 180;
        first = 1;
      }
      const a = (deg * Math.PI) / 180;
      len = Math.abs(w * Math.sin(a)) + Math.abs(h * Math.cos(a));
      const dx = (Math.sin(a) * len) / 2, dy = (-Math.cos(a) * len) / 2;
      line = [x + w / 2 - dx, y + h / 2 - dy, 2 * dx, 2 * dy];
      geom = `<linearGradient id="${id}" gradientUnits="userSpaceOnUse" x1="${n(x + w / 2 - dx)}" y1="${n(y + h / 2 - dy)}" x2="${n(x + w / 2 + dx)}" y2="${n(y + h / 2 + dy)}">`;
    } else {
      // [bentuk] [saiz] at X Y
      let cx = w / 2, cy = h / 2, rxG, ryG, circle = false;
      let spec = "";
      if (/\bat\b|^(circle|ellipse|closest|farthest)/.test(parts[0]) || /^[\d.]+(px|%)/.test(parts[0])) { spec = parts[0]; first = 1; }
      const [sizePart, posPart] = spec.split(/\bat\b/).map((v) => (v || "").trim());
      if (posPart) {
        const p = posPart.split(/\s+/);
        cx = px(p[0], w) ?? w / 2;
        cy = px(p[1] ?? "50%", h) ?? h / 2;
      }
      const toks = sizePart.split(/\s+/).filter(Boolean);
      if (toks[0] === "circle") { circle = true; toks.shift(); } else if (toks[0] === "ellipse") toks.shift();
      if (toks.length === 1 && /px$/.test(toks[0])) { rxG = ryG = parseFloat(toks[0]); }
      else if (toks.length === 2) { rxG = px(toks[0], w); ryG = px(toks[1], h); }
      else {
        // lalai: farthest-corner
        const fx = Math.max(cx, w - cx), fy = Math.max(cy, h - cy);
        if (circle) rxG = ryG = Math.hypot(fx, fy);
        else { rxG = fx * Math.SQRT2; ryG = fy * Math.SQRT2; }
      }
      len = rxG;
      line = [x + cx, y + cy, rxG, ryG];
      geom = `<radialGradient id="${id}" gradientUnits="userSpaceOnUse" cx="0" cy="0" r="1" gradientTransform="translate(${n(x + cx)} ${n(y + cy)}) scale(${n(rxG)} ${n(ryG)})">`;
    }
    // hentian: kedudukan hilang diagih sekata; rgb hentian lutsinar ikut jiran (interpolasi pra-darab CSS)
    const stops = parts.slice(first).map((p) => {
      const m = p.match(/^(.*?)(?:\s+(-?[\d.]+)(px|%))?$/s);
      return { c: color(m[1]), o: m[2] === undefined ? undefined : m[3] === "%" ? parseFloat(m[2]) / 100 : parseFloat(m[2]) / len };
    });
    if (stops[0].o === undefined) stops[0].o = 0;
    if (stops[stops.length - 1].o === undefined) stops[stops.length - 1].o = 1;
    for (let i = 1; i < stops.length - 1; i++) {
      if (stops[i].o !== undefined) continue;
      let j = i;
      while (stops[j].o === undefined) j++;
      const a = stops[i - 1].o, b = stops[j].o;
      for (let k = i; k < j; k++) stops[k].o = a + ((b - a) * (k - i + 1)) / (j - i + 1);
    }
    if (repeating) {
      // satu kitaran = hentian pertama..terakhir; vektor gradien dipendekkan kepadanya dan diulang
      const o0 = stops[0].o, span = stops[stops.length - 1].o - o0;
      if (span > 0) {
        for (const st of stops) st.o = (st.o - o0) / span;
        geom = g[1] === "linear"
          ? `<linearGradient id="${id}" gradientUnits="userSpaceOnUse" spreadMethod="repeat" x1="${n(line[0] + line[2] * o0)}" y1="${n(line[1] + line[3] * o0)}" x2="${n(line[0] + line[2] * (o0 + span))}" y2="${n(line[1] + line[3] * (o0 + span))}">`
          : `<radialGradient id="${id}" gradientUnits="userSpaceOnUse" spreadMethod="repeat" cx="0" cy="0" r="1" gradientTransform="translate(${n(line[0])} ${n(line[1])}) scale(${n(line[2] * span)} ${n(line[3] * span)})">`;
      }
    }
    stops.forEach((s, i) => {
      if (s.c[3] === 0) {
        const nb = stops[i + 1] ?? stops[i - 1];
        if (nb) s.c = [nb.c[0], nb.c[1], nb.c[2], 0];
      }
    });
    // CSS menginterpolasi warna pra-darab alfa; SVG tidak. Antara dua hentian yang alfa DAN warnanya
    // berbeza, hentian perantaraan mengikut lengkung pra-darab (jika tidak, bahagian tengah terlalu cerah).
    for (let i = stops.length - 2; i >= 0; i--) {
      const a = stops[i], b = stops[i + 1];
      if (a.c[3] === b.c[3] || (a.c[0] === b.c[0] && a.c[1] === b.c[1] && a.c[2] === b.c[2]) || b.o <= a.o) continue;
      const mid = [];
      for (let k = 1; k < 12; k++) {
        const t = k / 12, al = a.c[3] + (b.c[3] - a.c[3]) * t;
        const ch = (j) => (al > 0 ? (a.c[j] * a.c[3] * (1 - t) + b.c[j] * b.c[3] * t) / al : b.c[j]);
        mid.push({ c: [ch(0), ch(1), ch(2), al], o: a.o + (b.o - a.o) * t });
      }
      stops.splice(i + 1, 0, ...mid);
    }
    const tail = g[1] === "linear" ? "</linearGradient>" : "</radialGradient>";
    out += `<defs>${geom}${stops.map((s) => `<stop offset="${n(s.o)}" stop-color="rgb(${n(s.c[0])},${n(s.c[1])},${n(s.c[2])})"${s.c[3] < 1 ? ` stop-opacity="${n(s.c[3])}"` : ""}/>`).join("")}${tail}</defs>`;
    out += shape(`fill="url(#${id})"`);
  }
  return out;
}

function shadows(s) {
  // "dx dy blur [spread] warna, …" -> [{dx,dy,blur,spread,c}]
  return splitTop(s).map((p) => {
    const nums = [...p.matchAll(/(?:^|\s)(-?[\d.]+)(?:px)?(?=\s|$)/g)].map((m) => parseFloat(m[1]));
    const c = p.replace(/(?:^|\s)-?[\d.]+(?:px)?(?=\s|$)/g, " ").trim();
    return { dx: nums[0] || 0, dy: nums[1] || 0, blur: nums[2] || 0, spread: nums[3] || 0, c: color(c || "#000") };
  });
}

// ------------------------------------------------------------------ teks
function fontFor(fonts, st) {
  const w = parseInt(st["font-weight"] || "400", 10) || (st["font-weight"] === "bold" ? 700 : 400);
  return fonts.pick((st["font-family"] || "").replace(/&quot;|&#x27;/g, ""), w);
}
function textMetrics(fonts, st) {
  const f = fontFor(fonts, st);
  const size = parseFloat(st["font-size"] || "16");
  const k = size / f.unitsPerEm;
  const asc = f.ascender * k, desc = -f.descender * k;
  const gap = (f.tables.hhea?.lineGap || 0) * k;
  const lh = st["line-height"] && st["line-height"] !== "normal"
    ? (/px$/.test(st["line-height"]) ? parseFloat(st["line-height"]) : parseFloat(st["line-height"]) * size)
    : asc + desc + gap;
  const ls = parseFloat(st["letter-spacing"] || "0") || 0;
  return { f, size, asc, desc, lh, ls, base: (lh - (asc + desc)) / 2 + asc };
}
const isInlineKid = (k) => k.tag === "#text" || k.tag === "span" || k.tag === "b" || k.tag === "i" || k.tag === "strong" || k.tag === "em" || k.tag === "br";
const isInlineBox = (el) => el.style.display !== "flex" && el.kids.length > 0 && el.kids.every(isInlineKid) && el.kids.some((k) => k.tag === "#text" || k.kids?.length);

/** kumpul larian teks (gaya diwarisi) daripada kandungan sebaris */
function runs(el, st, out = [], box = null) {
  for (const k of el.kids) {
    if (k.tag === "#text") out.push({ text: k.text, st, box });
    else if (k.tag === "br") out.push({ br: true, st });
    else {
      // span dengan transform/opacity sendiri (animasi per huruf) = satu kotak atom
      const ks = k.style;
      const own = (ks.transform && ks.transform !== "none") || (ks.opacity !== undefined && ks.opacity !== "1") || (ks.filter && ks.filter !== "none");
      runs(k, inherit(st, ks), out, own ? { transform: ks.transform, opacity: ks.opacity === undefined ? 1 : parseFloat(ks.opacity), filter: ks.filter, origin: ks["transform-origin"] } : box);
    }
  }
  return out;
}
const inherit = (parent, own) => {
  const st = {};
  for (const k of INHERIT) if (parent[k] !== undefined) st[k] = parent[k];
  return Object.assign(st, own);
};

/** susun teks sebaris: pecah baris pada lebar `avail`; pulang {w, h, lines} */
function layoutText(ctx, el, st, avail) {
  const words = [];
  for (const r of runs(el, st)) {
    if (r.br) { words.push({ br: true }); continue; }
    const m = textMetrics(ctx.fonts, r.st);
    const pre = (r.st["white-space"] || "").startsWith("pre");
    let text = pre ? r.text : r.text.replace(/\s+/g, " ");
    if (r.st["text-transform"] === "uppercase") text = text.toUpperCase();
    const adv = (s) => (s ? ctx.fonts.advance(m.f, s, m.size) + m.ls * s.length : 0);
    const c = color(r.st.color || "#000");
    if (pre) {
      // ruang dikekalkan dan tidak memecah baris
      text.split("\n").forEach((part, i) => {
        if (i) words.push({ br: true });
        if (part) words.push({ word: part, space: false, w: adv(part), sw: 0, m, c, box: r.box, keep: true });
      });
      continue;
    }
    for (const tok of text.match(/[^ ]+ ?| /g) || []) {
      const word = tok.trimEnd();
      const space = tok.endsWith(" ");
      words.push({ word, space, w: adv(word), sw: space ? adv(" ") : 0, m, c, box: r.box });
    }
  }
  const lines = [];
  let cur = { items: [], w: 0, tail: 0 };
  const flush = () => { if (cur.items.length) lines.push(cur); cur = { items: [], w: 0, tail: 0 }; };
  for (const wd of words) {
    if (wd.br) { flush(); continue; }
    if (!wd.word && !cur.items.length) continue; // ruang di awal baris
    if (!wd.keep && cur.items.length && cur.w + cur.tail + wd.w > avail + 0.01) flush();
    if (!wd.word) { cur.tail += wd.sw; continue; }
    cur.items.push({ ...wd, x: cur.w + cur.tail });
    cur.w += cur.tail + wd.w;
    cur.tail = wd.sw;
  }
  flush();
  let h = 0, w = 0;
  for (const l of lines) {
    l.lh = Math.max(...l.items.map((i) => i.m.lh));
    l.base = Math.max(...l.items.map((i) => i.m.base));
    l.y = h;
    h += l.lh;
    w = Math.max(w, l.w);
  }
  return { w, h, lines, base: lines[0]?.base ?? 0 };
}

function emitText(ctx, T, st, x, y, boxW) {
  const align = st["text-align"] || "left";
  const sh = st["text-shadow"] && st["text-shadow"] !== "none" ? shadows(st["text-shadow"]) : null;
  let out = "";
  for (const l of T.lines) {
    const lx = x + (align === "center" ? (boxW - l.w) / 2 : align === "right" ? boxW - l.w : 0);
    const by = y + l.y + l.base;
    // segmen = perkataan berturutan dalam kotak (inline-block) yang sama; tanpa kotak = satu segmen
    const segs = [];
    for (const it of l.items) {
      const last = segs[segs.length - 1];
      if (last && last.box === (it.box || null) && !it.box) last.items.push(it);
      else segs.push({ box: it.box || null, items: [it] });
    }
    for (const seg of segs) {
      // kumpulkan perkataan berturutan yang sama gaya menjadi satu tspan
      const spans = [];
      for (const it of seg.items) {
        const key = it.m.f.__key + "|" + it.m.size + "|" + it.m.ls + "|" + it.c.join(",");
        const last = spans[spans.length - 1];
        if (last && last.key === key) { last.text += (last.space ? " " : "") + it.word; last.space = it.space; }
        else spans.push({ key, x: it.x, text: it.word, space: it.space, it });
      }
      const body = (colored) => spans.map((s) => {
        const m = s.it.m;
        // aksara yang tiada dalam fon mendapat tspan sendiri dengan fon sandarannya
        let px0 = lx + s.x;
        return ctx.fonts.split(m.f, s.text + (s.space ? " " : "")).map((r) => {
          const t = `<tspan x="${n(px0)}" font-family="${r.f.__family}" font-weight="${r.f.__weight}" font-size="${n(m.size)}"${m.ls ? ` letter-spacing="${n(m.ls)}"` : ""}${colored ? " " + paint(s.it.c) : ""}>${esc(r.text)}</tspan>`;
          px0 += r.f.getAdvanceWidth(r.text, m.size) + m.ls * r.text.length;
          return t;
        }).join("");
      }).join("");
      const first = seg.items[0], lastIt = seg.items[seg.items.length - 1];
      const sx0 = lx + first.x, sw = lastIt.x + lastIt.w - first.x;
      let open = "", close = "";
      if (seg.box) {
        const o = (seg.box.origin || "50% 50%").split(/\s+/);
        const cx = sx0 + (px(o[0], sw) ?? sw / 2), cy = y + l.y + (px(o[1] ?? "50%", l.lh) ?? l.lh / 2);
        if (seg.box.opacity <= 0) continue;
        const t = cssTransform(seg.box.transform, cx, cy);
        const f = cssFilter(seg.box.filter);
        open = `<g${t ? ` transform="${t}"` : ""}${seg.box.opacity < 1 ? ` opacity="${n(seg.box.opacity)}"` : ""}${f ? ` filter="${f}"` : ""}>`;
        close = "</g>";
      }
      out += open;
      if (sh) {
        // bayang teks CSS mengikut bentuk glif penuh (tidak dilemahkan alfa warna teks)
        const pad = Math.max(...sh.map((s) => s.blur * 1.5 + Math.abs(s.dx) + Math.abs(s.dy))) + 4;
        const id = uid("ts");
        let prim = "", merge = "";
        sh.slice().reverse().forEach((s, i) => {
          prim += `<feGaussianBlur in="SourceAlpha" stdDeviation="${n(s.blur / 2)}" result="b${i}"/><feOffset in="b${i}" dx="${n(s.dx)}" dy="${n(s.dy)}" result="o${i}"/><feFlood flood-color="rgb(${s.c[0]},${s.c[1]},${s.c[2]})" flood-opacity="${n(s.c[3])}"/><feComposite operator="in" in2="o${i}" result="s${i}"/>`;
          merge += `<feMergeNode in="s${i}"/>`;
        });
        out += `<defs><filter id="${id}" filterUnits="userSpaceOnUse" x="${n(sx0 - pad)}" y="${n(y + l.y - pad)}" width="${n(sw + pad * 2)}" height="${n(l.lh + pad * 2)}" color-interpolation-filters="sRGB">${prim}<feMerge>${merge}</feMerge></filter></defs>`;
        out += `<text xml:space="preserve" y="${n(by)}" fill="#000" filter="url(#${id})">${body(false)}</text>`;
      }
      out += `<text xml:space="preserve" y="${n(by)}">${body(true)}</text>` + close;
    }
  }
  return out;
}

// ------------------------------------------------------------------ susun atur kotak
/** margin [atas, kanan, bawah, kiri] dalam px; "auto" = NaN */
const margins = (st) => {
  const one = (v) => (v === "auto" ? NaN : parseFloat(v) || 0);
  const v = (st.margin || "0").trim().split(/\s+/).map(one);
  const m = [v[0], v[1] ?? v[0], v[2] ?? v[0], v[3] ?? v[1] ?? v[0]];
  ["margin-top", "margin-right", "margin-bottom", "margin-left"].forEach((k, i) => { if (st[k] !== undefined) m[i] = one(st[k]); });
  return m;
};
const sides = (v) => {
  // "a", "a b", "a b c", "a b c d" -> [atas, kanan, bawah, kiri]
  const p = (v || "0").split(/\s+/).map((s) => parseFloat(s) || 0);
  return p.length === 1 ? [p[0], p[0], p[0], p[0]] : p.length === 2 ? [p[0], p[1], p[0], p[1]] : p.length === 3 ? [p[0], p[1], p[2], p[1]] : p;
};
const borderOf = (v) => {
  if (!v) return null;
  const m = v.match(/^([\d.]+)px\s+\w+\s+(.+)$/);
  return m ? { w: parseFloat(m[1]), c: color(m[2]) } : null;
};

/** Kira saiz `el` (kotak sempadan) dan susun anak. `cw/ch` = blok pengandung; `fit` = kecut-ikut-kandungan. */
// Sifat CSS yang difahami penukar (atau yang tiada kesan pada gambar). Yang lain diberi amaran
// supaya jurang kelihatan semasa bake, bukan sebagai gambar yang salah.
const KNOWN_CSS = new Set(["position", "top", "left", "right", "bottom", "inset", "width", "height", "max-width", "display", "flex-direction",
  "align-items", "justify-content", "gap", "padding", "padding-top", "padding-right", "padding-bottom", "padding-left", "margin", "margin-top",
  "margin-right", "margin-bottom", "margin-left", "transform", "transform-origin", "opacity", "filter", "mix-blend-mode", "overflow", "clip-path",
  "background", "background-color", "border", "border-left", "border-radius", "border-top-left-radius", "border-top-right-radius",
  "border-bottom-right-radius", "border-bottom-left-radius", "box-shadow", "color", "font-family", "font-size", "font-weight",
  "font-style", "letter-spacing", "line-height", "text-align", "text-transform", "text-shadow", "white-space",
  // tiada kesan pada frame yang dilukis
  "pointer-events", "-webkit-clip-path", "box-sizing", "user-select", "cursor", "will-change", "isolation", "-webkit-font-smoothing"]);
const KNOWN_VALUE = { display: ["flex", "block", "none", "inline", "inline-block"], position: ["absolute", "fixed", "relative", "static"], overflow: ["hidden", "visible"] };

function measure(ctx, el, cw, ch, inh, fit) {
  const s = el.style;
  for (const k in s) {
    if (!KNOWN_CSS.has(k)) ctx.warn(`sifat CSS belum disokong: ${k}`);
    else if (KNOWN_VALUE[k] && !KNOWN_VALUE[k].includes(s[k])) ctx.warn(`nilai CSS belum disokong: ${k}: ${s[k]}`);
  }
  const st = (el.st = inherit(inh, s));
  if (s.display === "none") { el.skip = true; el.w = el.h = 0; return; }
  if (el.tag === "svg") {
    el.w = parseFloat(el.attrs.width ?? "300") || 0;
    el.h = parseFloat(el.attrs.height ?? "150") || 0;
    return;
  }
  if (el.tag.startsWith("nova-")) { el.w = cw; el.h = ch; return; }
  const abs = s.position === "absolute" || s.position === "fixed";
  const pad = sides(s.padding);
  for (const [i, k] of ["padding-top", "padding-right", "padding-bottom", "padding-left"].entries()) if (s[k]) pad[i] = parseFloat(s[k]);
  const b = borderOf(s.border);
  const bw = [b?.w || 0, b?.w || 0, b?.w || 0, b?.w || 0];
  const bl = borderOf(s["border-left"]);
  if (bl) bw[3] = bl.w;
  el.pad = pad; el.bw = bw; el.border = b; el.borderLeft = bl;
  const ins = s.inset !== undefined ? sides(s.inset) : null;
  const L = ins ? ins[3] : px(s.left, cw), R = ins ? ins[1] : px(s.right, cw), Tp = ins ? ins[0] : px(s.top, ch), B = ins ? ins[2] : px(s.bottom, ch);
  el.abs = abs; el.L = L; el.R = R; el.T = Tp; el.B = B;
  const mx = pad[1] + pad[3] + bw[1] + bw[3], my = pad[0] + pad[2] + bw[0] + bw[2];
  // lebar
  let w = px(s.width, cw);
  if (w === undefined && abs && L !== undefined && R !== undefined) w = cw - L - R;
  const maxW = px(s["max-width"], cw);
  let avail = w !== undefined ? w : abs ? cw - (L || 0) - (R || 0) : cw;
  if (maxW !== undefined) avail = Math.min(avail, maxW);
  const shrink = w === undefined && (abs || fit);
  let h = px(s.height, ch);
  if (h === undefined && abs && Tp !== undefined && B !== undefined) h = ch - Tp - B;
  const iw = Math.max(0, avail - mx); // lebar kandungan tersedia
  let contentW = 0, contentH = 0;
  // Teks bebas di sebelah elemen blok (atau terus dalam bekas flex) duduk dalam kotak tanpa nama, seperti CSS.
  if (!isInlineBox(el) && el.kids.some((k) => k.tag === "#text" && !/^\s*$/.test(k.text))) {
    const kids = [];
    let run = null;
    for (const k of el.kids) {
      if (isInlineKid(k)) { if (!run) kids.push((run = { tag: "div", attrs: {}, style: {}, kids: [], anon: true })); run.kids.push(k); }
      else { run = null; kids.push(k); }
    }
    el.kids = kids.filter((k) => !k.anon || k.kids.some((t) => t.tag !== "#text" || !/^\s*$/.test(t.text)));
  }
  const flex = s.display === "flex";
  const row = flex && (s["flex-direction"] || "row") === "row";
  const flow = el.kids.filter((k) => k.tag !== "#text" || !/^\s*$/.test(k.text));
  if (isInlineBox(el)) {
    el.text = layoutText(ctx, el, st, iw);
    contentW = el.text.w; contentH = el.text.h;
    el.base = el.text.base;
  } else {
    const gap = parseFloat(s.gap || "0") || 0;
    const alignItems = s["align-items"] || "stretch";
    // anak mutlak diukur selepas saiz sendiri diketahui
    const inflow = flow.filter((k) => !(k.style?.position === "absolute" || k.style?.position === "fixed") && k.tag !== "#text");
    let cx = 0, cy = 0, maxBase = 0;
    for (const k of inflow) {
      const kfit = row || (flex && alignItems !== "stretch");
      measure(ctx, k, iw, h !== undefined ? h - my : 0, st, kfit);
      const mg = margins(k.style), mt = mg[0] || 0, mb = mg[2] || 0;
      k.marginAutoX = Number.isNaN(mg[1]) && Number.isNaN(mg[3]);
      k.marginL = mg[3] || 0;
      if (row) {
        cx += mg[3] || 0; k.fx = cx; cx += k.w + (mg[1] || 0) + gap; contentH = Math.max(contentH, k.h);
        maxBase = Math.max(maxBase, k.base ?? k.h);
      } else {
        cy += mt; k.fy = cy; cy += k.h + mb + gap; contentW = Math.max(contentW, k.w);
      }
    }
    if (row) {
      contentW = Math.max(0, cx - gap);
      for (const k of inflow) {
        k.fy = alignItems === "baseline" ? maxBase - (k.base ?? k.h) : alignItems === "center" ? (contentH - k.h) / 2 : 0;
        if (alignItems === "baseline") contentH = Math.max(contentH, k.fy + k.h);
      }
      el.base = maxBase;
    } else {
      contentH = Math.max(0, cy - (inflow.length ? gap : 0));
    }
    el.inflow = inflow; el.row = row; el.alignItems = alignItems; el.flex = flex;
  }
  el.w = w !== undefined ? w : shrink ? Math.min(avail, contentW + mx) : avail;
  el.h = h !== undefined ? h : contentH + my;
  const innerW = el.w - mx, innerH = el.h - my;
  if (el.inflow) {
    const jc = s["justify-content"] || "flex-start";
    for (const k of el.inflow) {
      if (el.row) {
        if (jc === "center") k.fx += (innerW - contentW) / 2;
        if (el.alignItems === "center") k.fy = (innerH - k.h) / 2;
      } else {
        // paksi silang lajur flex / aliran blok
        if (el.flex && el.alignItems === "center") k.fx = (innerW - k.w) / 2;
        else if (el.flex && el.alignItems === "flex-end") k.fx = innerW - k.w;
        else if (k.marginAutoX) k.fx = (innerW - k.w) / 2; // margin: 0 auto
        else { k.fx = k.marginL; if (k.tag !== "svg" && k.style.width === undefined && !k.text) k.w = Math.max(k.w, innerW - k.marginL); }
        if (el.flex && jc === "center") k.fy += (innerH - contentH) / 2;
      }
    }
  }
  // anak mutlak: blok pengandung = kotak ini
  for (const k of flow) {
    if (k.tag === "#text" || !(k.style?.position === "absolute" || k.style?.position === "fixed")) continue;
    measure(ctx, k, el.w, el.h, st, false);
    k.fx = k.L !== undefined ? k.L : k.R !== undefined ? el.w - k.R - k.w : 0;
    k.fy = k.T !== undefined ? k.T : k.B !== undefined ? el.h - k.B - k.h : 0;
    k.absPos = true;
  }
}

// ------------------------------------------------------------------ pancar
const IDENT = [1, 0, 0, 1, 0, 0];
const mul = (m, o) => [m[0] * o[0] + m[2] * o[1], m[1] * o[0] + m[3] * o[1], m[0] * o[2] + m[2] * o[3], m[1] * o[2] + m[3] * o[3], m[0] * o[4] + m[2] * o[5] + m[4], m[1] * o[4] + m[3] * o[5] + m[5]];
function matOf(t, cx, cy) {
  // sama seperti cssTransform tetapi sebagai matriks (untuk keputusan klip)
  let m = [1, 0, 0, 1, cx, cy];
  for (const g of (t || "").matchAll(/([a-zA-Z]+)\(([^)]*)\)/g)) {
    const a = g[2].split(",").map((v) => parseFloat(v));
    const r = (a[0] * Math.PI) / 180;
    const o = g[1] === "translate" ? [1, 0, 0, 1, a[0], a[1] || 0] : g[1] === "translateX" ? [1, 0, 0, 1, a[0], 0] : g[1] === "translateY" ? [1, 0, 0, 1, 0, a[0]]
      : g[1] === "scale" ? [a[0], 0, 0, a.length > 1 ? a[1] : a[0], 0, 0] : g[1] === "scaleX" ? [a[0], 0, 0, 1, 0, 0] : g[1] === "scaleY" ? [1, 0, 0, a[0], 0, 0] : g[1] === "rotate" ? [Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0] : g[1] === "matrix" ? a : IDENT;
    m = mul(m, o);
  }
  return mul(m, [1, 0, 0, 1, -cx, -cy]);
}
/** adakah segi empat (x,y,w,h) di bawah matriks m menutup seluruh bingkai? (tanpa putaran sahaja) */
function coversFrame(ctx, m, x, y, w, h) {
  if (Math.abs(m[1]) > 1e-6 || Math.abs(m[2]) > 1e-6 || m[0] <= 0 || m[3] <= 0) return false;
  const x0 = m[0] * x + m[4], y0 = m[3] * y + m[5], x1 = m[0] * (x + w) + m[4], y1 = m[3] * (y + h) + m[5];
  return x0 <= 0.01 && y0 <= 0.01 && x1 >= ctx.W - 0.01 && y1 >= ctx.H - 0.01;
}
const hasBlend = (el) => {
  if (el._hb !== undefined) return el._hb;
  let v = false;
  for (const k of el.kids || []) {
    if (k.tag === "#text") continue;
    if (k.style?.["mix-blend-mode"] && k.style["mix-blend-mode"] !== "normal") v = true;
    else if (k.tag === "svg" ? /mix-blend-mode/.test(k.inner) : hasBlend(k)) v = true;
    if (v) break;
  }
  return (el._hb = v);
};

/** Kotak dakwat elemen HTML dalam ruang tempatannya: kotak sendiri ∪ limpahan anak (selepas transform anak). */
function inkBox(el, x, y) {
  let box = [x, y, x + el.w, y + el.h];
  if (el.tag === "svg" || el.style.overflow === "hidden" || !el.kids) return box;
  const cx0 = x + (el.bw?.[3] || 0) + (el.pad?.[3] || 0), cy0 = y + (el.bw?.[0] || 0) + (el.pad?.[0] || 0);
  for (const k of el.kids) {
    if (k.tag === "#text" || k.skip || k.fx === undefined) continue;
    const kx = k.absPos ? x + k.fx : cx0 + k.fx, ky = k.absPos ? y + k.fy : cy0 + k.fy;
    let kb = inkBox(k, kx, ky);
    const t = k.style.transform;
    if (t && t !== "none") {
      const o = (k.style["transform-origin"] || "50% 50%").split(/\s+/);
      const mm = matOf(t, kx + (px(o[0], k.w) ?? k.w / 2), ky + (px(o[1] ?? "50%", k.h) ?? k.h / 2));
      const pts = [[kb[0], kb[1]], [kb[2], kb[1]], [kb[0], kb[3]], [kb[2], kb[3]]].map(([px0, py0]) => [mm[0] * px0 + mm[2] * py0 + mm[4], mm[1] * px0 + mm[3] * py0 + mm[5]]);
      kb = [Math.min(...pts.map((q) => q[0])), Math.min(...pts.map((q) => q[1])), Math.max(...pts.map((q) => q[0])), Math.max(...pts.map((q) => q[1]))];
    }
    box = [Math.min(box[0], kb[0]), Math.min(box[1], kb[1]), Math.max(box[2], kb[2]), Math.max(box[3], kb[3])];
  }
  return box;
}

function emit(ctx, el, x, y, m) {
  if (el.skip) return;
  const s = el.style;
  const opacity = s.opacity === undefined ? 1 : parseFloat(s.opacity);
  if (opacity <= 0) return;
  if (el.tag === "nova-canvas") { emitCanvas(ctx, el); return; }
  const w = el.w, h = el.h;
  // --- atribut kumpulan
  let attrs = "";
  let m2 = m;
  if (s.transform && s.transform !== "none") {
    const o = (s["transform-origin"] || "50% 50%").split(/\s+/);
    const cx = x + (px(o[0], w) ?? w / 2), cy = y + (px(o[1] ?? "50%", h) ?? h / 2);
    const t = cssTransform(s.transform, cx, cy);
    if (t) { attrs += ` transform="${t}"`; m2 = mul(m, matOf(s.transform, cx, cy)); }
  }
  if (opacity < 1) attrs += ` opacity="${n(opacity)}"`;
  let filt = cssFilter(s.filter);
  const ref = filt && filt.match(/^url\(["']?#([^"')]+)["']?\)$/);
  if (ref && el.tag !== "svg" && w > 0 && h > 0) {
    const def = ctx.html.match(new RegExp(`<filter\\b([^>]*\\bid="${ref[1]}"[^>]*)>([\\s\\S]*?)</filter>`));
    if (def && !/filterUnits="userSpaceOnUse"/.test(def[1])) {
      const a = parseAttrs(def[1]);
      const id = uid("hf");
      // kotak rujukan = kotak elemen bersama limpahan anak-anaknya (seperti Chrome), bukan bbox kandungan SVG
      const ib = inkBox(el, x, y);
      const bw0 = ib[2] - ib[0], bh0 = ib[3] - ib[1];
      const fx = ib[0] + (px(a.x ?? "-10%", bw0) ?? 0), fy = ib[1] + (px(a.y ?? "-10%", bh0) ?? 0);
      const fw = px(a.width ?? "120%", bw0), fh = px(a.height ?? "120%", bh0);
      const rest = Object.entries(a).filter(([k]) => !["id", "x", "y", "width", "height", "filterUnits"].includes(k)).map(([k, v]) => ` ${k}="${v}"`).join("");
      ctx.out += `<defs><filter id="${id}" filterUnits="userSpaceOnUse" x="${n(fx)}" y="${n(fy)}" width="${n(fw)}" height="${n(fh)}"${rest}>${def[2]}</filter></defs>`;
      filt = `url(#${id})`;
    }
  }
  if (filt) attrs += ` filter="${filt}"`;
  let style = "";
  if (s["mix-blend-mode"] && s["mix-blend-mode"] !== "normal") style += `mix-blend-mode:${s["mix-blend-mode"]};`;
  // CSS: transform mewujudkan konteks susunan (mengasingkan adunan anak); SVG tidak — nyatakan secara eksplisit
  else if (s.transform && s.transform !== "none" && hasBlend(el)) style += "isolation:isolate;";
  if (style) attrs += ` style="${style}"`;
  if (s["clip-path"] && s["clip-path"] !== "none") {
    const shape = clipShape(s["clip-path"], x, y, w, h);
    if (shape) {
      const id = uid("cp");
      ctx.out += `<defs><clipPath id="${id}">${shape}</clipPath></defs>`;
      attrs += ` clip-path="url(#${id})"`;
    } else ctx.warn(`clip-path CSS belum disokong: ${s["clip-path"].slice(0, 60)}`);
  }
  if (attrs) ctx.open(`<g${attrs}>`, /opacity=|filter=|style=|clip-path=/.test(attrs));

  if (el.tag === "svg") emitSvg(ctx, el, x, y, m2);
  else {
    const R = cornerRadii(s, w, h);
    if (s["box-shadow"] && s["box-shadow"] !== "none") {
      for (const sd of shadows(s["box-shadow"]).reverse()) {
        // spread membesarkan kotak bayang (dan jejarinya) ke semua arah
        const sp = sd.spread, sx = x + sd.dx - sp, sy = y + sd.dy - sp, sw = w + sp * 2, sh = h + sp * 2;
        if (sw <= 0 || sh <= 0) continue;
        const SR = R && sp ? R.map((r) => (r[0] > 0 && r[1] > 0 ? [Math.max(0, r[0] + sp), Math.max(0, r[1] + sp)] : r)) : R;
        if (sd.blur > 0) {
          const id = uid("bs"), padB = sd.blur * 1.5 + 4;
          ctx.out += `<defs><filter id="${id}" filterUnits="userSpaceOnUse" x="${n(sx - padB)}" y="${n(sy - padB)}" width="${n(sw + padB * 2)}" height="${n(sh + padB * 2)}" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="${n(sd.blur / 2)}"/></filter></defs>`;
          ctx.out += boxShape(sx, sy, sw, sh, SR, `${paint(sd.c)} filter="url(#${id})"`);
        } else ctx.out += boxShape(sx, sy, sw, sh, SR, paint(sd.c));
      }
    }
    ctx.out += background(s.background || s["background-color"], x, y, w, h, R, ctx.warn);
    if (el.border && el.border.w > 0) {
      const bw = el.border.w;
      ctx.out += boxShape(x + bw / 2, y + bw / 2, w - bw, h - bw, R, `fill="none" ${paint(el.border.c, "stroke")} stroke-width="${n(bw)}"`, bw / 2);
    }
    if (el.borderLeft) {
      const id = uid("bl");
      ctx.out += `<defs><clipPath id="${id}">${boxShape(x, y, w, h, R)}</clipPath></defs><rect x="${n(x)}" y="${n(y)}" width="${n(el.borderLeft.w)}" height="${n(h)}" ${paint(el.borderLeft.c)} clip-path="url(#${id})"/>`;
    }
    const cx0 = x + el.bw[3] + el.pad[3], cy0 = y + el.bw[0] + el.pad[0];
    let clipped = false;
    if (s.overflow === "hidden" && w > 0 && h > 0 && !coversFrame(ctx, m2, x, y, w, h)) {
      const id = uid("ov");
      ctx.out += `<defs><clipPath id="${id}">${boxShape(x, y, w, h, R)}</clipPath></defs>`;
      ctx.open(`<g clip-path="url(#${id})">`, true);
      clipped = true;
    }
    if (el.text) ctx.out += emitText(ctx, el.text, el.st, cx0, cy0, w - el.bw[1] - el.bw[3] - el.pad[1] - el.pad[3]);
    for (const k of el.kids) {
      if (k.tag === "#text") continue;
      if (k.absPos) emit(ctx, k, x + k.fx, y + k.fy, m2);
      else if (k.fx !== undefined) emit(ctx, k, cx0 + k.fx, cy0 + k.fy, m2);
    }
    if (clipped) ctx.close();
  }
  if (attrs) ctx.close();
}

function emitSvg(ctx, el, x, y, m) {
  const a = el.attrs;
  if (!el.inner) return;
  if (el.w <= 0 || el.h <= 0) { ctx.out += `<defs>${el.inner}</defs>`; return; } // pemegang <defs> (Chrome tidak melukis svg 0×0)
  let t = "", vb = null, sc = 1, tx = x, ty = y;
  if (a.viewBox) {
    vb = a.viewBox.split(/[\s,]+/).map(parseFloat);
    sc = Math.min(el.w / vb[2], el.h / vb[3]);
    tx = x + (el.w - vb[2] * sc) / 2 - vb[0] * sc;
    ty = y + (el.h - vb[3] * sc) / 2 - vb[1] * sc;
  }
  if (tx || ty) t += `translate(${n(tx)} ${n(ty)})`;
  if (sc !== 1) t += ` scale(${sc})`;
  const visible = el.style.overflow === "visible";
  let clip = "";
  if (!visible && !coversFrame(ctx, m, x, y, el.w, el.h)) {
    const id = uid("sv");
    ctx.out += `<defs><clipPath id="${id}"><rect x="${n(x)}" y="${n(y)}" width="${n(el.w)}" height="${n(el.h)}"/></clipPath></defs>`;
    clip = ` clip-path="url(#${id})"`;
  }
  if (clip) ctx.out += `<g${clip}>`;
  ctx.out += t ? `<g transform="${t.trim()}">` : "<g>";
  ctx.out += el.inner;
  ctx.out += "</g>";
  if (clip) ctx.out += "</g>";
}

function emitCanvas(ctx, el) {
  // <nova-canvas><nova-fill><nova-shader data-props='{"source":{"sksl":…},"uniforms":{…}}'/></nova-fill></nova-canvas>
  const walk = (node) => {
    for (const k of node.kids || []) {
      if (k.tag === "nova-shader") {
        const p = JSON.parse(k.attrs["data-props"] || "{}");
        if (p.source?.sksl) ctx.shader(p.source.sksl, p.uniforms || {});
      } else if (k.tag === "nova-path" || k.tag === "nova-blurmask") {
        ctx.warn(`elemen Skia <${k.tag}> belum disokong`);
      } else walk(k);
    }
  };
  walk(el);
}

/** nyahpendua id: Chrome menggunakan takrif PERTAMA bagi id berulang; namakan semula yang kemudian */
function dedupeIds(svg) {
  const seen = new Set();
  let dup = false;
  const out = svg.replace(/\sid="([^"]+)"/g, (m, id) => {
    if (!seen.has(id)) { seen.add(id); return m; }
    dup = true;
    return ` id="${id}__d${uid("")}"`;
  });
  return dup ? out : svg;
}

/**
 * @param html   markup satu frame
 * @param ctx0   { W, H, fonts: {pick(family, weight)} , warn(msg) }
 * @returns      senarai operasi lukis
 */
export function convert(html, ctx0) {
  uidN = 0;
  const { W, H } = ctx0;
  const ops = [];
  const head = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">`;
  const ctx = {
    W, H, html, fonts: ctx0.fonts, warn: ctx0.warn || (() => {}),
    out: "", stack: [],
    open(tagStr, effect) { this.stack.push({ tagStr, effect }); this.out += tagStr; },
    close() { this.stack.pop(); this.out += "</g>"; },
    flush() {
      if (this.out.length > this.stack.reduce((a, s) => a + s.tagStr.length, 0)) {
        ops.push({ t: "svg", d: dedupeIds(head + this.out + "</g>".repeat(this.stack.length) + "</svg>") });
      }
      this.out = this.stack.map((s) => s.tagStr).join("");
    },
    shader(sksl, uniforms) {
      if (this.stack.some((s) => s.effect)) this.warn("shader di dalam kumpulan berkesan (opacity/filter/klip): kesan itu tidak dikenakan pada shader");
      this.flush();
      ops.push({ t: "shader", sksl, uniforms });
    },
  };
  const root = parse(html);
  const base = { color: "#000", "font-size": "16px", "font-family": "sans-serif", "font-weight": "400" };
  for (const k of root.kids) {
    if (k.tag === "#text") continue;
    measure(ctx, k, W, H, base, false);
    emit(ctx, k, k.L || 0, k.T || 0, IDENT);
  }
  ctx.flush();
  return ops;
}
