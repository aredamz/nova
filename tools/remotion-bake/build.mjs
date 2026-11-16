#!/usr/bin/env node
// Bundel satu komposisi Remotion supaya boleh dijalankan di Node tanpa pelayar.
//
//   node build.mjs --project <akar projek Remotion> --target <fail.ts> --out <bundle.cjs>
//
// <fail.ts> mengeksport `COMP = { id, component, width, height, fps, durationInFrames }`.
// Modul yang perlukan pelayar/GPU diganti: media jadi tiada, kanvas Skia jadi elemen
// penanda yang NOVA lukis sendiri (shader SkSL yang sama, pada CPU).
import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const argv = process.argv.slice(2);
const opt = (name, def) => { const i = argv.indexOf("--" + name); return i < 0 ? def : argv[i + 1]; };
const project = path.resolve(opt("project", process.cwd()));
const target = path.resolve(opt("target"));
const outfile = path.resolve(opt("out"));
const HERE = path.dirname(fileURLToPath(import.meta.url));
const esbuild = createRequire(project + "/package.json")("esbuild");
// Laluan sebenar: node_modules yang berupa symlink (pnpm, workspace) mesti menghasilkan satu salinan modul sahaja.
const remotion = fs.realpathSync(project + "/node_modules/remotion") + "/dist/cjs/";
const exact = {
  "bake-target": target,
  remotion: HERE + "/shim-remotion.ts",
  "remotion-cjs": remotion + "index.js",
  "@shopify/react-native-skia": HERE + "/shim-skia.tsx",
  "@remotion/skia": HERE + "/shim-skia.tsx",
  "@remotion/media": HERE + "/shim-media.tsx",
  "react-native": HERE + "/shim-empty.ts",
};
await esbuild.build({
  entryPoints: [HERE + "/entry.tsx"], bundle: true, platform: "node", format: "cjs", outfile, logLevel: "error",
  absWorkingDir: project, nodePaths: [project + "/node_modules"],
  loader: { ".png": "empty", ".jpg": "empty", ".wasm": "empty" },
  define: { "process.env.NODE_ENV": '"production"' },
  plugins: [{ name: "nova-bake", setup(b) {
    b.onResolve({ filter: /.*/ }, (a) => {
      if (exact[a.path]) return { path: exact[a.path] };
      if (a.path.startsWith("remotion-cjs/")) return { path: remotion + a.path.slice(13) + ".js" };
      return undefined;
    });
    // @remotion/noise menyimpan hanya 10 seed; episod yang guna lebih banyak seed membina semula
    // jadual hingar pada hampir setiap panggilan (±45% masa render). Cache lebih besar (±9 KB setiap seed), nilai sama.
    b.onLoad({ filter: /@remotion[\\/]noise[\\/]dist[\\/].*\.m?js$/ }, async (a) => ({
      contents: (await fs.promises.readFile(a.path, "utf8")).replace(/(seedCache\dd\.size > )10\b/g, "$132768"),
      loader: "js",
    }));
  } }],
});
console.error(`bundel: ${outfile}`);
