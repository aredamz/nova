# NOVA — 2D Motion Graphics Rendering Engine

> **Dokumen:** `plan.md`  
> **Status:** Technical plan / implementation blueprint  
> **Versi dokumen:** 2.0  
> **Tarikh kemas kini:** 15 November 2026  
> **Sasaran deployment awal:** Ubuntu 24.04, 16 vCPU, 32 GB RAM, tiada GPU  
> **Skop:** Rendering video 2D sahaja

---

## 0. Ringkasan eksekutif

NOVA ialah enjin rendering video 2D yang dibina menggunakan Rust dan dioptimumkan untuk menghasilkan motion graphics, typography animation, social-media creatives, advertisement, branded templates, SVG compositions dan video berasaskan data. Matlamat produk kini lebih khusus: **mencapai kualiti visual yang diterima sebagai setara dengan output Remotion bagi kelas template yang disokong, sambil mengurangkan masa render end-to-end pada VPS CPU-only.**

NOVA bukan browser automation tool. Ia tidak menggunakan Chromium, DOM, CSS layout engine atau screenshot browser untuk menghasilkan frame. Template diterangkan melalui JSON DSL yang tervalidasi, dikompilkan menjadi scene graph, dinilai berdasarkan frame/time yang diminta, dirasterkan ke framebuffer RGBA, dikomposit dan dihantar kepada FFmpeg untuk encoding.

**Prinsip utama:**

1. CPU-first dan berfungsi tanpa GPU.
2. Tiada Chromium atau browser runtime.
3. Rendering deterministik: frame boleh dihasilkan dalam apa-apa susunan tanpa bergantung pada frame sebelumnya.
4. Multithreaded rendering dengan penggunaan RAM yang terkawal.
5. Reusable template: satu komposisi boleh menerima banyak set data tanpa mengubah kod engine.
6. Cache asset, glyph dan layer statik apabila selamat.
7. Gunakan library matang untuk primitive khusus; bina sendiri timeline, scene graph, compiler, scheduler, compositor policy, cache dan API.
8. FFmpeg mengendalikan codec dan muxing; NOVA tidak membina codec video sendiri.

### Keputusan seni bina untuk MVP — dikemas kini mengikut sasaran prestasi dan kualiti

| Bahagian | Pilihan / calon | Peranan |
|---|---|---|
| Bahasa | Rust stable | Core engine, CLI dan workers |
| Raster backend parity candidate | `skia-safe` CPU raster surface | Calon utama untuk kualiti paths, paints, filters, image sampling dan text integration |
| Raster backend lightweight | `tiny-skia` | Baseline ringan dan fallback untuk primitives 2D |
| Raster backend eksperimen | `vello_cpu` | Benchmark SIMD/CPU; bukan dependency wajib sehingga feature parity dibuktikan |
| SVG | `usvg` + `resvg` | Parse dan render SVG statik; benchmark juga laluan Skia SVG jika diperlukan |
| Text | `cosmic-text` + Swash atau Skia textlayout/HarfBuzz | Shaping, fallback, layout dan glyph rasterization; bandingkan visual parity dan kelajuan |
| Image decode | `image` atau Skia codec | Decode sekali, cache decoded asset dan variants |
| Parallelism | `rayon` + custom bounded scheduler | Frame-level parallelism dan work stealing |
| Serialization | `serde` + `serde_json` | Template DSL |
| Validation | JSON Schema + semantic validator Rust | Validasi input, assets dan resource limits |
| Video output | FFmpeg `libavcodec`/CLI raw-frame pipe | Encode dan mux video; asingkan masa encode daripada render |
| API kemudian | `axum` | REST API untuk render jobs |
| Queue kemudian | Redis + Rust worker | Queue berbilang job dan retry |

**Keputusan penting:** jangan mengunci `tiny-skia` sebagai backend tunggal sebelum benchmark kualiti dan kelajuan. `skia-safe` patut menjadi calon parity utama kerana Skia menyediakan CPU raster surfaces dan ekosistem drawing/effects yang luas; bina proof-of-concept kecil untuk menilai build complexity, binary size, memory dan throughput. `tiny-skia` kekal calon ringan. `vello_cpu` diuji sebagai backend prestasi berasingan kerana dokumentasi semasa menyatakan had pada filter graphs dan multithreaded filters. Semua backend mesti melalui trait yang sama.

NOVA bukan cuba menyalin keseluruhan browser. Kita meniru hanya subset visual Remotion yang diperlukan oleh template sasaran: typography, vector, image compositing, gradients, masks, shadows, blur, blend modes dan transitions yang ditetapkan dalam feature matrix.


### 0.1 Sasaran utama yang baharu: Remotion-level quality, faster CPU rendering

Pengguna melaporkan satu video berdurasi kira-kira 4 minit mengambil lebih kurang 30 minit untuk dirender menggunakan Remotion. NOVA mesti menggunakan angka ini sebagai **baseline awal yang perlu disahkan**, bukan sebagai bukti bahawa Remotion sentiasa berjalan pada kelajuan tersebut.

Andaian pengiraan jika output ialah 30 FPS:

- 4 minit × 60 saat × 30 FPS = **7,200 frames**.
- 30 minit = 1,800 saat.
- 7,200 / 1,800 = **4 rendered frames/second** secara purata.
- Purata masa end-to-end bagi setiap frame bersamaan 250 ms, termasuk apa-apa overhead pipeline yang belum diasingkan.

| Masa siap bagi video 4 minit | Throughput efektif pada 30 FPS | Faktor lebih pantas berbanding baseline 30 minit |
|---:|---:|---:|
| 30 minit | 4 FPS | 1× (baseline) |
| 20 minit | 6 FPS | 1.5× |
| 15 minit | 8 FPS | 2× |
| 10 minit | 12 FPS | 3× |
| 6 minit | 20 FPS | 5× |
| 4 minit | 30 FPS | 7.5× / real-time |

Angka FPS di atas ialah throughput purata sepanjang render, bukan playback FPS. Jika video sebenar 60 FPS, frame count dan sasaran throughput perlu digandakan. Ukur FPS, resolution, codec, CRF, audio, assets dan penggunaan Remotion sebenar sebelum menetapkan perbandingan muktamad.

**KPI berperingkat (sasaran, bukan janji):**

1. **Gate A — lebih pantas:** NOVA end-to-end median sekurang-kurangnya 1.25× lebih pantas daripada Remotion baseline untuk composition yang sama, tanpa penurunan visual yang tidak diterima.
2. **Gate B — sasaran produk awal:** 2× lebih pantas (sekitar 15 minit atau kurang bagi contoh baseline 30 minit).
3. **Gate C — sasaran agresif:** 5× lebih pantas (sekitar 6 minit) untuk template yang sesuai dengan caching dan frame parallelism.
4. **Stretch goal:** menghampiri 4 minit (real-time throughput) untuk composition ringan/sederhana. Ini bukan sasaran wajib bagi semua composition.

Jangan optimize nombor render sahaja. Kualiti visual, audio sync, deterministic output, memory safety dan reliability ialah syarat penerimaan yang sama penting.

### 0.2 Wajib ukur Remotion dahulu sebelum membina semula

30 minit boleh berpunca daripada rendering React/DOM, concurrency tidak optimum, media decode, image format, asset fetch, effect GPU yang jatuh ke software path, FFmpeg encoding, atau satu bottleneck dalam template. NOVA tidak boleh menyelesaikan bottleneck yang belum dikenal pasti.

Sebelum implementation berat:

- Jalankan `npx remotion benchmark` pada VPS dan rekod concurrency yang disyorkan.
- Jalankan render dengan verbose logs dan simpan slowest-frame timings.
- Uji concurrency 4, 6, 8, 10, 12 dan 16 secara berasingan; jangan anggap nilai maksimum paling pantas.
- Sahkan versi Remotion; Linux multiprocess Chromium diaktifkan secara default pada versi 4.0.137 dan ke atas menurut dokumentasi semasa.
- Asingkan bundle/startup, asset download, frame render, media decode dan encoding.
- Semak PNG vs JPEG untuk frame intermediates apabila alpha tidak diperlukan; PNG boleh lebih perlahan.
- Semak penggunaan `<Video>` / `@remotion/media`, `<OffthreadVideo>` cache dan bilangan video decode threads jika composition menggunakan video.
- Semak sama ada composition menggunakan WebGL/Three.js/Skia/P5 effects yang mungkin berubah prestasi pada mesin tanpa GPU.
- Rekod `renderMedia()` config, concurrency, Chromium flags, FFmpeg codec/preset/CRF, resolution, FPS dan audio.

Rujukan prestasi Remotion semasa: `renderMedia()` / `renderFrames()` options, `npx remotion benchmark`, concurrency tuning dan verbose frame timings. NOVA benchmark mesti dibandingkan dengan Remotion yang telah ditune secara munasabah, bukan konfigurasi Remotion yang sengaja tidak optimum.

### 0.3 Prinsip performance-first

Urutan optimization NOVA:

1. **Kurangkan kerja yang perlu dibuat** — compile template sekali, cache asset/glyph/layout, precompute curves, reuse rasterized layers dan skip render untuk static spans.
2. **Render parallel** — frame-level scheduling dengan bounded in-flight frames; tile-level parallelism hanya jika profile menunjukkan satu frame terlalu berat.
3. **Kurangkan memory traffic** — zero/low-copy buffers, buffer pools, cache locality, avoid full-frame intermediate copies.
4. **Optimize hot loops** — SIMD/runtime CPU feature dispatch hanya selepas profiling.
5. **Encode tanpa menahan renderer** — bounded producer/consumer pipeline, encoder thread budget dan measured codec profile.
6. **Jaga visual parity** — reference frames daripada Remotion dan golden tests untuk typography/effects.

