# Frame bake: melukis komposisi Remotion dengan NOVA

Templat NOVA menerangkan bentuk dan keyframe. Itu tidak cukup untuk video yang
geometrinya **dikira semula oleh kod pada setiap frame**: rig watak yang menjana semula
garis luar anggota, set 3D yang diunjur ke path, lorekan cel melalui `clipPath`. Tiada
keyframe yang boleh menyatakannya.

Untuk video begitu NOVA menyediakan laluan kedua: kod asal terus menjadi sumber, dan NOVA
hanya **melukis**. Kod React dijalankan di Node tanpa pelayar, hasil setiap frame disimpan
sebagai *pek*, dan layer `frames` melukis pek itu.

```
kod Remotion (React)                      NOVA
────────────────────                      ────
build.mjs   bundel (esbuild), modul pelayar diganti
bake.mjs    setiap frame: renderToStaticMarkup ─► HTML + SVG
            convert.mjs: susun atur HTML mini ─► SVG tulen + operasi shader
            ─► pek (frames.json + frames-NN.bin)  ─►  layer `frames`
                                                       usvg  (hurai)
                                                       Skia  (lukis)
                                                       x264  (encode)
```

Yang lambat dalam Remotion ialah Chromium mengambil tangkapan skrin setiap frame, bukan
kod React. Tanpa Chromium, kod yang sama berjalan ±80 ms sebingkai pada satu teras.

## Guna

Satu arahan untuk semua langkah (bundel, bake, campur audio, templat, `nova render`):

```bash
(cd tools/remotion-bake && npm install)        # sekali (opentype.js)

node tools/remotion-bake/render.mjs --project <projek> --target target.ts \
     --fonts <folder ttf> --out video.mp4 [--lufs -14] [--quality standard|ultra|ultra1080] [--clean]
```

Contoh sedia jalan: [`examples/remotion-bake`](../examples/remotion-bake). Langkah yang sama, satu
demi satu:

```bash
node tools/remotion-bake/build.mjs --project <projek> --target target.ts --out bundle.cjs
node tools/remotion-bake/bake.mjs bundle.cjs --project <projek> --fonts <folder ttf> --out pack
node tools/remotion-bake/mix.mjs pack/audio.json --project <projek> --out mix.wav --lufs -14
nova render template.json -o video.mp4
```

`target.ts` mengeksport komposisi yang hendak dibakar:

```ts
import { Episod, DURASI } from "../src/episodes/Episod";
export const COMP = { id: "Episod", component: Episod, width: 1080, height: 1920, fps: 30, durationInFrames: DURASI };
```

Templat hanya perlu merujuk pek (dan fon yang teksnya gunakan):

```json
{ "schema_version": "1.0",
  "composition": { "id": "Episod", "width": 1080, "height": 1920, "fps": 30, "duration_frames": 7504 },
  "assets": [
    { "id": "bold", "type": "font", "src": "fonts/NotoSans-Bold.ttf", "family": "Noto Sans" },
    { "id": "bake", "type": "frames", "src": "pack/frames.json" } ],
  "audio": [ { "id": "mix", "src": "mix.wav" } ],
  "layers": [ { "id": "episod", "type": "frames", "asset": "bake" } ] }
```

Layer `frames` ialah layer biasa: ia boleh dianjak, diskala, dipudarkan, diberi
`blend_mode`, dan layer NOVA lain boleh diletak di atas atau di bawahnya. Frame tempatan 0
layer memaparkan frame pertama pek; selepas frame terakhir, frame terakhir ditahan.

`bake.mjs --dump <frame>` menulis SVG satu frame ke stdout, untuk menyemak apa yang
dihasilkan penukar.

## Apa yang disokong

