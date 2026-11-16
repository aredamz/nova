#!/usr/bin/env node
// Campur kiu audio hasil bake (audio.json) menjadi satu trek WAV — pengganti campuran audio Remotion.
//
//   node mix.mjs <pek>/audio.json --project <akar projek> --out mix.wav [--lufs -14] [--report mix.json]
//
// Setiap kiu = satu <Audio> yang berbunyi berterusan: fail, frame mula, kedudukan dalam fail, kadar main,
// gelung/trim, dan volum setiap frame. Fail dinyahkod sekali oleh ffmpeg (48 kHz stereo float; kadar main
// digunakan di situ: `atempo` bila nada dikekalkan, pensampelan semula bila tidak), kemudian sampel
// dijumlahkan di sini. Volum berubah per frame seperti Remotion, dengan tanjakan 5 ms supaya tiada klik.
//
// --lufs N: master akhir (loudnorm satu laluan + pengehad), puncak disemak SELEPAS AAC dan sasaran
// diturunkan jika puncak melebihi -1 dBFS — rantai yang sama dengan skrip render Remotion projek asal.
import fs from "node:fs";
import path from "node:path";
import { execFile, spawn, spawnSync } from "node:child_process";

const argv = process.argv.slice(2);
const opt = (name, def) => { const i = argv.indexOf("--" + name); return i < 0 ? def : argv[i + 1]; };
if (!argv[0] || argv[0].startsWith("--") || !opt("out")) {
  console.error("guna: node mix.mjs <pek>/audio.json --project <akar projek> --out mix.wav [--lufs -14] [--report mix.json]");
  process.exit(2);
}
const cueFile = path.resolve(argv[0]);
const project = path.resolve(opt("project", process.cwd()));
const outFile = path.resolve(opt("out"));
const lufs = opt("lufs") === undefined ? null : parseFloat(opt("lufs"));
const SR = 48000, CH = 2, RAMP = 240;

const doc = JSON.parse(fs.readFileSync(cueFile, "utf8"));
if (doc.nova_audio !== 1) throw new Error("bukan fail kiu audio NOVA (nova_audio: 1)");
const { fps, frames, cues } = doc;
const sampleAt = (frame) => Math.round((frame * SR) / fps);
const total = sampleAt(frames);
const mix = new Float32Array(total * CH);

// atempo menerima 0.5–100 setiap satu; rantai untuk nilai di luar julat
const atempo = (x) => {
  const parts = [];
  while (x < 0.5) { parts.push("atempo=0.5"); x /= 0.5; }
  while (x > 100) { parts.push("atempo=100"); x /= 100; }
  if (Math.abs(x - 1) > 1e-9) parts.push(`atempo=${x}`);
  return parts;
};
// Satu varian = (fail, kadar, nada). Hasil nyahkod sudah dalam "masa garis masa": 1 s sampel = 1 s video.
const variantKey = (c) => `${c.src}|${c.rate}|${c.preserve_pitch ? 1 : 0}|${c.tone ?? 1}`;
function decode(c) {
  const file = path.resolve(project, c.src);
  if (!fs.existsSync(file)) throw new Error(`fail audio tiada: ${c.src}`);
  const pitch = (c.tone ?? 1) * (c.preserve_pitch ? 1 : c.rate); // faktor nada
  const filters = [`aresample=${SR}`];
  if (Math.abs(pitch - 1) > 1e-9) filters.push(`asetrate=${SR * pitch}`, `aresample=${SR}`); // tempo & nada × pitch
  filters.push(...atempo(c.rate / pitch)); // baki tempo, nada kekal
  // Mono disalin penuh ke kiri & kanan (seperti Remotion/Web Audio); penukaran lalai ffmpeg menurunkannya 3 dB.
  const channels = new Promise((resolve) => execFile("ffprobe", ["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=channels", "-of", "csv=p=0", file], (_e, out) => resolve(parseInt(out, 10))));
  return channels.then((ch) => new Promise((resolve, reject) => {
    if (ch === 1) filters.push("pan=stereo|c0=c0|c1=c0");
    const p = spawn("ffmpeg", ["-v", "error", "-i", file, "-vn", "-af", filters.join(","), "-ac", String(CH), "-ar", String(SR), "-f", "f32le", "pipe:1"],
      { stdio: ["ignore", "pipe", "inherit"] });
    const chunks = [];
    p.stdout.on("data", (d) => chunks.push(d));
    p.on("error", reject);
    p.on("close", (code) => {
      if (code) return reject(new Error(`ffmpeg gagal menyahkod ${c.src}`));
      const b = Buffer.concat(chunks);
      resolve(new Float32Array(b.buffer, b.byteOffset, Math.floor(b.length / 4)));
    });
  }));
}