Jangan bermula dengan menulis codec sendiri atau micro-optimize easing math. Fokus pada frame rendering, image decode, filters, compositing, text/glyph rasterization dan data movement kerana ini lebih berpotensi mendominasi masa.

---

## 1. Matlamat produk dan definisi kejayaan

### 1.1 Matlamat

NOVA mesti boleh:

- Menerima composition/template dalam format JSON.
- Memproses input data dan asset yang dibenarkan.
- Menghasilkan frame 2D mengikut timeline dan frame rate.
- Melukis text, shape, path, image dan SVG.
- Mengaplikasikan transform, opacity, easing, keyframes dan transitions.
- Menggabungkan layer mengikut z-order dan blend rules.
- Menghasilkan output video MP4 melalui FFmpeg.
- Menjalankan banyak job tanpa kehabisan RAM atau membebankan API.
- Memberi output yang konsisten untuk input dan konfigurasi yang sama.

### 1.2 Pengguna sasaran

- AI agent yang menghasilkan template JSON.
- Backend automation yang menghasilkan iklan atau social media video.
- Developer yang membina template visual dan mahu mengisi data dinamik.
- Sistem batch rendering yang menghasilkan banyak variasi video daripada satu template.

### 1.3 Bukan matlamat

NOVA tidak akan membina perkara ini dalam MVP:

- 3D scenes, 3D camera, mesh atau ray tracing.
- GPU rendering atau GPU dependency.
- Browser/HTML/CSS/DOM execution.
- Video editing UI atau timeline GUI.
- Full video editor dengan interactive playback.
- Codec H.264/H.265/AV1 sendiri.
- Arbitrary JavaScript, Lua, Python atau WASM execution dalam template.
- Animated SVG dengan seluruh spesifikasi SMIL/CSS animation.
- Full After Effects expression compatibility.
- Real-time game engine.

**Prinsip skop:** NOVA ialah 2D composition renderer untuk menghasilkan video, bukan pengganti penuh After Effects atau browser.

### 1.4 Visual parity dengan Remotion

NOVA tidak perlu menyokong semua kemungkinan React/DOM/CSS Remotion. Ia mesti menghasilkan output yang setara untuk **NOVA Template Profile** yang ditetapkan dan disokong. Tentukan feature matrix secara eksplisit:

| Kategori | Keperluan parity |
|---|---|
| Typography | Font family/weight/style, kerning, tracking, line-height, wrapping, alignment, fallback dan glyph coverage |
| Shapes/vector | Bezier paths, stroke joins/caps, fills, gradients, clipping dan antialiasing |
| Images | Crop/contain/cover, transform, alpha, interpolation dan color handling |
| Layer composition | Z-order, opacity, premultiplied alpha rules dan blend modes yang disokong |
| Animation | Easing, keyframes, subframe interpolation, transforms dan timing yang tepat |
| Effects | Shadow, blur, masks dan transitions yang disenaraikan dalam feature matrix |
| Output | Resolution, FPS, pixel format, color space, audio timing dan encoder profile |

Gunakan asset/font yang sama, nilai warna yang sama, resolution/FPS sama dan reference PNG frames daripada Remotion. Bandingkan sekurang-kurangnya 100 frame terpilih termasuk frame dengan text, gradient, blur, masks, transition dan high-detail images.

Visual acceptance perlu gabungkan:

- SSIM/perceptual metric sebagai signal automatik, bukan satu-satunya keputusan.
- Pixel difference heatmap untuk mencari clipping, alignment dan alpha errors.
- Human review side-by-side pada 100% zoom.
- Typography review khusus: baseline, glyph shape, kerning, line breaks dan antialiasing.
- Audio/video sync test untuk final MP4.

Cadangan threshold awal: SSIM purata ≥0.98 bagi scene yang sesuai untuk pixel comparison, tetapi typography dan filter tertentu memerlukan human acceptance kerana rasterizer berlainan boleh menghasilkan edge pixels berlainan walaupun rupa visual hampir sama. Threshold akhir mesti ditetapkan daripada reference set sebenar.

### 1.5 Kriteria kejayaan MVP

MVP dianggap siap apabila:

- CLI boleh validate, preview dan render template JSON.
- Sekurang-kurangnya 8 jenis layer asas berfungsi: solid, rect, ellipse, path, text, image, SVG dan group.
- Transform, opacity dan keyframes berfungsi secara deterministik.
- Output MP4 1080p/30 FPS berjaya dijana.
- Text Bahasa Melayu/Inggeris dengan font fallback berfungsi; sokongan shaping kompleks diuji.
- Rendering boleh berjalan dengan 1, 2, 4, 8 dan sehingga 16 worker threads.
- Memory usage terkawal dengan bounded in-flight frames.
- Snapshot tests dapat mengesan perubahan visual yang tidak disengajakan.
- Visual acceptance lulus berbanding reference Remotion untuk template profile yang disokong.
- Benchmark end-to-end menunjukkan sekurang-kurangnya 1.25× lebih pantas daripada Remotion baseline yang telah ditune; sasaran produk v1 ialah 2×.
- Tiada kebergantungan pada Chromium atau GPU.

---

## 2. Sasaran hardware dan andaian operasi

### 2.1 VPS awal

| Resource | Nilai |
|---|---:|
| OS | Ubuntu 24.04 |
| CPU | 16 vCPU |
| RAM | 32 GB |
| GPU | Tiada |
| Rendering target | 2D CPU rendering |
| Output awal | 1920×1080, 30 FPS, MP4 |

vCPU tidak semestinya sama dengan 16 core fizikal yang eksklusif. Prestasi sebenar bergantung pada CPU host, CPU quota, steal time, memory bandwidth dan jenis virtualization. NOVA mesti mengukur hardware sebenar semasa startup dan benchmark.

### 2.2 Dasar pengagihan resource awal

Jangan hard-code bahawa semua 16 vCPU sentiasa tersedia. Gunakan konfigurasi runtime:

- `render_threads`: bermula dengan 8 atau 10; boleh diuji sehingga 16.
- `max_in_flight_frames`: 2–3 kali `render_threads` sebagai titik mula, kemudian ukur.
- `max_concurrent_jobs`: 1 untuk MVP; dinaikkan selepas per-frame parallelism stabil.
- Simpan resource untuk OS, API, Redis, storage I/O dan FFmpeg.
- Elakkan nested parallelism yang menyebabkan Rayon workers, image-processing workers dan FFmpeg threads berebut CPU.

### 2.3 RAM dan framebuffer

Anggaran saiz satu RGBA8 framebuffer:

- 1920×1080×4 = 8,294,400 bytes, kira-kira 7.91 MiB.
- 3840×2160×4 = 33,177,600 bytes, kira-kira 31.64 MiB.

Ini belum termasuk intermediate layers, decoded images, glyph masks, SVG resources dan encoder buffers. Oleh itu:

- Jangan simpan keseluruhan video sebagai frame array dalam RAM.
- Gunakan bounded channel dan buffer pool.
- Hadkan bilangan frame yang sedang diproses.
- Buat memory accounting untuk cache.
- Tambah backpressure apabila encoder lebih perlahan daripada renderer.

---

## 3. Architecture high-level

```text
                     TEMPLATE JSON + DATA + ASSETS
                                   |
                                   v
                         1. INPUT / VALIDATION
                                   |
                                   v
                         2. TEMPLATE COMPILER
                    parse -> resolve -> normalize
                                   |
                                   v
                             SCENE GRAPH
                                   |
                                   v
                       TIMELINE / ANIMATION EVAL
                                   |
                                   v
                            FRAME PLANNER
                  active layers, transforms, bounds
                                   |
                                   v
                         RENDER SCHEDULER
                     bounded frame/task window
                                   |
                  +----------------+----------------+
                  |                |                |
                  v                v                v
             WORKER 0         WORKER 1         WORKER N
             rasterize        rasterize        rasterize
                  |                |                |
                  +----------------+----------------+
                                   |
                                   v
                          ORDERED FRAME QUEUE
                                   |
                                   v
                          COMPOSITOR / OUTPUT
                                   |
                                   v
                         FFmpeg ENCODER / MUXER
                                   |
                                   v
                             MP4 / WEBM
```

### 3.1 Komponen utama

1. **Input layer** — membaca fail template, data dan assets.
2. **Validation** — JSON Schema validation, type checking, semantic validation dan security validation.
3. **Template compiler** — menormalisasi nilai, mengira frame ranges, resolve references, parse paths dan prepare asset.
4. **Scene graph** — struktur layer, group, transform, styles dan z-order.
5. **Timeline evaluator** — mengira nilai property berdasarkan frame/time.
6. **Frame planner** — memilih layer aktif, transform akhir, clipping bounds dan asset yang diperlukan untuk satu frame.
7. **Raster backend** — melukis vector/path/image/text ke pixel buffer.
8. **Compositor** — menggabungkan layer secara tertib dengan alpha/blend rules.
9. **Scheduler** — mengagihkan frame kepada worker dengan had memori.
10. **Encoder** — menghantar frame tersusun ke FFmpeg dan menghasilkan video.

### 3.2 Prinsip modular

Scene graph, timeline, animation dan template DSL tidak boleh bergantung secara terus pada `tiny-skia`. Raster backend mesti berada di belakang trait/interface supaya backend boleh diganti atau dibandingkan.

---

## 4. Technology stack dan keputusan dependency

### 4.1 Core language: Rust

Sebab:

- Native compiled runtime.
- Memory safety dan ownership membantu kawal buffer besar.
- Concurrency tanpa data race melalui type system.
- Ekosistem library graphics dan media yang relevan.
- Sesuai untuk CLI dan server process yang berterusan.

Gunakan stable Rust. Pin versi dependency dalam `Cargo.lock` untuk build reproducible. Jangan gunakan nightly features untuk MVP melainkan ada benchmark yang menunjukkan keperluan khusus.

