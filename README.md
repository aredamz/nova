# NOVA

Enjin render video **2D** untuk motion graphics: templat JSON masuk, MP4 keluar.
Ditulis dalam Rust, berjalan pada CPU sahaja (tiada GPU, tiada Chromium).

```bash
nova render examples/product-ad/template.json --data examples/product-ad/data.json -o out/iklan.mp4
```

Pelan asal: [`docs/NOVA_Render_Engine_Plan_v2.md`](docs/NOVA_Render_Engine_Plan_v2.md). Apa yang diputuskan semasa membina, dan
di mana ia berbeza daripada pelan itu: [`docs/keputusan.md`](docs/keputusan.md).

> **Skop.** NOVA melukis bentuk, path, teks, gambar dan SVG dalam 2D. Ia **tidak** boleh
> merender scene 3D (Three.js / React Three Fiber, mesh, lampu, kamera 3D). Video 3D
> sedia ada tidak boleh dipindahkan ke sini; ia perlu direka semula sebagai 2D.

## Ciri

**Layer dan lukisan**

- Lapan jenis layer: `solid`, `rect`, `ellipse`, `path` (data path SVG), `text`, `image`
  (PNG, JPEG, WebP; orientasi EXIF dihormati), `svg` (statik) dan `group`.
- Fill warna atau gradient linear/radial. Stroke dengan cap, join dan trim, untuk kesan
  garisan "dilukis".
- Transform: position, anchor ternormal, scale (seragam atau x/y berasingan), rotation.
- Kesan: opacity, 13 blend mode, Gaussian blur, drop shadow, clip dengan bucu bundar.
  Opacity pada `group` digunakan pada seluruh kandungannya sebagai satu unit, seperti CSS.

**Teks**

- Fon daripada fail dalam templat, dengan senarai fallback, berat, italic, jarak baris,
  jarak huruf dan jajaran.
- Balut baris pada `max_width`, potong dengan elipsis pada `max_lines`.
- `spans`: warna berbeza untuk sebahagian teks.
- `reveal`: dedah teks perkataan demi perkataan (kapsyen karaoke) atau aksara demi aksara
  (mesin taip), tanpa mengubah susun atur.
- `background`: panel di belakang teks (kotak kapsyen, tag nama, cop), dengan `hug` supaya
  panel memeluk teks pendek.

**Animasi dan masa**

- Keyframe pada transform, opacity, blur, shadow, warna, saiz, bucu, stroke dan teks.
- Easing: sembilan lengkung bernama, cubic-bezier seperti CSS, dan spring yang diselesaikan
  secara tertutup (nilai lalai sama dengan `spring()` Remotion).
- Babak: `group` bermasa bertindak sebagai sequence; alih `start_frame` dan semua anak
  serta animasinya ikut beralih.
- Kadar frame bulat atau pecahan (contoh 30000/1001).

**Templat dan data**

- Data binding `{{slot}}` dengan laluan bertitik (`{{product.name}}`), jadi satu templat
  boleh menghasilkan banyak video.
- Templat tidak mengandungi kod: tiada expression, tiada skrip. Laluan aset yang keluar
  dari folder templat (mutlak, `..`, URL, symlink) ditolak.
- JSON Schema dijana daripada parser yang sama, sesuai diberi kepada AI agent yang menjana
  templat.
- `nova validate` melaporkan semua ralat sekali gus dengan lokasi tepat (contoh
  `layers[3].text.font_size`).

**Output**

- MP4 H.264 (yuv420p, BT.709) melalui FFmpeg. Tiga profil (`preview`, `balanced`,
  `high_quality`), atau tetapkan preset dan CRF sendiri.
- Audio: beberapa track dengan masa mula dan volume, dicampur ke AAC.
- Satu frame ke PNG (`nova preview`), atau helaian beberapa frame (`nova sheet`) untuk
  menyemak seluruh video sekilas pandang.
- Render sebahagian julat frame (`--start`, `--end`) dan laporan job JSON (`--json`).

**Enjin**

- CPU sahaja: tiada GPU, tiada Chromium. Satu binari kira-kira 22 MB.
- Deterministik: output sama bit demi bit untuk sebarang bilangan thread.
- Render selari dengan memori terhad; thread render menunggu jika encoder lambat.
- Guna semula kerja: frame yang serupa tidak dirender semula, dan layer mahal yang tidak
  berubah disimpan.
