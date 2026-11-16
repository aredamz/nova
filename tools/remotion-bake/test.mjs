// Ujian pembakar: `node --test tools/remotion-bake/test.mjs` (perlukan ffmpeg dan fontconfig).
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { convert } from "./convert.mjs";
import { loadFonts } from "./fonts.mjs";
import { audioCues } from "./audio.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const fonts = loadFonts(path.join(HERE, "tiada-folder")); // fon sistem sahaja
// Aturan fon sistem meniru Chrome di Linux (Arial -> Liberation Sans); di sistem lain fon yang dipasang berbeza.
const LINUX_FONTS = { skip: process.platform !== "linux" && "aturan fon sistem diuji di Linux sahaja" };
const svgOf = (html, warnings = []) => {
  const ops = convert(`<div style="position:absolute;top:0;left:0;width:1080px;height:1920px">${html}</div>`, { W: 1080, H: 1920, fonts, warn: (m) => warnings.push(m) });
  return ops.filter((o) => o.t === "svg").map((o) => o.d).join("");
};
const box = (style, inner = "") => `<div style="position:absolute;left:100px;top:200px;width:400px;height:300px;background:#f00;${style}">${inner}</div>`;

test("clip-path: inset dengan round", () => {
  const svg = svgOf(`<div style="position:absolute;inset:0;clip-path:inset(640px 70px 840px 70px round 110px)">${box("")}</div>`);
  assert.match(svg, /<clipPath id="[^"]+"><rect x="70" y="640" width="940" height="440" rx="110"\/>/);
});

test("clip-path: path, circle, ellipse, polygon", () => {
  assert.match(svgOf(box(`clip-path:path(evenodd, &quot;M0 0 L10 0 L10 10 Z&quot;)`)), /<path d="M0 0 L10 0 L10 10 Z" clip-rule="evenodd" transform="translate\(100 200\)"\/>/);
  assert.match(svgOf(box("clip-path:circle(50px at 100px 60px)")), /<circle cx="200" cy="260" r="50"\/>/);
  assert.match(svgOf(box("clip-path:ellipse(40% 50% at center)")), /<ellipse cx="300" cy="350" rx="160" ry="150"\/>/);
  assert.match(svgOf(box("clip-path:polygon(0 0, 100% 0, 50% 100%)")), /<polygon points="100,200 500,200 300,500"\/>/);
});

test("clip-path yang tidak dikenali memberi amaran", () => {
  const w = [];
  svgOf(box("clip-path:url(#x)"), w);
  assert.ok(w.some((m) => m.includes("clip-path")));
});

test("repeating-linear-gradient menjadi gradien berulang sepanjang satu kitaran", () => {
  const svg = svgOf(box("background:repeating-linear-gradient(180deg, rgba(0,0,0,0.5) 0px, rgba(0,0,0,0) 4px)"));
  const m = svg.match(/<linearGradient [^>]*spreadMethod="repeat" x1="([\d.]+)" y1="([\d.]+)" x2="([\d.]+)" y2="([\d.]+)"/);
  assert.ok(m, svg);
  assert.equal(+m[4] - +m[2], 4); // satu kitaran = 4 px ke bawah
  assert.equal(+m[2], 200);
});

test("jejari sudut: peratus, eliptik dan setiap sudut", () => {
  assert.match(svgOf(`<div style="position:absolute;left:0;top:0;width:100px;height:100px;background:#fff;border-radius:50%"></div>`), /<rect x="0" y="0" width="100" height="100" rx="50" ry="50"/);
  const svg = svgOf(`<div style="position:absolute;left:0;top:0;width:200px;height:100px;background:#000;border-bottom-left-radius:50% 30%;border-bottom-right-radius:50% 30%"></div>`);
  assert.match(svg, /<path d="M0 0L200 0L200 0L200 70A100 30 0 0 1 100 100L100 100A100 30 0 0 1 0 70L0 0L0 0Z"/);
});

test("teks bebas di sebelah elemen blok tidak hilang; margin auto memusatkan", () => {
  const svg = svgOf(`<div style="position:absolute;left:0;top:415px;width:430px;text-align:center;color:#fff;font-family:Arial, sans-serif;font-size:24px">` +
    `<svg width="46" height="46" viewBox="-23 -23 46 46" style="display:block;margin:0 auto 10px"><circle cx="0" cy="0" r="18"></circle></svg>Menyambung semula</div>`);
  assert.match(svg, /Menyambung/);
  assert.match(svg, /<rect x="192" y="415" width="46" height="46"\/>/); // ikon di tengah: (430 - 46) / 2
  const y = +svg.match(/<text[^>]* y="([\d.]+)"/)[1];
  assert.ok(y > 415 + 46 + 10, `teks di bawah ikon + margin (y=${y})`);
});

test("sifat CSS yang tidak difahami memberi amaran", () => {
  const w = [];
  svgOf(box("z-index:3;pointer-events:none"), w);
  assert.deepEqual(w, ["sifat CSS belum disokong: z-index"]);
});

test("audioCues: larian berterusan digabung, putus pada sela dan pada lompatan masa", () => {
  const rec = (f, t, v, k = "a") => ({ k, src: "public/a.mp3", f, t, r: 1, v });
  const recs = [rec(10, 0, 0.5), rec(11, 1 / 30, 0.6), rec(12, 2 / 30, 0.7), rec(20, 10 / 30, 1), rec(21, 0.9, 1), { ...rec(5, 0, 1, "b"), l: 1, pp: 1, r: 1.18 }];
  const cues = audioCues(recs, 30, 0);
  assert.deepEqual(cues.map((c) => [c.frame, c.volumes.length]), [[5, 1], [10, 3], [20, 1], [21, 1]]);
  assert.deepEqual(cues[1].volumes, [0.5, 0.6, 0.7]);
  assert.equal(cues[0].loop, true);
  assert.equal(cues[0].preserve_pitch, true);
  assert.equal(cues[0].rate, 1.18);
});