### 4.2 Rasterizer backend: benchmark Skia CPU vs tiny-skia vs Vello CPU

Penggunaan tiny-skia jika dipilih selepas benchmark:

- Fill/stroke paths.
- Rectangles, circles dan vector geometry.
- Solid colours, linear/radial gradients dan pattern yang disokong.
- Clip paths dan image compositing asas.
- Pixel buffer yang boleh disambungkan kepada compositor.

Batasan:

- Ia bukan full Skia replacement.
- Text bukan fungsi terbina dalam; glyph perlu disediakan oleh text subsystem.
- Jangan anggap semua filter atau advanced blend mode tersedia.

NOVA perlu mengimplementasi effects yang tiada sebagai pass tersendiri atau menangguhkannya ke fasa selepas MVP.

### 4.2.1 Skia CPU sebagai parity candidate

`skia-safe` menyediakan CPU raster surfaces serta API untuk paths, paints, images, fonts, image filters dan text layout modules bergantung pada Cargo features. Oleh sebab output Remotion/Chrome turut menggunakan keluarga teknologi Skia untuk banyak operasi 2D, Skia CPU ialah calon yang wajar diuji untuk parity visual dan feature coverage. Ini **tidak menjamin** output sama dengan Chrome: font shaping, CSS semantics, compositing, color management dan implementation details masih perlu disamakan.

Proof-of-concept wajib menguji:

- Raster surface CPU tanpa GPU backend.
- Build/deployment pada Ubuntu 24.04 x86_64.
- Static binary/shared library requirements, build time dan binary size.
- Text shaping dan font fallback untuk Latin, Malay, CJK dan RTL sample jika diperlukan.
- Gradients, clipping, blend modes, shadows, blur dan image sampling.
- Single-frame throughput, 8/16 parallel surfaces, memory dan thread safety.

Jika Skia CPU memberikan visual parity yang jauh lebih baik dengan performance yang memadai, pilih sebagai production backend walaupun binary lebih besar. Jika build/footprint atau speed terlalu lemah, gunakan tiny-skia + cosmic-text dengan compatibility tests.

### 4.2.2 Vello CPU sebagai backend benchmark

Uji Vello CPU hanya selepas feature matrix dipetakan. Dokumentasi semasa menyatakan SIMD implementation dan CPU-only operation, tetapi complex filter graphs tidak disokong dan filter dalam multithreaded mode mempunyai batasan. Jangan jadikan Vello CPU satu-satunya backend untuk composition yang memerlukan blur/shadow kompleks sehingga limitations itu berubah dan tests lulus.

### 4.3 SVG: usvg + resvg

- `usvg` parse SVG kepada tree yang telah dinormalisasi.
- `resvg` render tree ke tiny-skia pixmap.
- Parse SVG sekali dan cache parsed tree.
- Render SVG pada target size/scale yang relevan.
- SVG script, external references dan external URL loading mesti dinyahaktifkan atau dikawal oleh asset resolver sendiri.
- SVG animation bukan sebahagian MVP; animasikan keseluruhan SVG sebagai layer NOVA.

### 4.4 Text: benchmark cosmic-text vs Skia textlayout

Mulakan proof-of-concept dengan `cosmic-text` + Swash kerana ia menyediakan font discovery/fallback, shaping, layout dan glyph rasterization. Pada masa yang sama, uji Skia textlayout/HarfBuzz apabila `skia-safe` dipilih supaya text metrics dan raster path tidak memerlukan banyak format conversion.

Keputusan akhir mesti berdasarkan:

- Font discovery dan deterministic fallback.
- Text shaping dan layout.
- Line wrapping, alignment, tracking/letter spacing dan line-height.
- Glyph rasterization dan antialiasing.
- Unicode: Latin, Malay diacritics, CJK, Arabic/RTL dan mixed-direction sample.
- Text measurement yang konsisten dengan output raster.
- Performance untuk static text, animated transform dan dynamic text.

NOVA perlu membina adapter backend-neutral untuk shaped runs/glyph placements. Cache key glyph mesti mengambil kira font identity, glyph ID, size, variation, hinting/raster mode, subpixel position dan scale. Cache layout berasingan daripada glyph bitmap cache.

**Keperluan ujian text:** Latin, nombor, punctuation, emoji fallback (bergantung font), Malay diacritics, Arabic/RTL dan mixed-direction text. Jangan dakwa sokongan universal sebelum test fonts dan shaping dilakukan.

### 4.5 Images: image crate

- Decode asset ketika compile/load, bukan pada setiap frame.
- Normalize orientation jika format memerlukannya.
- Simpan decoded image dalam cache yang mempunyai had RAM.
- Generate resized variants jika saiz target berulang dan transform statik.
- Tentukan polisi warna, alpha dan interpolation dengan jelas.
- Format awal: PNG, JPEG dan format yang diaktifkan secara eksplisit melalui crate features.

Animated GIF dan animated WebP tidak termasuk dalam MVP; ia boleh ditambah kemudian sebagai time-based image source.

### 4.6 Parallelism: Rayon

Rayon digunakan untuk data-parallel tasks dan worker pool khusus render. Frame rendering merupakan parallelism utama kerana setiap frame boleh dikira secara bebas apabila template deterministik.

Jangan buat Rayon global pool tanpa konfigurasi. Sediakan `ThreadPoolBuilder` dan tetapkan bilangan worker daripada config/runtime probe.

### 4.7 Serialization dan validation

- `serde` / `serde_json` untuk deserialize DSL ke typed structs.
- JSON Schema untuk validasi struktur input.
- Semantic validator Rust untuk rules yang JSON Schema tidak sesuai tangani, seperti asset references, duplicate IDs, frame range, unsupported feature dan memory limits.
- Jangan benarkan arbitrary code execution dari template.

### 4.8 Encoding: FFmpeg

MVP gunakan FFmpeg CLI melalui stdin raw-frame pipe kerana ia mengurangkan kompleksiti FFI pada awal projek. Engine menghantar frame dalam format yang ditetapkan, mengikut susunan, dan FFmpeg mengendalikan codec serta container.

Kemudian, jika profiling menunjukkan subprocess/pipe sebagai bottleneck atau perlu kawalan timestamp/audio yang lebih kompleks, pertimbangkan direct `libavcodec`/`libavformat` integration.

**Jangan bina H.264, H.265 atau AV1 encoder sendiri.**

---

## 5. Composition model dan coordinate system

### 5.1 Composition

Composition ialah dokumen visual yang mempunyai:

- `id`, `name` dan `version`.
- `width`, `height`.
- `fps` sebagai numerator/denominator.
- `duration_frames` atau duration yang ditukar kepada integer frame count.
- `background`.
- `layers`.
- `assets`.
- `metadata` pilihan.

### 5.2 Frame/time model

Gunakan integer frame index sebagai sumber utama kebenaran.

- `frame_index` bermula dari 0.
- Frame `n` dinilai pada masa `n * fps_den / fps_num` saat.
- Elakkan mengumpul floating-point delta setiap frame kerana ia boleh menyebabkan time drift.
- Simpan fps sebagai rational number, contohnya 30/1 atau 30000/1001.
- Property interpolation boleh menggunakan `f32`/`f64`, tetapi input time berasal daripada integer frame/tick.

### 5.3 Coordinate system

- Origin: kiri atas.
- X bertambah ke kanan.
- Y bertambah ke bawah.
- Unit: logical pixels pada composition resolution.
- Rotation: radian dalam internal representation; DSL boleh menerima darjah untuk ergonomik jika conversion jelas.
- Transform order perlu ditetapkan: translate, rotate, scale dan anchor/pivot; implementasi mesti konsisten dan diuji.

### 5.4 Pixel dan alpha policy

- Framebuffer internal MVP: RGBA8.
- Gunakan alpha representation yang konsisten dengan backend; tiny-skia menggunakan premultiplied alpha untuk pixmap data.
- Documentasikan bahawa data premultiplied tidak boleh dicampur dengan straight-alpha tanpa conversion.
- Untuk MP4 biasa, output akhir perlu opaque: composite semua layer ke background opaque sebelum dihantar ke encoder.
- Untuk format alpha-capable pada masa depan, tentukan format/pixel format encoder secara berasingan.
- Tetapkan polisi color space dan gamma. MVP perlu mempunyai output konsisten dan test snapshots; linear-light compositing boleh ditambah sebagai pilihan kualiti selepas baseline stabil.

---

## 6. Scene graph dan layer system

### 6.1 Node types MVP

| Type | Fungsi | MVP |
|---|---|---|
| `solid` | Background/solid colour | Ya |
| `rect` | Rectangle / rounded rectangle | Ya |
| `ellipse` | Circle / ellipse | Ya |
| `path` | Vector path | Ya |
| `text` | Typography | Ya |
| `image` | Raster image | Ya |
| `svg` | Static SVG asset | Ya |
| `group` | Parent node / nested layers | Ya |
| `line` | Stroke line | Boleh diwakili path |
| `precomp` | Composition nested | Fasa 2 |
| `video` | Video source layer | Bukan MVP |

### 6.2 Common layer properties

Setiap layer boleh mempunyai:

- `id` unik.
- `type`.
- `name` pilihan.
- `visible`.
- `opacity`.
- `blend_mode`.
- `transform`.
- `anchor`.
- `start_frame` dan `end_frame`.
- `z_index` atau order dalam array.
- `clip`/mask settings jika feature itu disokong.
- `animations`.

### 6.3 Scene compile

Semasa compile:

1. Validate semua ID unik.
2. Resolve semua asset URI kepada path dalam asset root.
3. Parse SVG dan vector paths.
4. Resolve font references.
5. Precompute keyframe ranges dan animation tracks.
6. Validate parent-child hierarchy dan circular references.
7. Kira bounding boxes konservatif untuk layer dan effects.
8. Bentuk immutable compiled scene yang boleh dibaca oleh workers.