**SVG** (dilukis oleh NOVA melalui `usvg` + Skia): path, bentuk asas, `<text>`, `<image>`,
`<pattern>`, gradient linear/radial (termasuk `userSpaceOnUse` dan `gradientTransform`),
`clipPath` bentuk sewenang dan bersarang, `mask` luminance/alpha, `mix-blend-mode`,
`opacity`, `paint-order`, dash; filter `feGaussianBlur` (termasuk berarah), `feOffset`,
`feColorMatrix`, `feComponentTransfer`, `feComposite`, `feBlend`, `feFlood`, `feMerge`,
`feDropShadow`, `feMorphology`, `feTurbulence`, `feDisplacementMap`, `feImage`; fungsi
filter CSS `blur`, `brightness`, `contrast`, `saturate`, `sepia`, `hue-rotate`,
`grayscale`, `invert`, `opacity`, `drop-shadow`. Belum: `feConvolveMatrix`, pencahayaan
(`feDiffuseLighting`, `feSpecularLighting`) dan `feTile` (input diteruskan tanpa diubah).

**HTML** (disusun oleh `convert.mjs`, kerana tiada pelayar): subset yang dipakai video
motion graphics, bukan CSS penuh.

| Disokong | Nota |
|---|---|
| `position: absolute`, `top/left/right/bottom/inset`, `width/height` (px, %) | `box-sizing: border-box`, seperti Remotion |
| `display: flex` baris/lajur, `align-items`, `justify-content`, `gap` | tanpa `flex-grow`/`wrap` |
| aliran blok, `padding`, `margin` (termasuk `0 auto`), `max-width` | teks bebas di sebelah elemen blok mendapat kotak tanpa nama |
| `transform` (`translate[X/Y]`, `scale[X/Y]`, `rotate`, `matrix`), `transform-origin` | |
| `opacity`, `filter`, `mix-blend-mode`, `overflow: hidden` | `filter: url(#id)` memakai kotak elemen bersama limpahan anak, seperti Chrome |
| `clip-path`: `polygon()`, `inset()` (dengan `round`), `circle()`, `ellipse()`, `path()` | |
| `background` warna, `linear-gradient`, `radial-gradient`, `repeating-*-gradient` (berlapis) | interpolasi pra-darab alfa seperti CSS (hentian perantaraan) |
| `border`, `border-left`, `box-shadow` (termasuk spread), `border-radius` (px/%, eliptik, setiap sudut) | |
| teks: `font-*`, `color`, `letter-spacing`, `line-height`, `text-align`, `text-transform`, `text-shadow`, `white-space: pre` | pecah baris ikut ukuran fon sebenar |
| `<span>` dengan `transform`/`opacity` sendiri | animasi per huruf |

Sifat CSS yang tidak difahami penukar diberi **amaran semasa bake** (sekali setiap sifat), supaya
jurang kelihatan sebelum render dan bukan sebagai gambar yang salah; `transform` yang tidak
dikenali ialah ralat. Bake tanpa amaran bermakna semua CSS episod itu difahami.

**Fon.** Teks HTML diukur dengan fon sebenar: fail TTF dalam `--fonts` dahulu, kemudian fon
sistem melalui fontconfig dengan aturan Chrome — nama generik menjadi fon lalai Chrome (`sans-serif`
→ Arial → Liberation Sans pada kebanyakan Linux), dan nama khusus hanya diterima jika fontconfig
memulangkan keluarga itu sendiri atau alias metrik-serasinya; pengganti rambang ditolak dan nama
seterusnya dalam senarai dicuba. Aksara yang tiada dalam fon terpilih (cth. `●`, `✓`) diambil
daripada fon piawai Chrome (Times New Roman → Liberation Serif) jika ada di situ, kemudian daripada
carian fontconfig mengikut aksara. `<text>` SVG diselesaikan NOVA dengan aturan yang sama, dengan satu
beza: jika fon sandaran meliputi seluruh rentetan, `usvg` menukar seluruh rentetan itu, bukan hanya
aksara yang tiada (teks HTML tidak terjejas, kerana penukar memecahnya per fon). Untuk hasil
yang sama pada mesin lain, letak semua fon yang dipakai dalam `--fonts` dan templat.