const variants = new Map();
for (const c of cues) if (!variants.has(variantKey(c))) variants.set(variantKey(c), c);
const pcm = new Map();
{
  // nyahkod selari (terhad), kerana episod biasa ada 100+ fail pendek
  const todo = [...variants.entries()];
  const run = async () => { for (let e; (e = todo.pop()); ) pcm.set(e[0], await decode(e[1])); };
  await Promise.all(Array.from({ length: 8 }, run));
}

for (const c of cues) {
  const data = pcm.get(variantKey(c));
  const len = data.length / CH; // sampel (masa garis masa)
  const scale = SR / fps / c.rate; // frame sumber -> sampel hasil nyahkod
  const trimStart = (c.trim_before ?? 0) * scale;
  const end = c.trim_after !== undefined ? c.trim_after * scale : len;
  const span = end - trimStart; // panjang satu gelung / panjang boleh main
  if (!(span > 0)) continue;
  const u0 = c.t * SR; // kedudukan tanpa gelung pada sampel pertama kiu
  const first = sampleAt(c.frame);
  let prev = c.volumes[0];
  for (let i = 0; i < c.volumes.length; i++) {
    const a = sampleAt(c.frame + i), b = Math.min(total, sampleAt(c.frame + i + 1));
    const v = c.volumes[i];
    for (let s = a; s < b; s++) {
      let u = u0 + (s - first);
      if (c.loop) u %= span; else if (u >= span) break;
      const idx = Math.floor(u + trimStart);
      if (idx >= len) { if (c.loop) continue; break; }
      const k = s - a;
      const g = k < RAMP ? prev + ((v - prev) * (k + 1)) / RAMP : v;
      mix[s * CH] += data[idx * CH] * g;
      mix[s * CH + 1] += data[idx * CH + 1] * g;
    }
    prev = v;
  }
}

function writeWav(file, samples) {
  const head = Buffer.alloc(44);
  const bytes = samples.length * 4;
  head.write("RIFF", 0); head.writeUInt32LE(36 + bytes, 4); head.write("WAVEfmt ", 8);
  head.writeUInt32LE(16, 16); head.writeUInt16LE(3, 20); head.writeUInt16LE(CH, 22); // 3 = IEEE float
  head.writeUInt32LE(SR, 24); head.writeUInt32LE(SR * CH * 4, 28); head.writeUInt16LE(CH * 4, 32); head.writeUInt16LE(32, 34);
  head.write("data", 36); head.writeUInt32LE(bytes, 40);
  fs.writeFileSync(file, Buffer.concat([head, Buffer.from(samples.buffer, samples.byteOffset, bytes)]));
}
const ff = (args) => {
  const r = spawnSync("ffmpeg", ["-hide_banner", "-nostats", ...args], { encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.status) throw new Error("ffmpeg gagal: " + r.stderr.slice(-400));
  return r.stderr;
};
// kelantangan bersepadu & puncak sebenar (EBU R128)
const measure = (file) => {
  const log = ff(["-i", file, "-af", "ebur128=peak=true", "-f", "null", "-"]);
  const tail = log.slice(log.lastIndexOf("Summary:"));
  return { lufs: parseFloat(/I:\s+(-?[\d.]+|-inf)/.exec(tail)?.[1] ?? "NaN"), peak: parseFloat(/Peak:\s+(-?[\d.]+|-inf)/.exec(tail)?.[1] ?? "NaN") };
};

const report = { cues: cues.length, files: new Set(cues.map((c) => c.src)).size, seconds: total / SR };
if (lufs === null) {
  writeWav(outFile, mix);
} else {
  const raw = outFile + ".raw.wav", probe = outFile + ".probe.m4a";
  writeWav(raw, mix);
  const before = measure(raw);
  report.raw = before;
  if (!Number.isFinite(before.lufs)) {
    fs.renameSync(raw, outFile); // senyap: tiada apa untuk dinormalkan
  } else {
    let target = lufs + 1.3; // pengehad di bawah mengambil ±1.3 LU
    for (let attempt = 1; ; attempt++) {
      ff(["-y", "-i", raw, "-af", `loudnorm=I=${target.toFixed(1)}:TP=-1.5:LRA=11,alimiter=limit=0.7:attack=1:release=50:level=false`, "-ar", String(SR), "-c:a", "pcm_s24le", outFile]);
      ff(["-y", "-i", outFile, "-c:a", "aac", "-b:a", "192k", probe]); // puncak diukur selepas AAC, seperti fail akhir
      report.master = { ...measure(probe), loudnorm_target: +target.toFixed(1), attempts: attempt };
      if (report.master.peak <= -1.0 || attempt === 3) break;
      target -= 0.7;
    }
    fs.rmSync(raw); fs.rmSync(probe);
  }
}
if (opt("report")) fs.writeFileSync(path.resolve(opt("report")), JSON.stringify(report, null, 1));
const m = report.master;
console.error(`mix: ${report.cues} kiu, ${report.files} fail, ${report.seconds.toFixed(1)} s` + (m ? ` — ${m.lufs} LUFS, puncak ${m.peak} dBFS` : ""));