Jangan mutate scene graph global sewaktu render frame. Frame worker perlu menerima state hasil evaluation yang immutable atau mempunyai local scratch data sendiri.

---

## 7. Timeline dan animation engine

### 7.1 Animation properties MVP

- `position.x`, `position.y`.
- `scale.x`, `scale.y`.
- `rotation`.
- `opacity`.
- `anchor.x`, `anchor.y` jika perlu.
- `fill.color`.
- `stroke.width`.
- `text.value` melalui data binding, bukan perlu animasi typewriter dahulu.
- `path`/morph animation bukan MVP.

### 7.2 Interpolation

Sediakan:

- `step`.
- `linear`.
- `ease_in_quad`, `ease_out_quad`, `ease_in_out_quad`.
- `ease_in_cubic`, `ease_out_cubic`, `ease_in_out_cubic`.
- Cubic Bezier easing dengan parameter yang tervalidasi.

Spring/physics animation boleh ditambah kemudian dengan hasil yang deterministik dan parameter yang jelas. Untuk MVP, elakkan integrasi simulation yang bergantung pada sejarah frame.

### 7.3 Keyframe evaluation

Animation track menyimpan keyframes tersusun mengikut frame. Untuk frame `n`:

1. Cari keyframe sebelum dan selepas `n`.
2. Jika di luar range, gunakan nilai hold/fill rule yang telah ditetapkan.
3. Normalize progress dalam range.
4. Apply easing.
5. Interpolate nilai property.
6. Gabungkan dengan base transform.

### 7.4 Determinism

- Frame 450 boleh dihasilkan tanpa menghasilkan frame 449.
- Semua animation value ditentukan daripada frame/time dan input template.
- Randomness mesti menerima explicit seed.
- Particle systems pada masa depan perlu menggunakan fixed timestep/seed dan random-access strategy; jangan masukkannya dalam MVP.

---

## 8. Template DSL (JSON)

### 8.1 Prinsip DSL

JSON DSL ialah interface utama untuk AI agent dan automation. DSL mesti:

- Mudah dijana oleh model.
- Typed dan tervalidasi.
- Tidak mengandungi executable code.
- Menyokong references kepada assets melalui ID.
- Menyokong data slots dan placeholders yang dibenarkan.
- Versioned supaya format boleh berkembang.

### 8.2 Contoh template MVP

```json
{
  "schema_version": "1.0",
  "composition": {
    "id": "product-ad-001",
    "name": "Product Advertisement",
    "width": 1920,
    "height": 1080,
    "fps": { "num": 30, "den": 1 },
    "duration_frames": 450,
    "background": "#101522"
  },
  "assets": [
    {
      "id": "product-photo",
      "type": "image",
      "src": "assets/product.png"
    }
  ],
  "layers": [
    {
      "id": "headline",
      "type": "text",
      "start_frame": 0,
      "end_frame": 450,
      "transform": {
        "position": { "x": 180, "y": 250 },
        "scale": { "x": 1, "y": 1 },
        "rotation": 0,
        "opacity": 1,
        "anchor": { "x": 0, "y": 0 }
      },
      "text": {
        "value": "CREATE SOMETHING BOLD",
        "font_family": "Inter",
        "font_size": 92,
        "font_weight": 700,
        "color": "#FFFFFF",
        "line_height": 1.05,
        "align": "left",
        "max_width": 1500
      },
      "animations": [
        {
          "property": "transform.position.y",
          "keyframes": [
            { "frame": 0, "value": 340, "easing": "ease_out_cubic" },
            { "frame": 18, "value": 250, "easing": "linear" }
          ]
        },
        {
          "property": "opacity",
          "keyframes": [
            { "frame": 0, "value": 0, "easing": "linear" },
            { "frame": 12, "value": 1, "easing": "linear" }
          ]
        }
      ]
    },
    {
      "id": "product-image",
      "type": "image",
      "asset": "product-photo",
      "start_frame": 0,
      "end_frame": 450,
      "fit": "contain",
      "transform": {
        "position": { "x": 1050, "y": 180 },
        "scale": { "x": 1, "y": 1 },
        "rotation": 0,
        "opacity": 1,
        "anchor": { "x": 0.5, "y": 0.5 }
      }
    }
  ]
}
```

### 8.3 Data binding

Template boleh mempunyai placeholder seperti `{{product.name}}` atau explicit slot object. Resolusi hanya dibenarkan melalui data object yang diberikan kepada render job.

Rules:

- Tiada expression evaluation.
- Tiada function call dalam placeholder.
- Tiada environment variable expansion dalam template.
- Tiada access kepada filesystem bebas.
- Missing required slot mesti menghasilkan validation error sebelum render.
- Escaping text dibuat secara automatik; data tidak dianggap markup.

### 8.4 Versioning

- Setiap template mesti mempunyai `schema_version`.
- Compiler mesti menolak version yang tidak disokong dengan error yang jelas.
- Migration tool boleh ditambah apabila DSL berubah.
- Template compiler tidak patut silently reinterpret property lama.

---

## 9. Rasterization dan rendering pipeline

### 9.1 Per-frame pipeline

Bagi setiap frame `n`:

1. Dapatkan compiled composition.
2. Evaluate semua animation tracks pada `n`.
3. Tentukan layer aktif berdasarkan `start_frame` / `end_frame`.
4. Resolve transform matrix dan opacity.
5. Kira layer order dan clip bounds.
6. Ambil asset daripada cache.
7. Rasterize setiap layer yang diperlukan.
8. Composite layer mengikut z-order.
9. Pastikan output framebuffer berada dalam format yang encoder jangka.
10. Serahkan frame dengan index `n` kepada ordered output stage.

### 9.2 Backend abstraction

Bina trait konseptual seperti:

```rust
trait RasterBackend: Send + Sync {
    fn create_surface(&self, width: u32, height: u32) -> Result<Surface>;
    fn draw_path(&self, surface: &mut Surface, path: &Path, paint: &Paint);
    fn draw_image(&self, surface: &mut Surface, image: &ImageAsset, options: &ImageDrawOptions);
    fn composite(&self, destination: &mut Surface, source: &Surface, options: &CompositeOptions);
    fn finish(&self, surface: Surface) -> Result<FrameBuffer>;
}
```

API sebenar boleh berubah mengikut borrow rules dan backend, tetapi sempadan abstraction mesti kekal. Jangan dedahkan `tiny_skia::Pixmap` ke `nova-core`.

### 9.3 Text render path

1. Parse text style.
2. Gunakan `cosmic-text` untuk shaping dan layout.
3. Dapatkan glyph positions dan glyph raster data.
4. Gunakan glyph cache.
5. Composite glyph mask pada lokasi yang sesuai.
6. Apply layer transform/opacity.

Jangan reshape text pada setiap frame jika content, font, width, size dan text style tidak berubah. Cache layout berasingan daripada rasterized glyph.

### 9.4 SVG render path

1. Load SVG bytes dari asset resolver.
2. Parse `usvg::Tree` sekali.
3. Cache parsed tree mengikut content hash dan parser options.
4. Render menggunakan `resvg` ke tiny-skia pixmap pada scale/target yang sesuai.
5. Simpan hasil raster jika saiz dan render options berulang.

SVG yang mempunyai filter berat perlu diuji. External image/data URI perlu melalui resolver dan size limits.

### 9.5 Image render path

- Decode sekali.
- Simpan original decoded data jika RAM mengizinkan.
- Buat resized cache variants untuk transform yang berulang.
- Gunakan nearest/linear/cubic sampling mengikut pilihan output.
- Tambah Lanczos atau high-quality downsampling selepas benchmark dan visual tests.

### 9.6 Blend modes dan effects

MVP:

- Normal/source-over.
- Opacity.
- Clip rectangle/path jika backend menyokong dengan baik.
- Solid fill, stroke dan gradients.

Fasa 2:

- Multiply, screen, overlay, darken/lighten.
- Drop shadow.
- Gaussian blur melalui separable passes.
- Mask alpha/luma.
- Motion blur pilihan.

Effects mesti mempunyai bounds expansion yang betul supaya clipping dan cache tidak memotong shadow/blur. Jangan implement effect kompleks sebelum reference tests tersedia.

---

## 9A. Performance-first frame pipeline

Bahagian ini menjadi requirement teras kerana sasaran produk ialah mengurangkan masa render 4-minit daripada baseline Remotion kira-kira 30 minit. Semua optimization mesti disokong profiler dan benchmark.

### 9A.1 Compile once, evaluate many frames

- Parse JSON, validate, resolve assets dan compile scene graph sekali bagi setiap render job.
- Parse SVG sekali; decode image sekali; resolve font sekali.
- Precompute frame ranges, keyframe segments, easing coefficients, static bounds dan transform metadata.
- Jangan allocate vectors/strings atau parse JSON di dalam hot path setiap frame.
- Timeline evaluator mesti menerima `frame_index`/rational timestamp dan memulangkan nilai property tanpa stateful playback.

### 9A.2 Content rasterization dipisahkan daripada frame compositing

Pisahkan setiap layer kepada:

1. **Content stage:** menghasilkan raster content (text glyphs, vector shape, image) yang boleh di-cache.
2. **Placement stage:** transform, opacity, crop dan clip yang boleh berubah setiap frame.
3. **Composite stage:** gabung layer ke final frame.

Contoh: logo statik yang hanya bergerak/scale tidak perlu dibentuk semula daripada SVG pada setiap frame. Rasterize pada resolution/scale bucket yang sesuai sekali, kemudian transform dan composite. Untuk scale animation, cache beberapa resolution bucket atau gunakan high-quality resampling supaya tidak blur.