**Skia** (`@remotion/skia`): `<Fill><Shader/></Fill>` menjadi operasi shader. Sumber SkSL
yang sama dijalankan oleh Skia pada CPU, dengan uniform frame itu. Elemen Skia lain
(`Path`, `BlurMask`) belum disokong; pembakar memberi amaran.

## Kualiti melebihi rujukan: master 4K

Frame yang dibakar ialah vektor, jadi ia boleh dilukis pada sebarang saiz. `--scale 2` melukis
semula setiap frame pada 2160×3840 (bukan membesarkan gambar 1080p), dan `--also` menulis fail
1080p daripada frame yang sama:

```bash
nova render template.json --scale 2 --profile master -o episod-4k.mp4 --also episod-1080p.mp4
# hanya fail saiz asal (dilukis pada 2x lalu dikecilkan; tiada master 4K):
nova render template.json --scale 2 --profile master --supersample -o episod-1080p.mp4
```

- **Garis halus dan tepi** mendapat empat kali piksel; fail 1080p ialah render 4K yang disampel
  turun (Lanczos), jadi anti-aliasnya lebih bersih daripada render 1080p terus.
- **Grain dan shader lain** dibuat pada resolusi pek dan dibesarkan (Catmull-Rom): butir grain
  kekal sebesar yang ditulis, tidak menjadi separuh saiz.
- **Profil `master`** (x264 medium, CRF 17, `-tune grain`) mengekalkan tekstur yang dilenyapkan
  oleh CRF 18 biasa. Fail lebih besar: ±60 Mbps pada 4K, ±13 Mbps pada 1080p.

Yang TIDAK dibuat, dan sebabnya: motion blur dan 60 fps memerlukan komposisi dinilai pada
pecahan frame. Kod yang ditulis untuk frame integer tidak selamat untuk itu (warna `NaN`, kesan
satu-frame hilang); lihat keputusan 17 dalam [`keputusan.md`](keputusan.md).

## Audio

`<Audio>` (`@remotion/media`, juga `Audio`/`Html5Audio` daripada `remotion`) tidak memainkan apa-apa
semasa bake; ia **merekod** apa yang Remotion akan campurkan pada frame itu: fail, kedudukan dalam
fail, kadar main, gelung/trim dan volum (fungsi `volume` dinilai seperti `AudioForRendering`, termasuk
`loopVolumeCurveBehavior`). `bake.mjs` menggabungkan rekod itu menjadi kiu dalam `pack/audio.json`:

```json
{ "nova_audio": 1, "fps": 30, "frames": 7504, "cues": [
  { "src": "public/audio/ep/n1-01.mp3", "frame": 6, "t": 0, "rate": 1.18, "preserve_pitch": true, "volumes": [1, 1, 1] },
  { "src": "public/sfx/rain.mp3", "frame": 1, "t": 0.033333, "rate": 1, "loop": true, "volumes": [0.01, 0.02, 0.03] } ] }
```

`mix.mjs` menyahkod setiap fail sekali dengan FFmpeg (48 kHz stereo; kadar main digunakan di situ —
`atempo` bila `preservePitch`, pensampelan semula bila tidak; fail mono disalin penuh ke kedua-dua
saluran seperti Web Audio) dan menjumlahkan sampel. Volum berubah setiap frame seperti Remotion,
dengan tanjakan 5 ms supaya tiada klik. `--lufs N` menambah master: `loudnorm` satu laluan +
pengehad, puncak diukur selepas AAC dan sasaran diturunkan jika melebihi −1 dBFS.

Diukur pada episod 250 s (282 kiu, 106 fail) berbanding render audio Remotion: masa sejajar
(anjakan 0 ms), beza aras median 0.06 dB (paling teruk ±0.5 dB dalam mana-mana saat), korelasi
sampul 0.996. Campur 5 s (30 s dengan master); render audio sahaja Remotion mengambil ±30 minit
(diukur semasa mesin turut menjalankan bake, jadi angka itu kasar).
Gelombang tidak sama sampel demi sampel pada suara yang dipercepat, kerana regangan masa
(`atempo`) berbeza fasa daripada peregang Remotion.

