# remotion-bake

Tools that bake a Remotion composition into a NOVA frame pack without a browser, and mix
its audio. The complete documentation is
[`docs/bake-remotion.md`](../../docs/bake-remotion.md) (in Malay).

## Usage

```bash
npm install                                   # once, in this folder (opentype.js)

node render.mjs --project <remotion project> --target <target.ts> --out <video.mp4> \
     [--fonts <folder of .ttf/.otf>] [--lufs -14] [--quality standard|ultra|ultra1080] [--clean]
```

`render.mjs` runs every step: bundle, bake, audio mix, template and `nova render`. Run it
without arguments to list all options. `<target.ts>` exports the composition:

```ts
import { MyVideo } from "./src/MyVideo";
export const COMP = { id: "MyVideo", component: MyVideo, width: 1080, height: 1920, fps: 30, durationInFrames: 900 };
```

A ready-to-run example is in [`examples/remotion-bake`](../../examples/remotion-bake).

The steps can also be run individually:

```bash
node build.mjs --project <project> --target target.ts --out bundle.cjs
node bake.mjs  bundle.cjs --project <project> --fonts <fonts> --out pack
node mix.mjs   pack/audio.json --project <project> --out mix.wav --lufs -14
nova render template.json -o video.mp4        # the template refers to pack/frames.json and mix.wav
```

## Files

| File | Role |
|---|---|
| `render.mjs` | Runs the whole pipeline in one command. |
| `build.mjs` | Bundles the composition with the project's own esbuild and replaces browser-only modules. |
| `entry.tsx` | Provides the Remotion context and renders one frame to markup. |
| `shim-*.ts(x)` | `remotion` without a browser: `<Audio>` records cues, Skia becomes marker elements. |
| `convert.mjs` | Converts HTML and SVG markup to plain SVG and shader operations (a small HTML layout engine). |
| `fonts.mjs` | Measures text with the real fonts (opentype.js); system fonts follow Chrome's rules. |
| `bake.mjs` | Parallel workers (work is distributed on demand); writes `frames.json`, the shards and `audio.json`. |
| `audio.mjs`, `mix.mjs` | Audio records to cues to a single WAV track (optional `--lufs` master). |
| `test.mjs` | `npm test` (requires ffmpeg and fontconfig). |

## Requirements

- `esbuild`, `react`, `react-dom` and `remotion` installed in the Remotion project passed
  with `--project`. The tools were developed against Remotion 4.0.542 and React 19 and
  rely on Remotion internals, so other versions may require adjustments.
- `opentype.js` in this folder (`npm install`).
- `ffmpeg`, `ffprobe` and `fc-match` on `PATH`.
- A built `nova` binary (`cargo build --release` in the repository root).

Messages printed by the individual tools are currently in Malay.
