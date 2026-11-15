# Format templat NOVA (DSL v1.0)

Templat ialah satu fail JSON. Ia menerangkan satu composition: saiz, kadar frame, aset, dan
senarai layer dengan animasinya. Templat tidak mengandungi kod: tiada expression, tiada
skrip, tiada akses fail di luar folder aset.

Skema rasmi dijana daripada kod parser yang sama, jadi ia sentiasa tepat:

```bash
nova schema > schemas/template-v1.schema.json
```

Beri skema itu kepada AI agent yang menjana templat. Medan yang tidak dikenali ditolak
dengan lokasi yang tepat (contoh `layers[3].text.font_size`), dan `nova validate`
melaporkan semua masalah sekali gus, bukan satu demi satu.

## Rangka

```json
{
  "schema_version": "1.0",
  "composition": {
    "id": "iklan-001",
    "width": 1920, "height": 1080,
    "fps": 30,
    "duration_frames": 450,
    "background": "#0b1020"
  },
  "assets": [],
  "audio": [],
  "layers": []
}
```

| Medan | Nota |
|---|---|
| `width`, `height` | Mesti genap (H.264 yuv420p tidak boleh simpan dimensi ganjil). |
| `fps` | Nombor bulat, atau `{ "num": 30000, "den": 1001 }` untuk kadar pecahan. |
| `duration_frames` | Panjang dalam frame. Frame ialah unit masa yang sebenar; saat sentiasa dikira daripadanya. |
| `background` | Warna legap. Lalai hitam. |

## Masa

- Frame pertama ialah 0.
- `start_frame` (termasuk) dan `end_frame` (tidak termasuk) sesuatu layer adalah dalam masa
  **induknya**. Lalai: dari 0 hingga hujung induk.
- **Keyframe sesuatu layer bermula dari `start_frame` layer itu sendiri.** Keyframe `frame: 0`
  bermaksud "saat layer ini muncul", bukan frame 0 video.
- `group` berfungsi sebagai *sequence*: tukar `start_frame` group, dan semua anak serta
  animasinya ikut beralih. Inilah cara membina babak.

```json
{ "id": "babak-2", "type": "group", "start_frame": 240, "end_frame": 480, "layers": [
    { "id": "tajuk-2", "type": "text", "text": { "value": "Babak dua", "font_family": "Noto Sans", "font_size": 90 },
      "animations": [ { "property": "opacity", "keyframes": [ { "frame": 0, "value": 0 }, { "frame": 12, "value": 1 } ] } ] }
] }
```

`tajuk-2` mula pudar masuk pada frame 240 video.

## Koordinat dan transform

Origin di kiri atas; x ke kanan, y ke bawah; unit piksel.

```json
"transform": {
  "position": { "x": 960, "y": 540 },
  "anchor":   { "x": 0.5, "y": 0.5 },
  "scale":    1.2,
  "rotation": 15
}
```

- `anchor` ialah titik pada kotak layer itu sendiri, sebagai pecahan: `(0,0)` kiri atas,
  `(0.5,0.5)` tengah. Lalai `(0,0)`.
- `position` ialah tempat titik anchor itu diletakkan dalam ruang induk. Jadi untuk
  memusatkan teks yang panjangnya tidak diketahui: `anchor (0.5, 0.5)` dan `position` di
  titik tengah yang dikehendaki.
- `scale` ialah satu nombor atau `{ "x": 1, "y": 1 }`. `rotation` dalam darjah, ikut arah jam.
- Skala dan putaran berpusing pada anchor. Susunan: skala, kemudian putar, kemudian letak.

Kotak setiap jenis layer:

| Jenis | Kotak |
|---|---|
| `solid` | Seluruh composition |
| `rect`, `ellipse` | `size` |
| `text` | `max_width` (jika ada) atau lebar baris terpanjang, kali tinggi teks |
| `image`, `svg` | `size`, atau saiz asal aset |
| `path`, `group` | `size` jika diberi, jika tidak kosong (anchor di origin) |