Tidak disokong: audio daripada `<Video>`/`<OffthreadVideo>` (video tidak dibakar langsung).

## Format pek

`frames.json`:

```json
{ "nova_frames": 1, "width": 1080, "height": 1920, "fps": 30, "frames": 7504,
  "shards": ["frames-00.bin", "…"],
  "shaders": { "<kunci>": "<sumber SkSL>" },
  "index": [[shard, offset, panjang_mampat, panjang_mentah, id_kandungan], …] }
```

Setiap frame ialah satu blob zlib dalam serpihan. Selepas dinyahmampat, blob ialah
senarai operasi `[jenis u8][panjang u32 LE][muatan]`:

| Jenis | Muatan |
|---|---|
| 1 | dokumen SVG (UTF-8) sebesar frame, dilukis di atas apa yang sudah ada |
| 2 | JSON `{ "shader": "<kunci>", "uniforms": { nama: nombor \| [nombor] } }`: satu hantaran shader selebar frame, source-over |

Frame yang kandungannya sama berkongsi `id_kandungan`, jadi pelan frame mereka sama dan
frame itu tidak dirender dua kali.

Pek dilayan seperti templat, iaitu tidak dipercayai: serpihan mesti nama fail biasa di
sebelah manifes; setiap entri indeks disemak terhadap saiz serpihan sebelum render; saiz
mentah dihadkan (`max_frame_bytes`) dan penyahmampatan berhenti pada saiz yang diisytihar;
gambar yang dirujuk SVG hanya dibaca dari dalam folder pek; shader dikompil semasa
persediaan supaya ralat SkSL ialah ralat aset.

## Kenapa Skia, bukan resvg

NOVA sudah memakai `resvg` untuk aset SVG statik (diraster sekali). Untuk frame bake ia
terlalu perlahan: pada frame ±2,000 elemen, `resvg` mengambil **35 s**. Ia membuat satu
lapisan luar skrin bagi setiap elemen ber-`opacity` (525 dalam frame itu) dan filternya
berjalan sepiksel dalam Rust tanpa SIMD.

Pelukis dalam `crates/nova-raster/src/skia/svg.rs` mengekalkan `usvg` sebagai penghurai
(12 ms; ia sudah menyelesaikan pewarisan, unit, `use`, teks kepada garis luar dan fungsi
filter CSS) dan melukis pokoknya dengan Skia. Frame yang sama: **0.33 s**. Yang membezakan:

- `opacity` pada satu bentuk dilipat ke dalam cat (tiada lapisan);
- `clipPath` daripada bentuk biasa menjadi klip kanvas (tiada lapisan topeng);
- lapisan filter hanya sebesar kandungannya, bukan sebesar rantau filter (yang sering jauh
  lebih besar daripada skrin), dan kumpulan yang hasilnya di luar skrin tidak dilukis;
- rantaian filter warna (`brightness() saturate() sepia()`) menjadi satu penapis warna;
- blur dengan sigma ≤ 4 px dibuat oleh konvolusi boleh-pisah dalam enjin. Skia pada CPU
  menjalankan blur sekecil itu melalui shader yang ditafsir, ±100 ns sepiksel;
- hantaran shader dan isian gradient selebar skrin yang berulang antara frame (grain yang
  ditahan tiga frame, vignet yang sama sepanjang babak) dirender sekali dan dikongsi
  semua thread.

Dua perkara sengaja berbeza daripada pelayar. Kedua-duanya terhad kepada tepi beberapa
piksel dan tidak kelihatan pada video:

- blur sigma ≤ 4 px dibuat dalam nilai berkod sRGB, bukan cahaya linear;
- gradient tidak di-dither.

`NOVA_SVG_PROFILE=1` mencetak ke mana masa setiap dokumen pergi (isian, strok, lapisan
filter, blur) dan menyenaraikan elemen yang lambat.

## Ukuran

Lihat [`prestasi.md`](prestasi.md#frame-bake-satu-episod-penuh).