### 9A.3 Static-span / hold-frame reuse

Frame planner mesti mengesan frame range yang visual outputnya sama. Jika semua layer dan effect tidak berubah, render frame sekali dan reuse pixel buffer/reference untuk frame seterusnya; encoder masih menerima frame mengikut timestamp/FPS. Jangan duplicate buffer bytes tanpa perlu jika FFmpeg pipeline boleh menerima frame buffer yang sama secara read-only.

Cache validity perlu mengambil kira parent transforms, masks, effects, time-dependent expressions, animated assets, random seeds dan color profile. Jika mana-mana dependency berubah, invalidate cache.

### 9A.4 Avoid full-frame work where possible

- Track layer bounds dan clip regions.
- Skip layer yang bounds berada di luar viewport.
- Skip fully transparent/invisible layers.
- Use dirty-region/tile rendering hanya selepas backend menyokong clipping/partial surface dengan betul dan profiling membuktikan manfaat.
- Effects seperti blur/shadow memerlukan bounds expansion; jangan crop halos.
- Jangan guna tile rendering secara default jika ia meningkatkan compositing/copy overhead.

### 9A.5 Image and text hot paths

- Decode source image once dan cache by content hash.
- Precompute crop/resize variants yang benar-benar digunakan.
- Avoid repeated image resampling for same dimensions/filter.
- Shape text only when content/font/style/width changes.
- Cache glyph masks/bitmaps by font identity, glyph ID, size, variation, hinting, subpixel position dan raster backend.
- Precompute static text layout; animated position/opacity tidak patut trigger re-layout.
- Keep font files deterministic and bundled/registered per project to avoid host font mismatch.

### 9A.6 SIMD dan CPU dispatch

- Profile first; identify blend, pixel conversion, blur, resampling atau path rasterization hot loops.
- Implement scalar reference path first.
- Add AVX2 path only if runtime CPU detection confirms support; add portable fallback.
- Use `std::is_x86_feature_detected!` / target-feature functions carefully.
- Avoid unsafe SIMD until tests cover alignment, stride, tails and alpha correctness.
- Benchmark per operation and end-to-end; microbenchmark wins may not improve total render.

### 9A.7 Parallel scheduling

- Frame-level parallelism ialah laluan utama kerana frame bersifat random-access.
- Use dynamic work stealing and small chunks to balance variable frame cost.
- Use bounded in-flight frames and ordered output reorder buffer.
- One global CPU budget governs Rayon and FFmpeg threads.
- Tile-level parallelism hanya sebagai pilihan backend jika satu frame terlalu mahal dan frame-level parallelism tidak cukup.
- Ensure per-worker raster surfaces/contexts are not shared unsafely.

### 9A.8 Encoder pipeline

- Measure render-only vs encode-only time first.
- Use bounded producer/consumer queue; encoder and renderer overlap only while preserving ordered timestamps.
- Avoid an unnecessary PNG/JPEG sequence on disk for normal output; stream raw frames through pipe or libavcodec integration.
- For opaque MP4, convert/composite to encoder-compatible pixel format once at a defined stage.
- Benchmark `libx264` presets such as `veryfast`, `faster`, `fast` and `medium` at matched quality (CRF/quality metric), not merely same CRF without checking quality.
- If encoding dominates, faster preset or hardware-free codec choice may help; do not sacrifice visual quality silently.
- Audio muxing and `+faststart` must be measured separately from video frame rendering.

## 10. Caching strategy

### 10.1 Jenis cache

1. **Asset decode cache** — decoded raster image.
2. **SVG parse cache** — parsed usvg tree.
3. **Font/layout cache** — font resolution dan shaped layout.
4. **Glyph cache** — rasterized glyph masks.
5. **Static layer cache** — raster result untuk layer statik.
6. **Scaled image cache** — asset yang telah di-resize.
7. **Compiled template cache** — normalized scene graph.

### 10.2 Cache keys

Gunakan content hash + rendering-relevant parameters. Contoh static layer key:

- Asset/content hash.
- Layer configuration hash.
- Composition resolution.
- Scale/resolution bucket.
- Raster backend/version.
- Color/alpha settings.
- Font identity jika layer text.

Jangan cache berdasarkan nama fail semata-mata kerana fail boleh berubah tanpa nama berubah.

### 10.3 Invalidation

- Template changed → invalidate compiled template dan dependent layer caches.
- Asset content changed → invalidate asset decode dan dependent layers.
- Font changed → invalidate font/layout/glyph caches.
- Resolution changed → invalidate rasterized layer variants.
- Backend version/options changed → invalidate rendered output cache.

### 10.4 Memory budget

Cache mesti ada hard limit dan eviction policy (LRU atau size-aware). Catat cache hit/miss, bytes stored, evictions dan decode time.

### 10.5 Static layer optimization

Layer hanya boleh dianggap static jika semua property visual, asset, parent transform, effects dan clipping tidak berubah sepanjang range yang dicache. Jika parent animated, layer child mungkin tidak perlu rerasterize content tetapi masih perlu transform/composite setiap frame.

Pisahkan konsep:

- **Raster cache:** content layer diraster sekali.
- **Transform cache:** transform metadata sahaja.
- **Composite cache:** final layer stack untuk range frame tertentu; lebih mahal dan hanya untuk composition yang betul-betul statik.

---

## 11. Parallel rendering dan scheduler

### 11.1 Strategi utama: frame-level parallelism

Frame pada indeks berbeza adalah bebas jika:

- Scene graph immutable.
- Animation evaluation bersifat random-access.
- Randomness seeded.
- Tiada effect yang bergantung pada previous frame.

Oleh itu, NOVA akan parallelize frame terlebih dahulu. Tile-level parallelism boleh dikaji kemudian.

### 11.2 Dynamic scheduling

Jangan tetapkan semua frame kepada chunk statik yang besar tanpa mengukur. Frame dengan banyak blur, SVG atau text boleh lebih berat daripada frame kosong.

Gunakan dynamic work distribution dengan chunk kecil/sederhana dan work stealing. Benchmark chunk sizes, contohnya 1, 2, 4, 8 dan 16 frames.

### 11.3 Ordered output

Encoder memerlukan frame sequence yang betul. Worker boleh siap dalam apa-apa urutan, tetapi output writer mesti menghantar frame 0, 1, 2, ... mengikut urutan.

Gunakan:

- `frame_index` untuk setiap result.
- bounded result channel.
- reorder buffer dengan had maksimum.
- sliding in-flight window supaya frame yang siap terlalu awal tidak memenuhi RAM sementara menunggu frame lebih lambat.
- cancellation propagation jika worker atau encoder gagal.

### 11.4 Elakkan nested parallelism

Jangan pada masa yang sama:

- Rayon 16 threads untuk frame rendering,
- setiap image filter melancarkan 16 threads,
- FFmpeg encoder menggunakan banyak threads,
- dan beberapa jobs berjalan serentak.

Tetapkan CPU budget global. Untuk MVP, satu render job menggunakan pool yang terhad; FFmpeg threads juga dikonfigurasi secara eksplisit.

### 11.5 Job concurrency

Fasa awal:

- Satu job aktif.
- Parallel frame rendering dalam job tersebut.

Selepas stabil:

- Beberapa job boleh berjalan.
- Scheduler mengagihkan thread budget antara job.
- Elakkan 4 jobs × 16 workers pada VPS 16 vCPU.
- Prioritization dan fair scheduling boleh ditambah kemudian.

---

## 12. Frame buffer dan memory management

### 12.1 Buffer pool

- Preallocate framebuffer berdasarkan resolusi dan worker count.
- Reuse buffers selepas encoder selesai membacanya.
- Jangan clone buffer penuh tanpa keperluan.
- Gunakan ownership transfer atau pool handles.
- Bounded queue memberi backpressure kepada renderer.

### 12.2 Output frame contract

Setiap frame output mesti menyatakan:

- `frame_index`.
- `width`, `height`.
- `pixel_format`.
- `stride`.
- `alpha_mode`.
- buffer bytes.

Encoder mesti validate metadata ini sebelum menerima data.

### 12.3 Failure behavior

- Worker panic/error tidak boleh menyebabkan deadlock queue.
- Semua sender mesti ditutup apabila job dibatalkan.
- Encoder exit status mesti diperiksa.
- Temporary files dibersihkan secara selamat.
- Partial output tidak ditandakan sebagai completed.

---

## 13. FFmpeg video pipeline

### 13.1 MVP pipeline

1. NOVA render RGBA frame.
2. NOVA composite ke background opaque untuk output MP4.
3. Ordered writer menghantar frame bytes ke FFmpeg stdin.
4. FFmpeg membaca rawvideo dengan pixel format, dimensions dan frame rate yang dinyatakan.
5. FFmpeg encode H.264 (libx264 jika build FFmpeg menyokongnya) dan mux MP4.
6. NOVA tunggu process exit, validate output dengan ffprobe dan publish fail.

Contoh arahan konsep:

```bash
ffmpeg \
  -hide_banner -loglevel error -y \
  -f rawvideo \
  -pixel_format rgba \
  -video_size 1920x1080 \
  -framerate 30 \
  -i pipe:0 \
  -an \
  -c:v libx264 \
  -preset medium \
  -crf 20 \
  -pix_fmt yuv420p \
  -movflags +faststart \
  output.mp4
```

Arahan akhir perlu disahkan dengan versi FFmpeg yang dipasang. Jika input buffer bukan straight RGBA atau frame mengandungi alpha, lakukan compositing/conversion sebelum pipe.

### 13.2 Encoding profile

Sediakan preset profile:

- `preview`: cepat, saiz fail tidak menjadi keutamaan.
- `balanced`: kualiti/kelajuan seimbang.
- `high_quality`: CRF lebih rendah dan preset lebih berat.