test("mix.mjs: kedudukan, volum per frame, mono aras penuh, gelung", { skip: spawnSync("ffmpeg", ["-version"]).status !== 0 }, () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "nova-mix-"));
  try {
    // 0.5 s nada mono amplitud 0.5
    const gen = spawnSync("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "aevalsrc=0.5*sin(2*PI*440*t):s=48000:d=0.5", "-ac", "1", "-c:a", "pcm_f32le", path.join(dir, "nada.wav")]);
    assert.equal(gen.status, 0);
    const cues = [
      { src: "nada.wav", frame: 30, t: 0, rate: 1, volumes: new Array(15).fill(1) }, // 1.0–1.5 s
      { src: "nada.wav", frame: 60, t: 0, rate: 1, volumes: new Array(15).fill(0.5) }, // 2.0–2.5 s, separuh
      { src: "nada.wav", frame: 90, t: 0, rate: 1, loop: true, volumes: new Array(30).fill(1) }, // 3.0–4.0 s, dua gelung
    ];
    fs.writeFileSync(path.join(dir, "audio.json"), JSON.stringify({ nova_audio: 1, fps: 30, frames: 150, cues }));
    const r = spawnSync("node", [path.join(HERE, "mix.mjs"), path.join(dir, "audio.json"), "--project", dir, "--out", path.join(dir, "mix.wav")], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
    const wav = fs.readFileSync(path.join(dir, "mix.wav"));
    const pcm = new Float32Array(wav.buffer, wav.byteOffset + 44, (wav.length - 44) / 4);
    assert.equal(pcm.length, 5 * 48000 * 2);
    const peak = (a, b) => { let p = 0; for (let i = Math.round(a * 48000) * 2; i < Math.round(b * 48000) * 2; i++) p = Math.max(p, Math.abs(pcm[i])); return p; };
    assert.equal(peak(0, 0.99), 0);
    assert.ok(Math.abs(peak(1.05, 1.45) - 0.5) < 0.01, `mono aras penuh: ${peak(1.05, 1.45)}`);
    assert.equal(peak(1.51, 1.99), 0);
    assert.ok(Math.abs(peak(2.05, 2.45) - 0.25) < 0.01);
    assert.ok(Math.abs(peak(3.55, 3.95) - 0.5) < 0.01, "gelung kedua berbunyi");
    assert.equal(peak(4.01, 5), 0);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});

test("box-shadow dengan spread melukis gelang di sekeliling kotak", () => {
  const svg = svgOf(`<div style="position:absolute;left:300px;top:100px;width:180px;height:320px;border-radius:14px;box-shadow:0 0 0 6px rgba(255,255,255,0.9);background:#000"></div>`);
  assert.match(svg, /<rect x="294" y="94" width="192" height="332" rx="20" ry="20" fill="rgb\(255,255,255\)" fill-opacity="0.9"\/><rect x="300" y="100" width="180" height="320" rx="14" ry="14"/);
});

test("aksara yang tiada dalam fon diukur dengan fon sandaran", LINUX_FONTS, () => {
  const font = fonts.pick("Arial, sans-serif", 400);
  const plain = fonts.advance(font, "REC", 100);
  assert.equal(plain, font.getAdvanceWidth("REC", 100));
  // U+2713 tiada dalam Liberation Sans: lebarnya datang daripada fon sandaran, bukan .notdef
  if (font.charToGlyphIndex("✓") === 0) assert.notEqual(fonts.advance(font, "✓", 100), font.getAdvanceWidth("✓", 100));
});

test("hanya aksara yang tiada bertukar fon; selebihnya kekal", LINUX_FONTS, () => {
  const svg = svgOf(`<div style="position:absolute;left:10px;top:6px;font-family:Arial, sans-serif;font-size:40px;color:#f00">✓ REC</div>`);
  const spans = [...svg.matchAll(/<tspan x="([\d.]+)" font-family="([^"]+)"[^>]*>([^<]*)<\/tspan>/g)].map((m) => [m[2], m[3], +m[1]]);
  assert.equal(spans.length, 2, svg);
  assert.notEqual(spans[0][0], "Liberation Sans");
  assert.equal(spans[0][1], "✓");
  assert.deepEqual(spans[1].slice(0, 2), ["Liberation Sans", " REC"]);
  assert.ok(spans[1][2] > spans[0][2]);
});

test("gradien separa lutsinar mengikut interpolasi pra-darab CSS", () => {
  const svg = svgOf(box("background:linear-gradient(90deg, rgba(160,200,255,0.1), rgba(0,0,0,0.5))"));
  const stops = [...svg.matchAll(/<stop offset="([\d.]+)" stop-color="rgb\(([\d.]+),([\d.]+),([\d.]+)\)" stop-opacity="([\d.]+)"/g)].map((m) => m.slice(1).map(Number));
  const mid = stops.find((st) => st[0] === 0.5);
  assert.ok(mid, svg);
  // pra-darab di tengah: (160*0.1/2) / 0.3 = 26.7, bukan 80 seperti interpolasi lurus
  assert.ok(Math.abs(mid[1] - 26.667) < 0.01 && Math.abs(mid[4] - 0.3) < 1e-9, String(mid));
  // alfa sama atau warna sama: tiada hentian tambahan
  assert.equal((svgOf(box("background:linear-gradient(#000, #fff)")).match(/<stop /g) || []).length, 2);
});
