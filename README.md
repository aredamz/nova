# NOVA

[![CI](https://github.com/aredamz/nova/actions/workflows/ci.yml/badge.svg)](https://github.com/aredamz/nova/actions/workflows/ci.yml)

NOVA is a **2D** video render engine for motion graphics. It takes a JSON template and
produces an MP4 file. It is written in Rust and runs entirely on the CPU; it requires
neither a GPU nor a browser.

```bash
mkdir -p out
nova render examples/product-ad/template.json --data examples/product-ad/data.json -o out/product-ad.mp4
```

> **Scope.** NOVA draws shapes, paths, text, images and SVG in two dimensions. It does
> **not** render 3D scenes (Three.js, React Three Fiber, meshes, lights or 3D cameras).
> Existing 3D videos cannot be ported; they must be redesigned in 2D.

## Contents

- [Features](#features)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Commands](#commands)
- [Template format](#template-format)
- [Rendering Remotion compositions](#rendering-remotion-compositions)
- [Architecture](#architecture)
- [Performance](#performance)
- [Repository layout](#repository-layout)
- [Testing](#testing)
- [Troubleshooting](#troubleshooting)
- [License](#license)

## Features

**Layers and drawing**

- Nine layer types: `solid`, `rect`, `ellipse`, `path` (SVG path data), `text`, `image`
  (PNG, JPEG and WebP; EXIF orientation is honoured), `svg` (static), `group`, and `frames`
  (a sequence of frames prepared outside NOVA, for example a Remotion composition executed
  without a browser; see [Rendering Remotion compositions](#rendering-remotion-compositions)).
- Solid, linear-gradient and radial-gradient fills. Strokes with cap, join and trim, which
  allows "drawn line" effects.
- Transforms: position, normalised anchor, scale (uniform or per axis) and rotation.
- Effects: opacity, 13 blend modes, Gaussian blur, drop shadow, and clipping with rounded
  corners. Opacity on a `group` applies to its content as a single unit, as in CSS.

**Text**

- Fonts are loaded from files declared in the template, with fallback lists, weight,
  italic, line height, letter spacing and alignment.
- Line wrapping at `max_width`, and truncation with an ellipsis at `max_lines`.
- `spans`: a different colour for part of the text.
- `reveal`: text revealed word by word (karaoke captions) or character by character
  (typewriter) without changing the layout.
- `background`: a panel behind the text (caption box, name tag, stamp), with `hug` so that
  the panel fits short text.

**Animation and timing**

- Keyframes on transform, opacity, blur, shadow, colour, size, corner radius, stroke and
  text properties.
- Easing: nine named curves, CSS-style cubic-bezier, and a spring solved in closed form
  (the defaults match Remotion's `spring()`).
- Scenes: a timed `group` acts as a sequence; moving its `start_frame` moves all of its
  children and their animations.
- Integer or fractional frame rates (for example 30000/1001).

**Templates and data**

- Data binding with `{{slot}}` placeholders and dotted paths (`{{product.name}}`), so one
  template can produce many videos.
- Templates contain no code: no expressions and no scripts. Asset paths that leave the
  template folder (absolute paths, `..`, URLs and symlinks) are rejected.
- The JSON Schema is generated from the same parser, which makes it suitable as input for
  an AI agent that generates templates.
- `nova validate` reports every error at once, each with its exact location (for example
  `layers[3].text.font_size`).

**Output**

- H.264 MP4 (yuv420p, BT.709) through FFmpeg. Four profiles are provided (`preview`,
  `balanced`, `high_quality` and `master`); the preset, CRF and tune can also be set
  individually.
- `--scale 2` redraws the entire composition at twice the size (1080×1920 becomes
  2160×3840). Vectors, text and baked frames are drawn at that size rather than enlarged.
- `--also file.mp4` writes a second file at the template size from the same frames in the
  same run. `--supersample` writes only the template-size file, reduced from the larger
  frames.
- Audio: multiple tracks with a start time and volume, mixed to AAC.
- A single frame as PNG (`nova preview`), or a contact sheet of several frames
  (`nova sheet`) for reviewing a whole video at a glance.
- Partial frame ranges (`--start`, `--end`) and a JSON job report (`--json`).

**Engine**

- CPU only. The result is a single binary of approximately 22 MB.
- Deterministic: the output is bit-identical for any number of threads.
- Parallel rendering with bounded memory; render threads wait when the encoder is slower.
- Work is reused: frames identical to the previous one are not rendered again, and
  expensive layers that do not change are cached.
- `nova doctor` checks the machine, and `nova benchmark` measures throughput.

**Not available in version 0.1**

In templates: 3D scenes, video layers, animated GIF or WebP, precompositions, masks and
mattes, motion blur, path morphing, per-character transform animation, automatic layout
(flex, auto-fit text), audio fades and ducking, caching across jobs, a REST API and a job
queue. Right-to-left text, CJK text and emoji have not been tested.

Arbitrary clipping, masks, SVG filters, shaders and geometry that changes on every frame
are available through the `frames` layer rather than through ordinary template layers.

## Installation

### Requirements

- Linux on x86-64, or macOS on Apple Silicon. Both are built and tested by the CI
  workflow on every push; development is carried out on Ubuntu. See
  [Platform notes](#platform-notes).
- A Rust toolchain (stable) to build the engine.
- FFmpeg with libx264, and `ffprobe`.
- The FreeType and Fontconfig libraries.
- Node.js, only for the Remotion bake tools (tested with Node.js 26).

### Ubuntu 24.04 or later, with sudo

```bash
git clone https://github.com/aredamz/nova.git
cd nova
scripts/setup-ubuntu.sh
```

The script installs the system packages (`ffmpeg`, `libfontconfig1-dev`,
`libfreetype-dev`), installs Rust if it is missing, runs `cargo build --release` and
finishes with `nova doctor`.

### macOS (Apple Silicon)

Requires [Homebrew](https://brew.sh):

```bash
git clone https://github.com/aredamz/nova.git
cd nova
scripts/setup-macos.sh
```

The script installs `ffmpeg` and `fontconfig` with Homebrew, installs Rust if it is
missing, runs `cargo build --release` and finishes with `nova doctor`.

### Linux without sudo

On a machine that already provides `ffmpeg`, `libfreetype6` and `libfontconfig1`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
scripts/dev-link-shim.sh       # linker symlinks in .link-shim/, in place of the -dev packages
cargo build --release
```

### Result

The binary is `target/release/nova`. Add it to `PATH`, or call it by its path:

```bash
export PATH="$PWD/target/release:$PATH"
nova doctor
```

The first build downloads a prebuilt Skia library, so no C++ compilation takes place. If
no prebuilt binary exists for the target, `skia-bindings` attempts to build Skia from
source, which requires clang, Python and ninja and takes considerably longer.

### Platform notes

- **Text rendering.** Skia rasterises glyphs with FreeType on Linux and with CoreText on
  macOS, so the edges of text differ slightly between the two systems. Output is
  deterministic on a given platform, but not pixel-identical across platforms.
- **Colour conversion.** The RGBA to I420 conversion has an AVX2 implementation for
  x86-64; other processors, including Apple Silicon, use the portable implementation.
- **`nova doctor` and reports.** CPU quota, steal time and peak memory are read from
  Linux interfaces and are reported as unknown on macOS.
- **Remotion bake and system fonts.** When a composition names a font that is not
  supplied with `--fonts`, the bake tools look it up through fontconfig using the rules of
  Chrome on Linux. On macOS, supply every font the composition uses with `--fonts`.

## Quick start

```bash
mkdir -p out

# Validate a template, then render it
nova validate examples/product-ad/template.json --data examples/product-ad/data.json
nova render   examples/product-ad/template.json --data examples/product-ad/data.json -o out/product-ad.mp4

# Review a video at a glance before a full render
nova sheet examples/shorts/template.json --assets-root examples --count 8 --columns 4 -o out/shorts-sheet.png
```

Further examples are described in [`examples/README.md`](examples/README.md).

## Commands

| Command | Purpose |
|---|---|
| `nova validate t.json [--data d.json]` | Checks a template completely (structure, references, assets and fonts) without rendering. All errors are reported at once. |
| `nova inspect t.json` | Lists layers, timing, assets, data slots and the number of frames that can be reused. |
| `nova preview t.json --frame 120 -o f.png` | Renders one frame to a PNG file. |
| `nova sheet t.json --count 12 -o sheet.png` | Renders evenly spaced frames onto one contact sheet. |
| `nova render t.json -o out.mp4` | Renders the composition to MP4 (H.264, yuv420p, BT.709). |
| `nova benchmark t.json --threads 1,4,8` | Measures render-only and end-to-end throughput; reports the median of several runs. |
| `nova doctor` | Reports CPU, quota, steal time, SIMD support, memory, FFmpeg and fonts. |
| `nova schema` | Prints the JSON Schema of the template format. |

Frequently used options:

```bash
nova render t.json -o out.mp4 --data d.json   # fill the {{...}} slots
        --profile preview|balanced|high_quality|master   # default: balanced (x264 medium, CRF 18)
        --preset veryfast --crf 18 --tune grain   # override the profile settings
        --scale 2 --also out-1x.mp4               # 2x master and a template-size file in one run
        --scale 2 --supersample                   # draw at 2x, write the template size only
        --threads 12 --encoder-threads 4          # CPU allocation
        --start 300 --end 600                     # a partial frame range
        --no-system-fonts                         # use only the fonts declared in the template
        --json                                    # machine-readable job report
```

Exit codes: `0` on success; `2` when the template or data is invalid (correct the input);
`1` for any other failure (FFmpeg, I/O, cancellation). Ctrl-C cancels cleanly: FFmpeg is
stopped and the partial file is removed.

## Template format

The complete reference is [`docs/template-format.md`](docs/template-format.md). The schema
is [`schemas/template-v1.schema.json`](schemas/template-v1.schema.json).

```json
{
  "schema_version": "1.0",
  "composition": { "id": "demo", "width": 1920, "height": 1080, "fps": 30, "duration_frames": 150, "background": "#0b1020" },
  "assets": [ { "id": "bold", "type": "font", "src": "assets/NotoSans-Bold.ttf", "family": "Noto Sans" } ],
  "layers": [
    { "id": "title", "type": "text",
      "transform": { "position": { "x": 960, "y": 540 }, "anchor": { "x": 0.5, "y": 0.5 } },
      "text": { "value": "{{title}}", "font_family": "Noto Sans", "font_size": 120, "font_weight": 700 },
      "animations": [
        { "property": "transform.scale", "keyframes": [ { "frame": 0, "value": 0.6, "easing": "spring" }, { "frame": 30, "value": 1 } ] },
        { "property": "opacity", "keyframes": [ { "frame": 0, "value": 0 }, { "frame": 12, "value": 1 } ] }
      ] }
  ]
}
```

## Rendering Remotion compositions

A template describes shapes and keyframes. That is not sufficient for videos whose geometry
is recomputed by code on every frame. For such videos NOVA provides a second path, the
*frame bake*: the React code of a [Remotion](https://www.remotion.dev) composition is
executed in Node.js without a browser, the markup of every frame is converted to SVG and
shader operations, and the `frames` layer draws the result. Audio declared with `<Audio>`
is mixed into a single track.

The repository contains a small example project. After building NOVA:

```bash
(cd tools/remotion-bake && npm install)          # once: the bake tools
(cd examples/remotion-bake && npm install)       # once: React and Remotion for the example

node tools/remotion-bake/render.mjs \
     --project examples/remotion-bake \
     --target  examples/remotion-bake/target.ts \
     --fonts   examples/shared/fonts \
     --out     out/hello.mp4
```

For an existing Remotion project, write a target file that exports the composition and
pass the project folder:

```ts
// target.ts
import { MyVideo } from "./src/MyVideo";
export const COMP = { id: "MyVideo", component: MyVideo, width: 1080, height: 1920, fps: 30, durationInFrames: 900 };
```

```bash
node tools/remotion-bake/render.mjs --project /path/to/project --target /path/to/project/target.ts \
     --fonts /path/to/fonts --out video.mp4 --lufs -14 --quality ultra1080
```

`--quality ultra` draws every frame at twice the size and writes a 2x master together
with a composition-size file; `--quality ultra1080` writes only the composition-size file,
reduced from the 2x frames.

The bake supports a defined subset of HTML and CSS, inline SVG, and `<Fill><Shader>` from
`@remotion/skia`; anything it does not understand is reported as a warning during the bake.
It was developed and tested against Remotion 4.0.542 and React 19, and relies on Remotion
internals, so other Remotion versions may require adjustments. The supported subset, the
pack format and the audio rules are documented in
[`docs/bake-remotion.md`](docs/bake-remotion.md), and the tools in
[`tools/remotion-bake/README.md`](tools/remotion-bake/README.md).

Remotion is a separate product with its own licence. NOVA does not include Remotion code;
the bake tools use the copy installed in the project being rendered.

## Architecture

```text
JSON template + data ──> bind ──> validate ──> Scene (immutable, shared)
                                                 │
                               evaluate(frame) ──┴──> FramePlan (plain data)
                                                          │
         N render threads:  FramePlan ──Skia CPU──> RGBA ──> I420 (BT.709)
                                                          │
                    ordered writer ──pipe──> FFmpeg (libx264) ──> MP4
```

- **Deterministic and random-access.** Every frame is a pure function of its frame number.
  The output is bit-identical for any number of threads, and this is covered by tests.
- **Bounded memory.** Render threads may run ahead of the writer by one window only (by
  default three times the thread count). When the encoder is slower, rendering waits
  instead of accumulating frames.
- **No repeated work.** A frame identical to the previous one is not rendered again;
  expensive layers that do not change are rendered once and cached; images are prepared
  once at their largest displayed size.
- **Quality.** Vectors and text are always drawn directly at the final transform, never
  from a scaled bitmap. Compositing takes place in sRGB, as in a browser. Gradients are
  dithered. No intermediate JPEG is used: frames are passed directly to the encoder.

## Performance

Measured figures and the procedure for reproducing them are given in
[`docs/prestasi.md`](docs/prestasi.md). In summary:

- Template path, on a four-core laptop: a four-minute 1080p video at 30 fps renders in
  approximately two minutes (x264 `medium`).
- Frame bake path, on a 16-vCPU server without a GPU: two four-minute 1080×1920 Remotion
  compositions rendered in seven to eight minutes each, audio included. The original
  Chromium renders of the same compositions took approximately 76 and 101 minutes
  according to their render records; those were not measured again.

```bash
scripts/run-benchmarks.sh                       # the full benchmark set
THREADS=1,4,8,12,16 RUNS=5 scripts/run-benchmarks.sh
```

## Repository layout

```text
crates/
  nova-core/        time, colour, easing, keyframes, geometry, Scene, evaluate(); no I/O and no backend
  nova-template/    JSON DSL, data binding, validation, compilation to a Scene
  nova-raster/      FramePlan to pixels: Skia CPU backend, layer cache, image, SVG and font preparation
  nova-encoder/     RGBA to I420 (AVX2), the FFmpeg process, ffprobe
  nova-scheduler/   parallel rendering with a bounded window and ordered output
  nova-cli/         the `nova` binary
tools/remotion-bake/  bake a Remotion composition into a frame pack and mix its audio
examples/           example templates and the Remotion bake example; see examples/README.md
benchmarks/         benchmark workload generator
schemas/            JSON Schema of the template format
scripts/            setup scripts (Ubuntu, macOS) and benchmark scripts
docs/               template format, frame bake, design notes and performance
```

The documents in `docs/` are currently written in Malay.

## Testing

```bash
cargo test --workspace --release        # 138 tests: unit, integration, golden image and CLI
(cd tools/remotion-bake && npm install && npm test)   # 13 tests for the bake tools
cargo clippy --workspace --all-targets
NOVA_UPDATE_GOLDEN=1 cargo test -p nova-raster --test golden   # regenerate; review every PNG that changes
```

Tests that require FFmpeg are skipped when it is not installed. The CI workflow
([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the setup script, both test
suites and the example renders on Ubuntu and on macOS (Apple Silicon).

## Troubleshooting

| Symptom | Cause and action |
|---|---|
| `unable to find library -lfreetype` during the build | Install `libfreetype-dev` and `libfontconfig1-dev`, or run `scripts/dev-link-shim.sh`. |
| Warning "is not a font asset of this template" | The font is not registered in `assets`, so a system font is used. Add the font file so that the result is the same on every machine. |
| Text appears as empty boxes | The font has no glyph for that character. Add a fallback font: `"font_family": ["Noto Sans", "Noto Sans Arabic"]`. |
| `width and height must be even` | H.264 yuv420p requires even dimensions. |
| `output directory ... does not exist` | Create the output folder first (`mkdir -p out`). |
| `cannot start FFmpeg` | Install FFmpeg, or set `NOVA_FFMPEG=/path/to/ffmpeg` (and `NOVA_FFPROBE`). |
| Rendering is slower than expected | Run `nova doctor` (CPU quota, steal time). Check the "limited by" line after a render: `encoder` means x264 is the limit, so try `--preset veryfast`; `renderer` means the scene itself is expensive, see the advice in `docs/template-format.md`. |

## License

MIT, as declared in [`Cargo.toml`](Cargo.toml).