## Jenis layer

Medan am untuk semua jenis: `id` (unik seluruh templat), `type`, `name`, `visible`,
`start_frame`, `end_frame`, `transform`, `opacity` (0..1), `blend_mode`, `blur`, `shadow`,
`animations`.

### `solid`

```json
{ "id": "latar", "type": "solid", "color": "#101522" }
```

### `rect` dan `ellipse`

```json
{ "id": "kad", "type": "rect", "size": { "width": 400, "height": 240 }, "corner_radius": 24,
  "fill": "#ffd166",
  "stroke": { "color": "#ffffff", "width": 4 } }
```

Perlu `fill` dan/atau `stroke`.

`fill` ialah warna, atau gradient. Koordinat gradient dalam unit kotak layer (0..1):

```json
"fill": { "type": "linear", "from": [0, 0], "to": [1, 1],
          "stops": [ { "offset": 0, "color": "#141a3a" }, { "offset": 1, "color": "#0b1020" } ] }

"fill": { "type": "radial", "center": [0.5, 0.5], "radius": 0.5,
          "stops": [ { "offset": 0, "color": "#ffffff" }, { "offset": 1, "color": "rgba(255,255,255,0)" } ] }
```

`stroke`: `color`, `width`, `cap` (`butt`/`round`/`square`), `join` (`miter`/`round`/`bevel`),
`trim_start`, `trim_end` (0..1). Animasikan `stroke.trim_end` dari 0 ke 1 untuk kesan garisan
"dilukis".

### `path`

```json
{ "id": "garis", "type": "path", "d": "M0 0 C120 -26 300 22 520 -4",
  "stroke": { "color": "#ffd166", "width": 10, "cap": "round" } }
```

`d` ialah data path SVG. `fill_rule`: `nonzero` (lalai) atau `evenodd`. `size` hanya
menentukan kotak untuk anchor dan gradient; ia tidak menskala path.

### `text`

```json
{ "id": "tajuk", "type": "text",
  "text": {
    "value": "Kasut Larian Aero",
    "font_family": "Noto Sans",
    "font_size": 104,
    "font_weight": 700,
    "italic": false,
    "color": "#ffffff",
    "line_height": 1.05,
    "letter_spacing": 0,
    "align": "left",
    "max_width": 820,
    "max_lines": 2
  } }
```

- `"\n"` memulakan baris baharu. Tanpa `max_width`, teks hanya patah pada `"\n"`.
- `font_family` boleh jadi senarai dengan fallback: `["Noto Sans", "Noto Sans Arabic"]`.
- `max_lines` memotong dengan elipsis.
- `spans` mewarnakan sebahagian teks: `{ "start": 11, "end": 27, "color": "#f2c14e" }`
  (indeks aksara, `end` tidak termasuk; mesti tersusun dan tidak bertindih).
- `reveal` memaparkan teks berperingkat tanpa mengubah susun atur:
  `{ "unit": "word", "count": 0, "hidden_color": "rgba(255,255,255,0.3)" }`. Hanya `count`
  perkataan (atau aksara, `"unit": "char"`) pertama dilukis dengan warna sebenar; selebihnya
  dengan `hidden_color` (lalai lutsinar). Animasikan `text.reveal` dengan easing `step`
  untuk kapsyen karaoke, atau secara linear dengan `"unit": "char"` untuk kesan mesin taip.
  Span berwarna hanya kelihatan selepas bahagiannya didedahkan.
- `background` melukis panel di belakang teks (kotak kapsyen, tag nama, cop):
  `{ "color": "rgba(8,10,16,0.72)", "radius": 24, "padding": { "x": 34, "y": 20 },
  "border_color": "#ff5a4e", "border_width": 7 }`. `padding` ialah satu nombor, `{x, y}`
  atau `{left, top, right, bottom}`, dan ia sebahagian daripada kotak layer.