Profile ialah konfigurasi, bukan nilai hard-coded. Simpan semua parameter dalam job metadata supaya output boleh dihasilkan semula.

### 13.3 Audio

Audio bukan keperluan core MVP tetapi video output boleh memerlukan soundtrack. Fasa 2 boleh menambah:

- Audio asset import.
- Trim dan offset.
- Volume envelope/fade.
- Basic mix.
- Mux ke MP4 menggunakan FFmpeg.

Jangan campur clock audio dengan frame clock secara tidak jelas. Gunakan rational timestamps dan dokumentasikan resampling/timebase.

---

## 14. CLI dan public interface

### 14.1 CLI commands

Cadangan:

```bash
nova validate ./project/template.json
nova inspect ./project/template.json
nova preview ./project/template.json --frame 120 --output ./preview.png
nova render ./project/template.json --output ./output.mp4
nova render ./project/template.json --data ./data/job.json --profile balanced
nova benchmark ./benchmarks/basic.json --threads 1,2,4,8,16
nova doctor
```

### 14.2 `nova doctor`

Memaparkan:

- Rust/NOVA build version.
- OS dan architecture.
- CPU model jika boleh diperoleh.
- available logical CPUs.
- CPU flags (AVX2 dan lain-lain) melalui runtime detection.
- RAM total/available.
- FFmpeg version dan encoder availability.
- Font directories.
- temp/output directory access.

Jangan assume AVX2 hanya kerana 16 vCPU. Gunakan runtime detection dan fallback scalar/SSE/NEON yang selamat.

### 14.3 API kemudian

REST API dicadangkan selepas CLI stabil:

- `POST /v1/renders` — create render job.
- `GET /v1/renders/{id}` — status/progress.
- `POST /v1/renders/{id}/cancel` — cancel.
- `GET /v1/renders/{id}/output` — get output metadata or signed download link.
- `POST /v1/templates/validate` — validate template.

API tidak render terus pada request thread. Ia enqueue job dan pulangkan job ID.

---

## 15. Project structure

```text
nova/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── plan.md
├── docs/
│   ├── architecture.md
│   ├── template-schema.md
│   ├── animation.md
│   ├── rendering-backends.md
│   ├── performance.md
│   └── security.md
├── crates/
│   ├── nova-core/
│   │   ├── src/composition.rs
│   │   ├── src/scene.rs
│   │   ├── src/layer.rs
│   │   ├── src/geometry.rs
│   │   ├── src/color.rs
│   │   └── src/time.rs
│   ├── nova-template/
│   │   ├── src/schema.rs
│   │   ├── src/parser.rs
│   │   ├── src/validator.rs
│   │   ├── src/compiler.rs
│   │   └── src/data_binding.rs
│   ├── nova-animation/
│   │   ├── src/timeline.rs
│   │   ├── src/keyframe.rs
│   │   ├── src/easing.rs
│   │   └── src/evaluator.rs
│   ├── nova-raster/
│   │   ├── src/backend.rs
│   │   ├── src/skia_cpu_backend.rs
│   │   ├── src/tiny_skia_backend.rs
│   │   ├── src/vello_cpu_backend.rs
│   │   ├── src/surface.rs
│   │   └── src/paint.rs
│   ├── nova-text/
│   ├── nova-svg/
│   ├── nova-image/
│   ├── nova-compositor/
│   ├── nova-cache/
│   ├── nova-scheduler/
│   ├── nova-encoder/
│   ├── nova-cli/
│   ├── nova-api/              # fasa API
│   └── nova-bench/
├── schemas/
│   └── template-v1.schema.json
├── examples/
│   ├── typography/
│   ├── product-ad/
│   └── transitions/
├── benchmarks/
│   ├── basic-1080p.json
│   ├── image-heavy-1080p.json
│   ├── complex-1080p.json
│   └── bulk-jobs/
├── tests/
│   ├── golden/
│   ├── fixtures/
│   └── integration/
└── scripts/
    ├── setup-ubuntu.sh
    └── run-benchmarks.sh
```

Struktur boleh berubah semasa pembangunan, tetapi crate boundaries perlu kekal jelas. Elakkan memecahkan setiap fail kecil menjadi crate berasingan jika belum memberi manfaat.

---

## 16. Security dan reliability

Template dan assets boleh datang daripada AI agent atau pengguna. Anggap semua input sebagai untrusted.

### 16.1 Filesystem

- Asset path mesti berada di bawah project asset root.
- Tolak `../`, absolute paths dan symlink escape yang keluar daripada root.
- Jangan benarkan arbitrary file read.
- Output path mesti berada di output workspace yang ditetapkan.

### 16.2 SVG

- Disable network/external URL fetch secara default.
- Reject atau sanitize unsupported external references.
- Tetapkan had jumlah nodes, path complexity, dimensions dan decoded assets.
- Elakkan SVG parser daripada membaca fail sistem.

### 16.3 Resource limits

Tetapkan limit untuk:

- Max width/height.
- Max total pixels.
- Max duration frames.
- Max layers/nesting depth.
- Max path commands.
- Max asset file size.
- Max decoded image memory.
- Max render duration/job.
- Max temp disk usage.

Nilai limit perlu configurable dan diuji berdasarkan hardware; default mesti menghalang input yang boleh menghabiskan RAM/disk.

### 16.4 Process execution

- FFmpeg dipanggil menggunakan argument array, bukan shell string yang dibina daripada input pengguna.
- Jangan interpolate user input ke shell.
- Capture stderr secara bounded.
- Timeout dan cancellation mesti disokong.
- Verify exit code dan output file sebelum job completed.

### 16.5 Failure recovery

- Job state: queued, validating, compiling, rendering, encoding, completed, failed, cancelled.
- Simpan error code dan diagnostics.
- Retry hanya untuk error transient, bukan template invalid.
- Temporary output menggunakan nama unik dan atomic rename apabila selesai.

---

## 17. Testing strategy

### 17.1 Unit tests

- Time/frame conversion.
- Easing functions.
- Keyframe lookup/interpolation.
- Transform matrix composition.
- Color parsing.
- Alpha compositing.
- Asset resolver path safety.
- Scene graph validation.
- Layer active-range evaluation.
- Cache key and invalidation.

### 17.2 Golden image tests

Setiap test fixture mempunyai expected PNG. Render pada backend/version yang dipin dan bandingkan:

- Exact pixel equality untuk operasi deterministic yang sepatutnya tepat.
- Pixel tolerance/SSIM untuk operasi yang mempunyai rounding atau rasterization variation.
- Fail test jika visual difference melepasi threshold.

Golden images perlu diperiksa manusia apabila berubah; jangan auto-update semua snapshots tanpa review.

### 17.3 Integration tests

- JSON → PNG preview.
- JSON → MP4.
- Text dan font fallback.
- SVG dengan gradients/clips.
- Transparency compositing.
- Out-of-order worker completion tetapi ordered video output.
- Encoder failure.
- Cancellation semasa rendering.
- Asset missing dan corrupt.
- Memory limit handling.

### 17.4 Determinism tests

Render composition yang sama beberapa kali dengan worker count berbeza. Frame output sepatutnya sama atau berada dalam tolerance yang ditetapkan. Jika backend menghasilkan perbezaan floating-point, dokumentasikan toleransi dan punca.

---

## 18. Benchmark plan

**Jangan menetapkan dakwaan kelajuan sebelum benchmark sebenar dijalankan.** NOVA perlu mempunyai dua benchmark modes: render-only dan end-to-end.

### 18.1A Remotion baseline capture — wajib sebelum NOVA performance claim

Bina satu folder benchmark immutable yang mengandungi:

- Source Remotion project dan lockfile.
- Composition ID dan props JSON.
- Semua asset/font yang digunakan (hash SHA-256).
- Remotion version, Node version, Chrome version dan FFmpeg version.
- Output reference MP4 dan PNG reference frames.
- Log verbose, `npx remotion benchmark` result dan system telemetry.
- Exact command/config untuk menghasilkan render 30 minit yang dilaporkan.

Rekod stage timings secara berasingan:

| Stage | Metrik |
|---|---|
| Bundle/startup | seconds |
| Asset preparation/download | seconds, bytes |
| Frame rendering | seconds, rendered FPS |
| Media decoding | seconds, cache hit rate |
| Encoding/muxing | seconds, encoder FPS |
| Total wall-clock | seconds |

Jika render 30 minit itu kebanyakannya encoding atau asset waiting, membina rasterizer baru mungkin tidak memberikan improvement besar. Jika frame render dominan, native renderer berpotensi memberi manfaat lebih tinggi.

### 18.1B Workload sasaran 4-minit

Untuk 4 minit pada 30 FPS: 7,200 frames. Uji 24 FPS dan 60 FPS hanya jika itu benar-benar format output yang digunakan. Benchmark utama wajib menggunakan FPS production sebenar.

| Scenario | Composition | Apa yang diuji |
|---|---|---|
| P1 — Text-heavy ad | 20–40 text layers, animated positions/opacity, fonts, tracking | shaping/layout cache, glyph cache |
| P2 — Brand/product ad | 10–20 image layers, crops, transforms, masks, shadows | image decode, resize cache, compositor |
| P3 — Template transition | 5–15 scenes, wipes, fades, scale/blur transitions | timeline, transition effects, static span reuse |
| P4 — Mixed Remotion clone | replicate actual 4-minute composition | end-to-end comparison |
| P5 — Long static spans | static background/brand panels with short animated regions | frame reuse, layer caching, static-span optimization |

### 18.1C Backend bake-off

Implement the same small scene set in:

1. Skia CPU (`skia-safe`).
2. tiny-skia + cosmic-text/resvg.
3. Vello CPU where required features are supported.