- Diukur pada laptop 4 teras: video 1080p 30 fps sepanjang 4 minit siap dalam 2 minit
  (x264 `medium`). Butiran dalam [`docs/prestasi.md`](docs/prestasi.md).
- `nova doctor` menyemak mesin; `nova benchmark` mengukur throughput.

**Belum ada dalam 0.1**

Scene 3D, layer video, GIF/WebP beranimasi, precomp, mask/matte, motion blur, path morph,
animasi transform per huruf, layout automatik (flex, auto-fit teks), fade dan ducking
audio, cache merentas job, REST API dan queue. Teks RTL/CJK dan emoji belum diuji. Belum
ada perbandingan terukur dengan Remotion. Lihat [`docs/keputusan.md`](docs/keputusan.md).

## Pasang

Ubuntu 24.04 ke atas, dengan sudo:

```bash
scripts/setup-ubuntu.sh        # apt (ffmpeg, libfontconfig1-dev, libfreetype-dev) + Rust + cargo build --release
```

Tanpa sudo (mesin sudah ada `ffmpeg`, `libfreetype6`, `libfontconfig1`):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
scripts/dev-link-shim.sh       # symlink pemaut dalam .link-shim/, ganti pakej -dev
cargo build --release
```

Binari: `target/release/nova` (kira-kira 22 MB, satu fail). Semak mesin dengan
`nova doctor`.

Build pertama memuat turun Skia yang sudah siap dikompil (tiada kompilasi C++). Jika
sasaran anda tiada binari siap, `skia-bindings` akan cuba membina Skia dari sumber, yang
memerlukan clang, python dan ninja serta masa yang lama.

## Arahan

| Arahan | Fungsi |
|---|---|
| `nova validate t.json [--data d.json]` | Semak templat sepenuhnya (struktur, rujukan, aset, fon) tanpa render. Semua ralat dilapor sekali gus. |
| `nova inspect t.json` | Layer, masa, aset, slot data, dan berapa frame yang boleh diguna semula. |
| `nova preview t.json --frame 120 -o f.png` | Satu frame ke PNG. |
| `nova sheet t.json --count 12 -o sheet.png` | Beberapa frame pada satu gambar, untuk menyemak seluruh video sekilas pandang. |
| `nova render t.json -o out.mp4` | Render ke MP4 (H.264, yuv420p, BT.709). |
| `nova benchmark t.json --threads 1,4,8` | Ukur render-only dan end-to-end; median beberapa run. |
| `nova doctor` | CPU, kuota, steal time, SIMD, RAM, FFmpeg, fon. |
| `nova schema` | JSON Schema format templat. |

Pilihan yang kerap digunakan:

```bash
nova render t.json -o out.mp4 --data d.json   # isi slot {{...}}
        --profile preview|balanced|high_quality   # lalai: balanced (x264 medium, CRF 18)
        --preset veryfast --crf 18                # atasi tetapan profil
        --threads 12 --encoder-threads 4          # agihan CPU
        --start 300 --end 600                     # sebahagian frame sahaja
        --no-system-fonts                         # hanya fon dalam templat
        --json                                    # laporan job untuk mesin
```

Kod keluar: `0` berjaya, `2` templat atau data salah (baiki input), `1` kegagalan lain
(FFmpeg, I/O, dibatalkan). Ctrl-C membatalkan dengan kemas: FFmpeg dihentikan dan fail
separa dibuang.

## Format templat

Rujukan penuh: [`docs/template-format.md`](docs/template-format.md). Skema:
[`schemas/template-v1.schema.json`](schemas/template-v1.schema.json).

```json
{
  "schema_version": "1.0",
  "composition": { "id": "demo", "width": 1920, "height": 1080, "fps": 30, "duration_frames": 150, "background": "#0b1020" },
  "assets": [ { "id": "bold", "type": "font", "src": "assets/NotoSans-Bold.ttf", "family": "Noto Sans" } ],
  "layers": [
    { "id": "tajuk", "type": "text",
      "transform": { "position": { "x": 960, "y": 540 }, "anchor": { "x": 0.5, "y": 0.5 } },
      "text": { "value": "{{tajuk}}", "font_family": "Noto Sans", "font_size": 120, "font_weight": 700 },
      "animations": [
        { "property": "transform.scale", "keyframes": [ { "frame": 0, "value": 0.6, "easing": "spring" }, { "frame": 30, "value": 1 } ] },
        { "property": "opacity", "keyframes": [ { "frame": 0, "value": 0 }, { "frame": 12, "value": 1 } ] }
      ] }
  ]
}
```

Senarai apa yang disokong ada di bahagian [Ciri](#ciri).

## Cara ia berfungsi

```text
templat JSON + data ──> bind ──> validate ──> Scene (tak berubah, dikongsi)
                                                │
                              evaluate(frame) ──┴──> FramePlan (data tulen)
                                                         │
        N render thread:  FramePlan ──Skia CPU──> RGBA ──> I420 (BT.709)
                                                         │
                 penulis tersusun ──pipe──> FFmpeg (libx264) ──> MP4