- `hug: true` mengecilkan kotak kepada lebar teks apabila teks muat dalam `max_width`
  tanpa dibalut (teks yang dibalut kekal selebar `max_width`), supaya panel memeluk
  kapsyen pendek.
- **Daftarkan fail fon dalam `assets`.** Jika tidak, fon sistem digunakan dan hasil boleh
  berbeza antara mesin; `nova validate` memberi amaran. Guna `--no-system-fonts` untuk
  memastikan hanya fon templat dipakai.

### `image` dan `svg`

```json
{ "id": "foto", "type": "image", "asset": "gambar-produk",
  "size": { "width": 760, "height": 760 }, "fit": "cover" }
```

`fit`: `contain` (lalai), `cover`, `fill`, `none`. Format gambar: PNG, JPEG, WebP. Orientasi
EXIF dihormati.

SVG statik sahaja (tiada animasi SMIL/CSS dalam fail). SVG diraster sekali pada saiz
terbesar yang akan dipaparkan sepanjang video, jadi ia kekal tajam walaupun diskala besar.
Rujukan fail atau URL luar dalam SVG diabaikan.

### `group`

```json
{ "id": "bingkai", "type": "group", "size": { "width": 760, "height": 760 },
  "clip": true, "corner_radius": 48,
  "layers": [ ... ] }
```

Anak diletakkan dan dimasakan relatif kepada group. `clip` memotong anak pada `size`
(`corner_radius` membundarkan potongan). `opacity` pada group digunakan pada seluruh
kandungannya sebagai satu unit, seperti CSS: anak yang bertindih tidak kelihatan berganda.

## Kesan

| Medan | Makna |
|---|---|
| `opacity` | 0..1, pada layer sebagai satu unit |
| `blend_mode` | `normal`, `multiply`, `screen`, `overlay`, `darken`, `lighten`, `color_dodge`, `color_burn`, `hard_light`, `soft_light`, `difference`, `exclusion`, `add` |
| `blur` | Gaussian blur, sisihan piawai dalam piksel, sama seperti CSS `filter: blur()` |
| `shadow` | `{ "x": 0, "y": 30, "blur": 80, "color": "rgba(0,0,0,0.55)" }`, semantik CSS `drop-shadow()` |

## Animasi

```json
"animations": [
  { "property": "transform.position.y",
    "keyframes": [
      { "frame": 10, "value": 420, "easing": "ease_out_cubic" },
      { "frame": 30, "value": 330 }
    ] }
]
```

- `easing` pada sesuatu keyframe mengawal segmen dari keyframe itu ke keyframe berikutnya.
  Lalai `linear`.
- Sebelum keyframe pertama nilai pertama dikekalkan; selepas yang terakhir, nilai terakhir.
- Frame keyframe mesti menaik.

Property yang boleh dianimasikan:

| Layer | Property |
|---|---|
| Semua | `transform.position.x`, `transform.position.y`, `transform.scale`, `transform.scale.x`, `transform.scale.y`, `transform.rotation`, `transform.anchor.x`, `transform.anchor.y`, `opacity`, `blur`, `shadow.x`, `shadow.y`, `shadow.blur`, `shadow.color` |
| `solid` | `color` |
| `rect` | `size.width`, `size.height`, `corner_radius`, `fill.color`, `stroke.color`, `stroke.width`, `stroke.trim_start`, `stroke.trim_end` |
| `ellipse` | `size.width`, `size.height`, `fill.color`, `stroke.*` |
| `path` | `fill.color`, `stroke.*` |
| `text` | `text.color`, `text.font_size`, `text.letter_spacing`, `text.reveal` |

Awalan `transform.` boleh ditinggalkan. Nilai warna ditulis sebagai rentetan (`"#ff0000"`).
Gradient tidak boleh dianimasikan.

### Easing