Compare render-only throughput, output quality, build/deploy friction, memory and parallel scaling. Choose backend per feature if necessary, but avoid copying pixels between backends in hot paths. Production backend decision must be documented with data.

### 18.2 Test scenes

| Test | Workload | Tujuan |
|---|---|---|
| A — Shapes | 2D shapes, paths, fills, gradients, transforms | Rasterizer throughput |
| B — Typography | Banyak text layers, font sizes, line wrapping, animated positions | Text shaping/cache/glyph performance |
| C — Image-heavy | 10–20 image layers, scale/rotate/crop | Decode/cache/compositing |
| D — SVG-heavy | SVG logos, icons, paths, gradients | SVG parse/cache/render |
| E — Mixed ad | Text + images + SVG + transitions + shadows jika disokong | Realistic template |
| F — Bulk | 10, 25 dan 50 job variations | Throughput dan memory control |

### 18.3 Standard test profile

- 1920×1080.
- 30 FPS.
- Durations: 5s (smoke), 15s (medium), 30s (main test).
- 4K test selepas 1080p stabil.
- Same assets and scene semantics untuk backend comparisons.
- Warm-up sekali; sekurang-kurangnya 5 measured runs.
- Laporkan median dan p95, bukan hanya best run.

### 18.4 Thread scaling

Jalankan setiap test pada:

- 1 thread.
- 2 threads.
- 4 threads.
- 8 threads.
- 12 threads.
- 16 threads.

Rekod speedup relative to 1 thread, CPU utilization, throughput dan memory. Jangan assume 16 threads paling pantas jika VPS provider mempunyai CPU quota.

### 18.5 Metrics wajib

- Total render time.
- Render-only time (tanpa encoding).
- Encoding time.
- Frames per second (render throughput, bukan playback FPS).
- Per-frame latency p50/p95/p99.
- Average/peak RSS.
- CPU usage dan CPU time.
- Cache hits/misses.
- Bytes written/read.
- Encoder preset/codec/CRF.
- Output file size.
- Visual quality metrics.

### 18.6 Benchmark fairness

Jika membandingkan NOVA dengan Remotion/HyperFrames/Raea:

- Gunakan resolution, FPS, duration dan visual workload yang setara.
- Asingkan masa startup daripada render time.
- Pastikan semua output encode menggunakan codec/preset/CRF yang setara, atau bandingkan render-only PNG frames.
- Jalankan pada host dan masa yang sama jika boleh.
- Jangan campur benchmark CPU-only dengan GPU rendering.
- Catat versi engine, library, FFmpeg, OS dan CPU.

### 18.7 Benchmark report template

| Renderer / backend | Scene | Threads | Render sec | FPS | Peak RAM | Encode sec | Output MB |
|---|---|---:|---:|---:|---:|---:|---:|
| NOVA Skia CPU | A | 1 | TBD | TBD | TBD | TBD | TBD |
| NOVA Skia CPU | A | 8 | TBD | TBD | TBD | TBD | TBD |
| NOVA tiny-skia | A | 8 | TBD | TBD | TBD | TBD | TBD |
| NOVA vello_cpu | A | 8 | TBD | TBD | TBD | TBD | TBD |
| Remotion | A | n/a | TBD | TBD | TBD | TBD | TBD |
| HyperFrames | A | n/a | TBD | TBD | TBD | TBD | TBD |

### 18.8 Provisional performance objectives

Berdasarkan baseline pengguna (4 minit video mengambil kira-kira 30 minit pada Remotion), sasaran berikut digunakan hanya jika video itu 30 FPS dan workload boleh diulang:

| Gate | Target total time | Effective throughput | Keperluan |
|---|---:|---:|---|
| Baseline | 30 min | 4 FPS | Remotion tuned reference |
| NOVA minimum win | ≤24 min | ≥5 FPS | Sekurang-kurangnya 1.25× faster |
| NOVA v1 product target | ≤15 min | ≥8 FPS | Sekurang-kurangnya 2× faster |
| Aggressive target | ≤6 min | ≥20 FPS | Sekurang-kurangnya 5× faster |
| Stretch | ≤4 min | ≥30 FPS | Real-time render throughput |

Acceptance rules:

- Gunakan median sekurang-kurangnya 5 runs; laporkan p95 jika ada variasi besar.
- NOVA mesti melepasi visual acceptance untuk scene yang sama.
- Bandingkan total wall-clock dan render-only secara berasingan.
- Jika encode menjadi bottleneck, optimize encoding pipeline/preset secara berasingan dan kekalkan output quality yang setara.
- Jika Remotion baseline selepas tuning menjadi lebih laju, recalibrate targets terhadap baseline baharu.
- Jangan claim 2×/5× lebih laju daripada Remotion sehingga semua gates di atas disahkan.

### 18.9 A/B test concurrency dan pipeline

Uji sekurang-kurangnya:

- Rayon threads: 1, 2, 4, 6, 8, 10, 12, 14, 16.
- In-flight frame window: 1×, 2× dan 3× worker count.
- Chunk sizes: 1, 2, 4, 8 dan 16 frames.
- FFmpeg threads: 1, 2, 4 dan auto; elakkan encoder mengambil semua CPU ketika renderer aktif.
- Render-to-encoder: synchronous baseline vs bounded producer/consumer.
- Cache disabled vs warm cache vs cold cache.
- Single job vs controlled multi-job; jangan campur kedua-dua keputusan.

Simpan CPU utilization, CPU steal time, RSS, memory bandwidth proxy jika tersedia, context switches, disk I/O dan throttling/quota. VPS vCPU mungkin berkongsi host; keputusan mesti diulang pada masa berlainan jika CPU steal tinggi.

### 18.10 Output quality gates

Untuk setiap test scene:

- Sama resolution, FPS, duration, fonts, source assets dan color space.
- Compare PNG frame output sebelum video encoding.
- Compare MP4 selepas encoding dengan codec, profile, preset dan CRF yang sama.
- Simpan frame diff dan contact sheet.
- Lulus visual review bagi text, gradients, shadow edges, alpha, crop dan transition timing.
- Pastikan audio duration, start offset dan sync tidak berubah.

Performance gain yang menyebabkan typography blur, gradient banding, incorrect alpha, missing glyph atau timing drift dianggap gagal.

---

## 19. Deployment pada Ubuntu 24.04

### 19.1 Build

- Install stable Rust toolchain.
- Install FFmpeg runtime dan pastikan encoder yang diperlukan tersedia.
- Build release mode.
- Simpan binary, configs, templates dan assets secara berasingan.
- Pin package/library versions untuk production image.

### 19.2 Service model

MVP boleh dijalankan sebagai CLI worker. Selepas API ditambah:

- `nova-api` untuk REST endpoints.
- `nova-worker` untuk render jobs.
- Redis untuk queue jika worker/jobs lebih daripada satu process.
- PostgreSQL untuk persistent job metadata jika diperlukan.
- Storage directory atau object storage untuk outputs.

Jangan masukkan Redis/PostgreSQL pada hari pertama jika single CLI process sudah memenuhi MVP. Tambah infra apabila job workflow memerlukannya.

### 19.3 Observability

Log structured JSON dengan:

- job_id.
- template_id.
- composition resolution/fps/duration.
- compile_ms.
- render_ms.
- encode_ms.
- frame_count.
- average_fps.
- peak_rss_bytes.
- cache hit/miss counts.
- error_code.

Metrik kemudian boleh dieksport ke Prometheus/Grafana.

---

## 20. Roadmap pembangunan

### Phase 0 — Repository, Remotion baseline dan backend bake-off

**Deliverables:**
- Rust workspace dan CLI skeleton.
- `nova doctor` dengan CPU flags, memory dan CPU quota/steal telemetry jika tersedia.
- Exact Remotion project baseline, tuned concurrency dan verbose timing logs.
- Benchmark harness render-only/end-to-end.
- Backend bake-off POC: Skia CPU, tiny-skia dan Vello CPU.
- Visual reference frames dan asset/font hashes.
- Setup instructions Ubuntu.

**Exit criteria:** bottleneck Remotion dikenal pasti; backend MVP dipilih berdasarkan parity, speed, memory dan build cost. Jangan mula mengoptimumkan tanpa baseline.

### Phase 1 — Static 2D renderer

**Deliverables:**
- RGBA framebuffer.
- tiny-skia backend.
- Solid, rect, ellipse, path.
- Transform dan source-over compositing.
- PNG output.

**Exit criteria:** golden image tests lulus untuk shapes dan basic paths.

### Phase 2 — Template compiler dan scene graph

**Deliverables:**
- JSON DSL v1.
- Serde structs.
- JSON Schema.
- Semantic validation.
- Asset resolver.
- Scene graph compile.

**Exit criteria:** template invalid ditolak sebelum rasterization; valid template menghasilkan PNG.

### Phase 3 — Timeline dan animation

**Deliverables:**
- Frame-based timeline.
- Keyframes.
- Easing.
- Transform/opacity animation.
- Deterministic frame selection.

**Exit criteria:** preview frame arbitrary index menghasilkan keputusan yang konsisten.

### Phase 4 — Text, image dan SVG

**Deliverables:**
- cosmic-text integration.
- Font fallback and glyph cache.
- Image decoding/cache.
- usvg/resvg integration.
- Layout/alignment/wrapping.

**Exit criteria:** typography, image dan SVG fixtures lulus visual tests.

### Phase 5 — Video encoding

**Deliverables:**
- Ordered frame queue.
- FFmpeg rawvideo pipe.
- MP4 profile.
- Progress reporting.
- Error handling/cancellation.

**Exit criteria:** video 1080p/30 FPS boleh dihasilkan dan metadata disahkan dengan ffprobe.

### Phase 6 — Performance engineering (critical path)

