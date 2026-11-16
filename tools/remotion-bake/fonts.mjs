// Fon untuk ukuran teks (opentype.js). Fail: fonts/<Keluarga>-<berat>.ttf
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const opentype = createRequire(import.meta.url)("opentype.js");

export function loadFonts(dir) {
  const faces = [];
  for (const f of (fs.existsSync(dir) ? fs.readdirSync(dir) : []).filter((f) => /\.(ttf|otf)$/i.test(f)).sort()) {
    const buf = fs.readFileSync(path.join(dir, f));
    const font = opentype.parse(buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength));
    const family = font.names.preferredFamily?.en || font.names.fontFamily?.en || f;
    const weight = font.tables.os2?.usWeightClass || 400;
    font.__family = family; font.__weight = weight; font.__key = family + "|" + weight; font.__file = f;
    faces.push(font);
  }
  // Keluarga yang tiada dalam folder fon dicari pada sistem melalui fontconfig, dengan aturan Chrome:
  // nama generik -> fon lalai Chrome (Arial / Times New Roman / Monospace), dan nama khusus hanya
  // diterima jika fontconfig memulangkan keluarga itu sendiri atau alias metrik-serasinya
  // (Arial -> Liberation Sans); pengganti rambang ditolak dan nama seterusnya dicuba.
  const GENERIC = { "sans-serif": "Arial", "system-ui": "Arial", "ui-sans-serif": "Arial", serif: "Times New Roman", "ui-serif": "Times New Roman", monospace: "monospace", "ui-monospace": "monospace" };
  const METRIC = { arial: "liberation sans", helvetica: "liberation sans", "times new roman": "liberation serif", times: "liberation serif", "courier new": "liberation mono", courier: "liberation mono" };
  const FC_WEIGHT = { 100: 0, 200: 40, 300: 50, 400: 80, 500: 100, 600: 180, 700: 200, 800: 205, 900: 210 };
  const system = new Map();
  const fromSystem = (name, weight) => {
    const generic = GENERIC[name.toLowerCase()];
    const key = (generic ?? name) + "|" + weight;
    if (system.has(key)) return system.get(key);
    let font = null;
    const fcw = FC_WEIGHT[Math.min(900, Math.max(100, Math.round(weight / 100) * 100))];
    const r = spawnSync("fc-match", ["-f", "%{family}|%{file}", `${generic ?? name}:weight=${fcw}`], { encoding: "utf8" });
    const [fams, file] = (r.stdout || "").split("|");
    const got = (fams || "").toLowerCase().split(",");
    const want = (generic ?? name).toLowerCase();
    if (file && (generic || got.includes(want) || got.includes(METRIC[want])) && /\.(ttf|otf)$/i.test(file)) {
      try {
        const buf = fs.readFileSync(file);
        font = opentype.parse(buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength));
        font.__family = font.names.preferredFamily?.en || font.names.fontFamily?.en || name;
        font.__weight = font.tables.os2?.usWeightClass || 400;
        font.__key = font.__family + "|" + font.__weight; font.__file = file; font.__system = true;
      } catch { font = null; }
    }
    system.set(key, font);
    return font;
  };
  const cache = new Map();
  const nearest = (pool, weight) => pool.reduce((a, b) => (Math.abs(b.__weight - weight) < Math.abs(a.__weight - weight) ? b : a));
  // Aksara yang tiada dalam fon: Chrome mencuba fon piawainya (Times New Roman -> Liberation Serif)
  // dahulu, kemudian carian fontconfig mengikut aksara. Penukar memecah teks kepada larian per fon
  // (`split`) supaya hanya aksara itu yang bertukar fon, seperti pelayar.
  const fallback = new Map();
  const fallbackFor = (font, ch) => {
    const key = font.__family + "|" + ch;
    if (fallback.has(key)) return fallback.get(key);
    let hit = null;
    const standard = fromSystem("serif", font.__weight);
    if (standard && standard.charToGlyphIndex(ch) > 0) hit = standard;
    else {
      const r = spawnSync("fc-match", ["-f", "%{file}", `${font.__family}:charset=${ch.codePointAt(0).toString(16)}`], { encoding: "utf8" });
      if (r.stdout && /\.(ttf|otf)$/i.test(r.stdout)) {
        try {
          const buf = fs.readFileSync(r.stdout);
          const f = opentype.parse(buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength));
          if (f.charToGlyphIndex(ch) > 0) {
            hit = f;
            f.__family = f.names.preferredFamily?.en || f.names.fontFamily?.en || font.__family;
            f.__weight = f.tables.os2?.usWeightClass || 400;
          }
        } catch { /* fon yang tidak dapat dibaca: guna lebar .notdef */ }
      }
    }
    fallback.set(key, hit);
    return hit;
  };
  return {
    faces,
    /** pecah `text` kepada larian [{ f, text }]: `font` sendiri, atau fon sandaran bagi aksara yang tiada */
    split(font, text) {
      let plain = true;
      for (const ch of text) if (ch !== " " && font.charToGlyphIndex(ch) === 0) { plain = false; break; }
      if (plain) return [{ f: font, text }];
      const out = [];
      for (const ch of text) {
        const f = (ch !== " " && font.charToGlyphIndex(ch) === 0 && fallbackFor(font, ch)) || font;
        const last = out[out.length - 1];
        if (last && last.f === f) last.text += ch; else out.push({ f, text: ch });
      }
      return out;
    },
    /** lebar `text` pada `size` px; aksara yang tiada dalam `font` diukur dengan fon sandarannya */
    advance(font, text, size) {
      return this.split(font, text).reduce((w, r) => w + r.f.getAdvanceWidth(r.text, size), 0);
    },
    /** `family` = senarai CSS ("Arial, sans-serif"); pulang muka pertama yang wujud */
    pick(family, weight) {
      const key = family + "|" + weight;
      let hit = cache.get(key);
      if (!hit) {
        for (const name of family.split(",").map((v) => v.replace(/["']/g, "").trim()).filter(Boolean)) {
          const fam = faces.filter((f) => f.__family.toLowerCase() === name.toLowerCase());
          hit = fam.length ? nearest(fam, weight) : fromSystem(name, weight);
          if (hit) break;
        }
        hit ??= faces.length ? nearest(faces, weight) : fromSystem("sans-serif", weight);
        if (!hit) throw new Error(`tiada fon untuk "${family}": letak TTF dalam ${dir} atau pasang fon sistem`);
        cache.set(key, hit);
      }
      return hit;
    },
  };
}