| Bentuk | Contoh |
|---|---|
| Nama | `"linear"`, `"step"`, `"ease_in_quad"`, `"ease_out_quad"`, `"ease_in_out_quad"`, `"ease_in_cubic"`, `"ease_out_cubic"`, `"ease_in_out_cubic"`, `"spring"` |
| Cubic Bezier (seperti CSS) | `{ "cubic_bezier": [0.2, 0, 0, 1] }` |
| Spring | `{ "spring": { "stiffness": 100, "damping": 10, "mass": 1 } }` |

`spring` ialah pengayun teredam yang diselesaikan secara tertutup (tiada simulasi), jadi
mana-mana frame boleh dikira tanpa frame sebelumnya. Nilai lalai sama dengan `spring()`
Remotion. Spring berjalan dalam masa sebenar: beri jarak keyframe yang cukup (kira-kira
30 frame pada 30 fps untuk nilai lalai) supaya ia sempat mendap; pada keyframe berikutnya
nilai tepat mengambil nilai sasaran.

## Aset

```json
"assets": [
  { "id": "regular", "type": "font",  "src": "assets/NotoSans-Regular.ttf", "family": "Noto Sans" },
  { "id": "bold",    "type": "font",  "src": "assets/NotoSans-Bold.ttf",    "family": "Noto Sans" },
  { "id": "foto",    "type": "image", "src": "assets/product.jpg" },
  { "id": "logo",    "type": "svg",   "src": "assets/logo.svg" }
]
```

`src` relatif kepada folder templat (atau `--assets-root`). Laluan mutlak, `..`, URL dan
symlink yang keluar dari folder itu ditolak. Beberapa fail fon boleh berkongsi `family`
yang sama (regular, bold); berat dipilih melalui `font_weight`.

## Audio

```json
"audio": [
  { "id": "suara",  "src": "audio/voice.wav" },
  { "id": "muzik",  "src": "audio/music.mp3", "start_frame": 30, "volume": 0.35 }
]
```

Track dicampur dan dimux oleh FFmpeg (AAC 192 kbps) dan dipotong pada panjang video.
Belum ada: fade, ducking, normalisasi loudness. Untuk master akhir, proses audio
berasingan dan mux hasilnya.

## Data binding

Mana-mana rentetan boleh mengandungi `{{slot}}`:

```json
"text": { "value": "{{product.name}}", "font_family": "Noto Sans", "font_size": "{{saiz}}" }
```

```bash
nova render template.json --data data.json -o out.mp4
```

- `{{a.b.0.c}}`: laluan bertitik ke dalam fail data; nombor ialah indeks array.
- Rentetan yang **seluruhnya** satu slot mengambil jenis JSON nilai itu (nombor, objek, array).
  Slot di tengah teks digantikan dengan teks nilai itu.
- Tiada expression, tiada fungsi. Slot yang tiada nilainya ialah ralat sebelum render.
- Nilai data tidak diimbas semula untuk slot lain.
- `nova inspect template.json` menyenaraikan slot yang diperlukan.

## Menulis templat yang laju

Engine menyimpan hasil raster layer yang tidak berubah dan menggunakannya semula
(lihat `docs/prestasi.md`). Tiga tabiat yang memanfaatkannya:

1. **Letak bayang pada bentuk legap yang ringkas**, bukan pada group. Bayang pada `rect`
   atau `ellipse` ber-fill legap (tanpa stroke) dan pada gambar tanpa ketelusan, dengan
   `opacity` 1, dilukis terus dan murah walaupun bergerak. Bayang pada group, teks, path
   atau bentuk lutsinar memerlukan layer luar skrin setiap kali kandungannya berubah.
   Untuk kad foto yang sedang zum: buat `rect` berbayang di belakang, dan foto dalam group
   ber-`clip` di atasnya.
2. **Blur besar yang sentiasa bergerak ialah kesan paling mahal.** Guna radial gradient
   yang pudar ke lutsinar untuk "glow" jika rupa itu memadai.
3. **Biarkan frame pegun betul-betul pegun.** Frame yang serupa dengan frame sebelumnya
   tidak dirender semula langsung. Satu bar kemajuan yang bergerak sepanjang video
   menghalang penjimatan ini.
