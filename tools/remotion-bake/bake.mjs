#!/usr/bin/env node
// Bakar komposisi Remotion kepada pek frame NOVA — tanpa pelayar.
//
//   node build.mjs --project <projek> --target target.ts --out bundle.cjs
//   node bake.mjs bundle.cjs --out <folder pek> --fonts <folder ttf> [--project <projek>]
//                            [--frames 0-7504] [--workers 12]
//   node bake.mjs bundle.cjs --fonts <folder ttf> --dump 800 > f800.svg     (satu frame, untuk semakan)
//
// Hasil: <out>/frames.json (manifes) + <out>/frames-NN.bin (blob frame dimampat zlib)
//        + <out>/audio.json (kiu audio: setiap <Audio> yang berbunyi; campur dengan mix.mjs).
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import crypto from "node:crypto";
import os from "node:os";
import { fork, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { convert } from "./convert.mjs";
import { loadFonts } from "./fonts.mjs";
import { audioCues } from "./audio.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const opt = (name, def) => { const i = argv.indexOf("--" + name); return i < 0 ? def : argv[i + 1]; };
const bundlePath = path.resolve(argv[0]);
const fontsDir = path.resolve(opt("fonts", path.join(HERE, "fonts")));
const projectRoot = path.resolve(opt("project", process.cwd()));

function setup() {
  const bundle = createRequire(import.meta.url)(bundlePath);
  // panjang fail media (untuk keluk volum gelung "repeat"); dipanggil jarang, jadi ffprobe segerak memadai
  const seconds = new Map();
  globalThis.__novaMediaSeconds = (src) => {
    if (!seconds.has(src)) {
      const r = spawnSync("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path.resolve(projectRoot, src)], { encoding: "utf8" });
      seconds.set(src, parseFloat(r.stdout) || Infinity);
    }
    return seconds.get(src);
  };
  const fonts = loadFonts(fontsDir);
  const warned = new Set();
  const ctx = { W: bundle.meta.width, H: bundle.meta.height, fonts, warn: (m) => { if (!warned.has(m)) { warned.add(m); console.error("amaran:", m); } } };
  return { bundle, ctx, frame: (f) => convert(bundle.renderFrame(f), ctx) };
}

const shaderKey = (sksl) => crypto.createHash("sha1").update(sksl).digest("hex").slice(0, 16);

function encode(ops, shaders, files) {
  const parts = [];
  for (const op of ops) {
    let kind, payload;
    if (op.t === "svg") {
      kind = 1; payload = Buffer.from(op.d, "utf8");
      // fail statik yang dirujuk (staticFile -> "public/…") disalin ke dalam pek
      if (files && op.d.includes('href="public/')) for (const m of op.d.matchAll(/href="(public\/[^"]+)"/g)) files.add(m[1]);
    }
    else { const key = shaderKey(op.sksl); shaders[key] = op.sksl; kind = 2; payload = Buffer.from(JSON.stringify({ shader: key, uniforms: op.uniforms }), "utf8"); }
    const head = Buffer.alloc(5);
    head.writeUInt8(kind, 0); head.writeUInt32LE(payload.length, 1);
    parts.push(head, payload);
  }
  return Buffer.concat(parts);
}

if (opt("dump") !== undefined) {
  const { frame } = setup();
  const ops = frame(parseInt(opt("dump"), 10));
  const which = parseInt(opt("op", "0"), 10);
  console.error(ops.map((o, i) => `${i}:${o.t}:${o.t === "svg" ? o.d.length : JSON.stringify(o.uniforms)}`).join("  "));
  process.stdout.write(ops.filter((o) => o.t === "svg")[which]?.d ?? "");
} else if (opt("worker") !== undefined) {
  // anak: minta julat frame daripada induk sehingga habis, tulis semuanya ke satu serpihan
  const fd = fs.openSync(opt("shard"), "w");
  const { frame, bundle } = setup();
  const shaders = {}, seen = new Map(), files = new Set();
  let off = 0;
  process.on("message", (job) => {
    if (job.end) {
      fs.closeSync(fd);
      process.send({ final: true, shaders, files: [...files] });
      process.disconnect();
      return;
    }
    const index = [], audio = [];
    for (let f = job.a; f < job.b; f++) {
      const raw = encode(frame(f), shaders, files);
      for (const r of bundle.takeAudio()) audio.push(r);
      const h = crypto.createHash("sha1").update(raw).digest("base64");
      let e = seen.get(h);
      if (!e) {
        const z = zlib.deflateSync(raw, { level: 3 });
        fs.writeSync(fd, z);
        e = [off, z.length, raw.length, f, h];
        seen.set(h, e); off += z.length;
      }
      index.push(e);
    }
    process.send({ a: job.a, index, audio });
  });
  process.send({ ready: true });
} else {
  const outDir = path.resolve(opt("out", "pack"));
  fs.mkdirSync(outDir, { recursive: true });
  const bundle = createRequire(import.meta.url)(bundlePath);
  const { width, height, fps, durationInFrames, id } = bundle.meta;
  const [fa, fb] = (opt("frames", `0-${durationInFrames}`)).split("-").map(Number);
  const total = fb - fa;
  const workers = Math.max(1, Math.min(parseInt(opt("workers", String(Math.max(1, os.cpus().length - 2))), 10), total));
  const t0 = Date.now();
  // Julat kecil diagih atas permintaan: babak berat tidak tersangkut pada satu pekerja.
  const CHUNK = 24;
  let next = fa, done = 0;
  const entries = new Array(total), audio = [], shaders = {}, files = new Set();
  const shards = await Promise.all(Array.from({ length: workers }, (_, w) => new Promise((resolve, reject) => {
    const shard = `frames-${String(w).padStart(2, "0")}.bin`;
    const child = fork(fileURLToPath(import.meta.url), [bundlePath, "--fonts", fontsDir, "--project", projectRoot, "--worker", String(w), "--shard", path.join(outDir, shard)]);
    const give = () => {
      if (next >= fb) return child.send({ end: true });
      const a = next; next = Math.min(fb, a + CHUNK);
      child.send({ a, b: next });
    };
    child.on("message", (m) => {
      if (m.final) { Object.assign(shaders, m.shaders); for (const f of m.files) files.add(f); return resolve(shard); }
      if (m.index) {
        m.index.forEach((e, i) => { entries[m.a - fa + i] = [w, ...e]; });
        for (const r of m.audio) audio.push(r);
        done += m.index.length;
        if (process.stderr.isTTY) process.stderr.write(`\rbake ${done}/${total}`);
      }
      give();
    });
    child.on("exit", (code) => code && reject(new Error(`pekerja ${w} gagal (kod ${code})`)));
  })));
  // Kandungan yang sama (frame pegun) mendapat satu id walaupun dibakar oleh pekerja berlainan.
  const first = new Map();
  const index = entries.map(([si, off, len, raw, src, hash]) => {
    if (!first.has(hash)) first.set(hash, [si, off, len, raw, src - fa]);
    return first.get(hash);
  });
  const manifest = { nova_frames: 1, id, width, height, fps, frames: total, first_frame: fa, shards, shaders, index };
  fs.writeFileSync(path.join(outDir, "frames.json"), JSON.stringify(manifest));
  const cues = audioCues(audio, fps, fa);
  fs.writeFileSync(path.join(outDir, "audio.json"), JSON.stringify({ nova_audio: 1, id, fps, frames: total, cues }));
  for (const f of files) {
    const dst = path.join(outDir, f);
    fs.mkdirSync(path.dirname(dst), { recursive: true });
    fs.copyFileSync(path.join(projectRoot, f), dst);
  }
  const bytes = shards.reduce((s, f) => s + fs.statSync(path.join(outDir, f)).size, 0);
  const secs = (Date.now() - t0) / 1000;
  console.error(`\nbake: ${total} frame, ${workers} pekerja, ${secs.toFixed(1)} s (${(total / secs).toFixed(1)} fps), pek ${(bytes / 1e6).toFixed(0)} MB, ${cues.length} kiu audio`);
}
