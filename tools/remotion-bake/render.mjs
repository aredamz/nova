#!/usr/bin/env node
// Render a Remotion composition with NOVA in one command, without a browser:
// bundle -> bake -> mix audio -> write template -> nova render.
//
//   node render.mjs --project <remotion project> --target <target.ts> --out <video.mp4>
//       [--fonts <dir of .ttf/.otf>]   fonts the composition uses (system fonts otherwise)
//       [--work <dir>]                 working folder (default: <project>/.nova-bake/<target name>)
//       [--workers <n>]                bake processes (default: CPUs - 2)
//       [--lufs <n>]                   master the mixed audio to this loudness (default: no mastering)
//       [--audio <file>]               use this audio track instead of mixing the composition's <Audio>
//       [--no-audio]                   video only
//       [--quality standard|ultra|ultra1080]
//                                      ultra: frames drawn at 2x; <out> is the 2x master and
//                                      <out>-1x.mp4 the composition-size file, from the same frames
//                                      ultra1080: drawn at 2x, only the composition-size file is written
//       [--nova <binary>]              default: target/release/nova of this repository, else `nova` on PATH
//       [--clean]                      delete the frame pack after a successful render
//       [-- <more arguments for `nova render`>]
//
// <target.ts> exports `COMP = { id, component, width, height, fps, durationInFrames }`.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import opentype from "opentype.js";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const all = process.argv.slice(2);
const split = all.indexOf("--");
const argv = split < 0 ? all : all.slice(0, split);
const passThrough = split < 0 ? [] : all.slice(split + 1);
const opt = (name, def) => { const i = argv.indexOf("--" + name); return i < 0 ? def : argv[i + 1]; };
const flag = (name) => argv.includes("--" + name);
const fail = (message) => { console.error("error: " + message); process.exit(2); };

if (!opt("project") || !opt("target") || !opt("out")) {
  console.error(fs.readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").slice(1, 20).map((l) => l.replace(/^\/\/ ?/, "")).join("\n"));
  process.exit(2);
}
const project = path.resolve(opt("project"));
const target = path.resolve(opt("target"));
const out = path.resolve(opt("out"));
const quality = opt("quality", "standard");
if (!["standard", "ultra", "ultra1080"].includes(quality)) fail(`unknown --quality ${quality}`);
if (!fs.existsSync(path.join(project, "node_modules", "remotion"))) fail(`${project} has no node_modules/remotion (run npm install in the project)`);
if (!fs.existsSync(target)) fail(`target file not found: ${target}`);
const work = path.resolve(opt("work", path.join(project, ".nova-bake", path.basename(target).replace(/\.[^.]+$/, ""))));
const repoNova = path.resolve(HERE, "../../target/release/nova");
const nova = opt("nova", fs.existsSync(repoNova) ? repoNova : "nova");

const run = (cmd, args, options = {}) => {
  const r = spawnSync(cmd, args, { stdio: "inherit", ...options });
  if (r.error) fail(`cannot run ${cmd}: ${r.error.message}`);
  if (r.status !== 0) process.exit(r.status ?? 1);
  return r;
};
const seconds = () => Number(process.hrtime.bigint()) / 1e9;
const node = process.execPath;

fs.mkdirSync(path.join(work, "fonts"), { recursive: true });
fs.mkdirSync(path.dirname(out), { recursive: true });
// Fonts are copied next to the template: template assets must live inside its folder.
for (const f of fs.readdirSync(path.join(work, "fonts"))) fs.rmSync(path.join(work, "fonts", f));
const fonts = [];
if (opt("fonts")) {
  const dir = path.resolve(opt("fonts"));
  for (const f of fs.readdirSync(dir).filter((f) => /\.(ttf|otf)$/i.test(f)).sort()) {
    fs.copyFileSync(path.join(dir, f), path.join(work, "fonts", f));
    const names = opentype.loadSync(path.join(dir, f)).names;
    const family = names.preferredFamily?.en || names.fontFamily?.en;
    if (!family) fail(`font ${f} has no family name`);
    fonts.push({ id: f.replace(/\.[^.]+$/, "").toLowerCase().replace(/[^a-z0-9_-]+/g, "-"), type: "font", src: "fonts/" + f, family });
  }
}

const t0 = seconds();
run(node, [path.join(HERE, "build.mjs"), "--project", project, "--target", target, "--out", path.join(work, "bundle.cjs")]);
const t1 = seconds();
const bake = [path.join(HERE, "bake.mjs"), path.join(work, "bundle.cjs"), "--project", project, "--fonts", path.join(work, "fonts"), "--out", path.join(work, "pack")];
if (opt("workers")) bake.push("--workers", opt("workers"));
run(node, bake);
const t2 = seconds();

const meta = JSON.parse(fs.readFileSync(path.join(work, "pack", "frames.json"), "utf8"));
const cues = JSON.parse(fs.readFileSync(path.join(work, "pack", "audio.json"), "utf8")).cues;
let audio = null;
if (flag("no-audio")) {
  audio = null;
} else if (opt("audio")) {
  audio = "audio" + path.extname(opt("audio"));
  fs.copyFileSync(path.resolve(opt("audio")), path.join(work, audio));
} else if (cues.length > 0) {
  audio = "mix.wav";
  const mix = [path.join(HERE, "mix.mjs"), path.join(work, "pack", "audio.json"), "--project", project, "--out", path.join(work, audio), "--report", path.join(work, "mix.json")];
  if (opt("lufs") !== undefined) mix.push("--lufs", opt("lufs"));
  run(node, mix);
}
const t3 = seconds();

const template = {
  schema_version: "1.0",
  composition: { id: meta.id, width: meta.width, height: meta.height, fps: meta.fps, duration_frames: meta.frames, background: "#000000" },
  assets: [...fonts, { id: "bake", type: "frames", src: "pack/frames.json" }],
  ...(audio ? { audio: [{ id: "mix", src: audio }] } : {}),
  layers: [{ id: "frames", type: "frames", asset: "bake" }],
};
fs.writeFileSync(path.join(work, "template.json"), JSON.stringify(template, null, 1));

const render = ["render", path.join(work, "template.json"), "-o", out];
if (quality === "ultra") render.push("--scale", "2", "--profile", "master", "--also", out.replace(/(\.[^.]+)?$/, "-1x.mp4"));
if (quality === "ultra1080") render.push("--scale", "2", "--profile", "master", "--supersample");
run(nova, [...render, ...passThrough]);
const t4 = seconds();

if (flag("clean")) fs.rmSync(path.join(work, "pack"), { recursive: true, force: true });
const s = (a, b) => (b - a).toFixed(1) + " s";
console.error(`bundle ${s(t0, t1)} | bake ${s(t1, t2)} | audio ${s(t2, t3)} | render ${s(t3, t4)} | total ${s(t0, t4)} for ${(meta.frames / meta.fps).toFixed(1)} s of video`);
console.error(`work folder: ${work}` + (flag("clean") ? "" : " (the frame pack in it can be deleted; --clean does that)"));