```

- **Deterministik dan random-access.** Setiap frame ialah fungsi tulen bagi nombor
  frame. Output sama bit demi bit untuk sebarang bilangan thread (diuji).
- **Memori terhad.** Thread render hanya boleh mendahului penulis sebanyak satu tetingkap
  (lalai 3 × thread). Jika encoder lambat, render berhenti menunggu, bukan menimbun frame.
- **Tiada kerja berulang.** Frame yang serupa dengan frame sebelumnya tidak dirender
  semula; layer mahal yang tidak berubah dirender sekali dan disimpan; gambar disediakan
  sekali pada saiz paparan terbesarnya.
- **Kualiti.** Vektor dan teks sentiasa dilukis terus pada transform akhir, tidak pernah
  daripada bitmap yang diskala. Komposit dalam sRGB seperti pelayar. Gradient di-dither.
  Tiada JPEG perantaraan: frame pergi terus ke encoder.

## Prestasi

Angka sebenar dan cara mengulanginya: [`docs/prestasi.md`](docs/prestasi.md).

```bash
scripts/run-benchmarks.sh                       # set penanda aras penuh
THREADS=1,4,8,12,16 RUNS=5 scripts/run-benchmarks.sh
```

## Struktur

```text
crates/
  nova-core/        masa, warna, easing, keyframe, geometri, Scene, evaluate() — tiada I/O, tiada backend
  nova-template/    DSL JSON, data binding, validasi, kompilasi ke Scene
  nova-raster/      FramePlan -> piksel; backend Skia CPU, cache layer, penyediaan gambar/SVG/fon
  nova-encoder/     RGBA -> I420 (AVX2), proses FFmpeg, ffprobe
  nova-scheduler/   render selari dengan tetingkap terhad dan output tersusun
  nova-cli/         binari `nova`
examples/           templat contoh (product-ad, shorts, kes01); lihat examples/README.md
benchmarks/         penjana beban kerja penanda aras
schemas/            JSON Schema templat
docs/               format templat, keputusan seni bina, prestasi
```

## Ujian

```bash
cargo test --workspace                  # 125 ujian: unit, integrasi, golden image, CLI
cargo clippy --workspace --all-targets
NOVA_UPDATE_GOLDEN=1 cargo test -p nova-raster --test golden   # jana semula; SEMAK setiap PNG yang berubah
```

Ujian yang memerlukan FFmpeg dilangkau jika ia tiada.

## Penyelesaian masalah

| Gejala | Punca dan tindakan |
|---|---|
| `unable to find library -lfreetype` semasa build | Pasang `libfreetype-dev libfontconfig1-dev`, atau jalankan `scripts/dev-link-shim.sh`. |
| Amaran "is not a font asset of this template" | Fon tidak didaftarkan dalam `assets`; fon sistem digunakan. Tambah fail fon supaya hasil sama di semua mesin. |
| Teks jadi kotak kosong | Fon tiada glyph untuk aksara itu. Tambah fon fallback: `"font_family": ["Noto Sans", "Noto Sans Arabic"]`. |
| `width and height must be even` | H.264 yuv420p perlukan dimensi genap. |
| `cannot start FFmpeg` | Pasang FFmpeg atau set `NOVA_FFMPEG=/laluan/ffmpeg` (dan `NOVA_FFPROBE`). |
| Render lebih perlahan daripada jangkaan | `nova doctor` (kuota CPU, steal time). Lihat baris "limited by" selepas render: `encoder` bermaksud x264 yang mengehadkan, cuba `--preset veryfast`; `renderer` bermaksud scene itu sendiri berat, lihat petua dalam `docs/template-format.md`. |