**Deliverables:**
- Compile-once scene graph and precomputed timeline.
- Rayon thread pool with measured 1–16 thread scaling.
- Dynamic frame scheduler and bounded in-flight window.
- Buffer pool and ordered encoder pipeline.
- Asset decode, glyph, text layout, SVG parse dan static-layer caches.
- Static-span/hold-frame reuse.
- Skip invisible/offscreen layers and avoid redundant full-frame copies.
- SIMD hotspots selepas profiling dan scalar fallback.
- Encoder preset/thread tuning.
- Per-stage profiling reports and visual parity reports.

**Exit criteria:** median end-to-end time beats tuned Remotion baseline by at least 1.25× on the production reference composition while passing visual acceptance; 2× is the v1 product target. RAM remains bounded.

### Phase 7 — Production service

**Deliverables:**
- Axum API.
- Job state machine.
- Redis queue jika diperlukan.
- Persistent metadata.
- Storage/output management.
- Monitoring.

**Exit criteria:** job boleh dihantar, dipantau, dibatalkan dan diselesaikan tanpa blocking API.

---

## 21. Risiko dan mitigasi

| Risiko | Kesan | Mitigasi |
|---|---|---|
| CPU VPS tidak konsisten | Benchmark berubah-ubah | `nova doctor`, CPU quota/steal monitoring, repeated runs |
| Text rendering kompleks | Glyph/layout bug | cosmic-text adapter, Unicode tests, font fixtures |
| Memory pressure | OOM | Bounded queue, pool, cache limits, max dimensions |
| Encoder jadi bottleneck | Worker menunggu | Separate render/encode metrics, tune FFmpeg, bounded backpressure |
| Library API berubah | Build/regression | Pin versions, lockfile, backend abstraction |
| SVG input berbahaya/terlalu berat | Security/DoS | Disable network references, parser limits, asset sandbox |
| Multithreading tidak scale | Throughput rendah | Profile, tune chunk size, avoid nested pools |
| Color/alpha inconsistency | Visual artifacts | Explicit alpha contract, golden images, conversion tests |
| Cache invalidation salah | Output salah | Content-hash keys dan tests untuk dependency changes |
| Scope creep | MVP tertangguh | 2D-only roadmap dan feature gates |

---

## 22. Keputusan yang perlu dikekalkan sepanjang pembangunan

1. NOVA kekal 2D-only untuk roadmap ini.
2. Tiada Chromium, DOM, HTML rendering atau browser screenshots.
3. Engine core tidak bergantung pada backend raster tertentu.
4. Skia CPU, tiny-skia dan Vello CPU mesti melalui backend bake-off; jangan lock backend sebelum kualiti/performance test. Skia CPU ialah parity candidate, tiny-skia lightweight candidate, Vello CPU experimental candidate.
5. Timeline dan composition bersifat deterministic/random-access.
6. Frame rendering parallel, output encoding ordered.
7. Memory bounded ialah keperluan seni bina, bukan optimization kemudian.
8. FFmpeg bertanggungjawab untuk codecs dan muxing.
9. Template JSON tidak menjalankan arbitrary code.
10. Benchmark mesti memisahkan render-only daripada encoding.
11. Remotion baseline mesti ditune dahulu menggunakan benchmark rasmi dan verbose timings; perbandingan NOVA mesti setara dan repeatable.
12. KPI utama ialah end-to-end render time dengan visual parity; target awal 2× faster, 5× aggressive, real-time stretch untuk workload yang sesuai.
13. Semua performance claims mesti berdasarkan median sekurang-kurangnya lima runs dan report render-only serta encode-only.
14. Semua optimizations mesti mempunyai benchmark dan visual regression tests.

---

## 23. Definition of Done untuk versi 1.0

NOVA 1.0 boleh dianggap selesai apabila semua perkara berikut dipenuhi:

- [ ] Rust workspace dan release build stabil.
- [ ] `nova validate`, `nova inspect`, `nova preview`, `nova render`, `nova benchmark`, `nova doctor` tersedia.
- [ ] JSON DSL v1 didokumentasikan dan tervalidasi.
- [ ] Shapes, paths, text, images, SVG dan groups disokong.
- [ ] Transform, opacity, keyframes dan easing disokong.
- [ ] PNG preview dan MP4 export berfungsi.
- [ ] FFmpeg process failures dikesan dengan betul.
- [ ] Frame order betul walaupun worker selesai di luar susunan.
- [ ] Memory dan queue mempunyai had yang boleh dikonfigurasi.
- [ ] Unit, integration dan golden tests tersedia.
- [ ] Benchmark 1/2/4/8/12/16 threads direkodkan.
- [ ] Output deterministic diuji dengan thread count berbeza.
- [ ] Security limits untuk paths, SVG dan assets dilaksanakan.
- [ ] Tiada runtime Chromium atau GPU dependency.
- [ ] README menerangkan install, render, template format dan troubleshooting.

---

## 24. Rujukan teknikal rasmi

Rujukan berikut digunakan untuk menyemak kesesuaian library dan API. Versi dependency sebenar hendaklah dipin ketika implementation bermula; jangan anggap nombor versi dalam dokumen ini sebagai arahan untuk sentiasa menggunakan `latest`.

### Rendering dan vector
- [skia-safe — docs.rs](https://docs.rs/skia-safe/latest/skia_safe/) — Rust bindings to Skia; CPU raster rendering is the default configuration and GPU modules are optional.
- [Rust Skia surface documentation](https://rust-skia.github.io/doc/skia_safe/surface/type.Surface.html) — CPU-memory raster surface versus GPU render target.

- [tiny-skia — docs.rs](https://docs.rs/tiny-skia/latest/tiny_skia/) — CPU 2D rasterizer; paths, gradients, strokes, clipping dan image blending.
- [tiny-skia — repository](https://github.com/linebender/tiny-skia)
- [Vello CPU — docs.rs](https://docs.rs/vello_cpu/latest/vello_cpu/) — CPU-only 2D renderer dan pilihan backend eksperimen; semak limitations sebelum aktifkan multithreaded filters.
- [Vello — repository](https://github.com/linebender/vello)
- [resvg — docs.rs](https://docs.rs/resvg/latest/resvg/) — SVG rendering ke tiny-skia pixmap.
- [usvg — docs.rs](https://docs.rs/usvg/latest/usvg/) — SVG parsing dan normalized tree.

### Typography dan images

- [cosmic-text — docs.rs](https://docs.rs/cosmic-text/latest/cosmic_text/) — text shaping, font discovery/fallback, layout dan glyph rasterization.
- [image — docs.rs](https://docs.rs/image/latest/image/) — image decoding/encoding dan image buffers.

### Parallelism, serialization dan validation

- [Rayon — docs.rs](https://docs.rs/rayon/latest/rayon/) — data parallelism, parallel iterators dan custom thread pools.
- [Serde — docs.rs](https://docs.rs/serde/latest/serde/) — serialization/deserialization framework.
- [jsonschema — docs.rs](https://docs.rs/jsonschema/latest/jsonschema/) — JSON Schema validation.
- [Rust runtime CPU feature detection](https://doc.rust-lang.org/std/macro.is_x86_feature_detected.html) — runtime detection bagi CPU features seperti AVX2.

### Video encoding
- [Remotion performance tips](https://www.remotion.dev/docs/performance) — concurrency benchmark, render logs, media tags, image format and encoding trade-offs.
- [Remotion renderFrames API](https://www.remotion.dev/docs/renderer/render-frames) — concurrency, browser reuse, cache and per-frame timing options.
- [Remotion concurrency](https://www.remotion.dev/docs/terminology/concurrency) — local render concurrency and diminishing returns.

- [FFmpeg documentation](https://ffmpeg.org/documentation.html)
- [FFmpeg formats documentation](https://www.ffmpeg.org/ffmpeg-formats.html) — rawvideo input memerlukan width, height, pixel format dan frame rate yang dinyatakan.

---

## 25. Nota akhir kepada AI coding agent

Anggap dokumen ini sebagai spesifikasi seni bina dan pelan pelaksanaan, bukan arahan untuk menulis keseluruhan engine dalam satu langkah.

Urutan kerja yang wajib:

1. Baca keseluruhan `plan.md` sebelum menulis kod.
2. Bentangkan repository structure dan dependency choices yang akan digunakan.
3. Implement satu phase pada satu masa.
4. Tambah tests bersama setiap feature.
5. Jalankan `cargo fmt`, `cargo clippy` dan `cargo test` secara berkala.
6. Jangan tambah dependency besar tanpa justifikasi dan benchmark.
7. Jangan menggantikan backend atau architecture tanpa mengemas kini dokumen.
8. Jangan claim performance improvement tanpa benchmark yang repeatable dan visual parity pass.
9. Ukur Remotion tuned baseline dahulu; jangan anggap 30 minit sebagai intrinsic Remotion speed.
10. Utamakan Skia CPU bake-off untuk parity, kemudian banding tiny-skia/Vello CPU.
11. Setiap feature perlu ada cost profile: render time per frame, memory dan cacheability.
12. Jika library yang dirancang tidak menyokong feature tertentu, nyatakan limitation dan cadangkan fallback yang tidak melanggar skop.
13. Pastikan keseluruhan output kekal 2D, CPU-first dan bebas daripada Chromium/GPU dependency.

**Hasil akhir yang dikehendaki:** sebuah native 2D motion graphics renderer yang menerima template JSON, menghasilkan output visual setara dengan Remotion untuk feature profile yang disokong, mengurangkan masa render secara terbukti melalui caching, frame parallelism, SIMD dan pipeline encoding yang cekap, mengurus memori secara bounded, serta mengeluarkan video melalui FFmpeg. Sasaran awal ialah sekurang-kurangnya 2× lebih pantas daripada Remotion baseline yang telah ditune; 5× ialah sasaran agresif dan real-time ialah stretch goal, bukan jaminan.
