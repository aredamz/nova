# Keputusan seni bina dan status

Dokumen ini merekodkan apa yang diputuskan semasa membina NOVA 0.1, di mana ia berbeza
daripada `NOVA_Render_Engine_Plan_v2.md`, dan apa yang belum dibuat. Pelan itu meminta
supaya sebarang perubahan seni bina didokumenkan; inilah tempatnya.

## Ringkasan status

| Fasa pelan | Status |
|---|---|
| 0 — Repo, baseline Remotion, bake-off backend | **Separa.** Repo, CLI, `doctor`, harness benchmark siap. Baseline Remotion dan bake-off tiga backend **belum** (lihat bawah). |
| 1 — Renderer 2D statik | Siap |
| 2 — Template compiler dan scene graph | Siap |
| 3 — Timeline dan animasi | Siap |
| 4 — Teks, gambar, SVG | Siap |
| 5 — Encoding video | Siap |
| 6 — Performance engineering | Siap untuk skop 0.1; diukur pada laptop 4 teras sahaja, belum pada VPS 16 vCPU |
| 7 — Servis produksi (API, queue) | Belum dimulakan |

## Yang belum dibuat, dan kenapa ia penting

1. **Baseline Remotion dan ujian parity visual.** Pelan (0.2, 18.1A) mewajibkan ukuran
   Remotion yang telah ditala sebelum sebarang dakwaan kelajuan. Ini belum dibuat: video
   Remotion sedia ada ialah 3D (React Three Fiber), jadi tiada composition 2D yang setara
   untuk dibandingkan. **Oleh itu tiada angka "N× lebih laju daripada Remotion" dalam
   repo ini.** Yang ada ialah throughput NOVA sendiri (`docs/prestasi.md`). Untuk menutup
   jurang ini: bina satu composition 2D yang sama dalam Remotion dan dalam NOVA, render
   kedua-duanya pada mesin yang sama, dan bandingkan masa serta frame PNG.
2. **Ukuran pada VPS sasaran.** Semua angka datang daripada laptop 4 teras / 8 thread.
   Skala ke 16 vCPU perlu diukur, bukan diandaikan: `scripts/run-benchmarks.sh`.
3. **Bake-off backend.** Hanya Skia CPU dilaksanakan (keputusan 1).

Ciri yang tiada dalam 0.1: precomp boleh guna semula, layer video, GIF/WebP beranimasi,
animasi transform per huruf/perkataan (warna per span dan reveal karaoke/mesin taip sudah
ada), mask/matte
alpha atau luma, motion blur, path morph, layout automatik (flex, auto-fit teks), fade
dan ducking audio, cache merentas job, REST API dan queue. Ujian teks RTL/CJK dan emoji
belum ditulis (shaping melalui HarfBuzz menyokongnya, tetapi belum disahkan dengan fon
dan fixture).

## Keputusan

### 1. Skia CPU sebagai backend tunggal

Pelan meminta bake-off Skia CPU / tiny-skia / Vello CPU sebelum memilih. Yang dibuat:
proof-of-concept Skia (build, saiz binari, teks, filter), kemudian terus dipilih.

Sebab: pelayar yang Remotion gunakan dibina atas rasterizer yang sama, jadi antialiasing,
gradient, blur dan teks (HarfBuzz + FreeType) paling hampir tanpa perlu ditiru. tiny-skia
tiada teks dan tiada filter, jadi kedua-duanya perlu ditulis sendiri. Vello CPU masih
mempunyai had pada filter.

Kos yang diterima: binari Skia siap kompil dimuat turun semasa build (22 MB binari
akhir), dan pemautan memerlukan `libfreetype` serta `libfontconfig` sistem.

Sempadan backend dikekalkan seperti yang pelan mahu, tetapi pada aras yang lebih tinggi:
`RasterBackend` / `FrameRenderer` menerima `FramePlan` dan memulangkan piksel. Tiada jenis
Skia keluar dari `nova-raster`. Backend lain boleh ditambah di sebalik trait yang sama;
trait aras `draw_path` dalam pelan akan menghalang penggunaan layer, filter dan paragraph
Skia.

resvg (dan tiny-skia di dalamnya) masih digunakan, tetapi hanya untuk meraster SVG sekali
semasa penyediaan aset.

### 2. Enam crate, bukan tiga belas

`nova-animation` digabung ke `nova-core`; teks, SVG, gambar, compositor dan cache ke
`nova-raster`. Pelan sendiri menasihatkan supaya tidak memecah crate sebelum ia memberi
manfaat. `nova-api` dan `nova-bench` belum wujud; benchmark ialah arahan `nova benchmark`.

### 3. Thread berskop dan scheduler sendiri, tanpa Rayon

Setiap thread render memiliki keadaan yang tidak boleh dikongsi (surface, koleksi fon,
cache layout teks), dan penjadualan perlu menghormati tetingkap in-flight. Thread berskop
`std` dengan satu mutex dan dua condvar memenuhi kedua-duanya dalam kira-kira 200 baris,
tanpa kebergantungan tambahan.

### 4. Skema dijana daripada jenis, bukan ditulis tangan

Validasi struktur dibuat oleh serde dengan `deny_unknown_fields` (ralat membawa laluan
medan), diikuti validator semantik Rust. JSON Schema dijana daripada jenis yang sama
(`nova schema`), jadi skema dan parser tidak boleh bercanggah. Satu ujian memastikan fail
skema dalam repo sentiasa terkini.

### 5. Keyframe dalam masa tempatan layer; group sebagai sequence

Frame keyframe dikira dari `start_frame` layer itu, dan anak mengikut masa group induknya.
Ini memberi "babak" tanpa jenis `precomp`: tukar `start_frame` satu group dan seluruh
babak beralih. Contoh dalam pelan (semua layer bermula pada 0) berkelakuan sama.

### 6. Anchor ternormal; `position` ialah lokasi anchor

`anchor` ialah pecahan kotak layer, dan `position` ialah tempat titik itu diletakkan.
Memusatkan teks yang panjangnya tidak diketahui menjadi `anchor (0.5, 0.5)` tanpa perlu
tahu lebarnya. Kesan pada contoh pelan: layer gambar dengan anchor `(0.5, 0.5)` kini
dipusatkan pada `position`, bukan diletakkan dari bucu kiri atas.

### 7. Blur, shadow, blend mode, clip dan opacity group dalam MVP

Pelan meletakkan kesan ini dalam "Fasa 2" (9.6), tetapi seksyen 1.4 menyenaraikannya
sebagai syarat parity. Dengan Skia ia adalah image filter dan save-layer sedia ada.
`opacity` pada group dikomposit sebagai satu unit, seperti CSS. `shadow.blur` mengikut
semantik CSS (jejari = 2 sigma).

### 8. Spring dalam MVP, diselesaikan secara tertutup

Pelan menangguhkan spring kerana "bergantung pada sejarah frame". Pengayun teredam ada
penyelesaian tertutup, jadi ia fungsi tulen bagi masa. Diuji terhadap integrasi numerik.

### 9. Komposit dalam sRGB berkod gamma

Pelayar mengadun CSS dalam sRGB berkod gamma. Linear-light (disebut dalam pelan 5.4
sebagai pilihan kualiti) akan membuat fade, gradient dan blur kelihatan berbeza daripada
Remotion, jadi ia tidak digunakan. Gradient diinterpolasi premultiplied seperti CSS, dan
di-dither untuk mengelak banding 8-bit.

### 10. Penukaran warna dalam thread render

Frame ditukar RGBA ke I420 (BT.709, limited range) oleh thread render, dengan AVX2
apabila ada (hasil bit-serupa dengan laluan skalar; diuji). FFmpeg menerima `yuv420p`
terus: tiada swscale, paip membawa 1.5 bait sepiksel dan bukan 4, dan matriks warna
sepadan dengan tag `bt709` dalam fail. Satu ujian meng-encode tompok warna dan
menyahkodnya semula untuk mengesahkan warna tidak beralih.

### 11. Guna semula tanpa logik invalidation

- **Frame serupa.** `FramePlan` (semua nilai yang boleh berbeza antara frame) dibanding
  dengan frame sebelumnya. Jika sama, frame tidak dirender; encoder tetap menerima setiap
  frame.
- **Cache layer.** *Polisi* (nod mana dirender melalui layer luar skrin) ditentukan sekali
  daripada scene sahaja: subtree yang tidak pernah berubah dan mengandungi gradient,
  gambar atau filter; dan nod ber-filter yang pegun sekurang-kurangnya suku hayatnya.
  *Guna semula* ialah perbandingan nilai tepat: nod yang sama, transform peranti yang
  sama. Layer cache hanya dikomposit pada kedudukan piksel bulat ia dirender, tidak pernah
  diskala atau diputar. Oleh sebab polisi tidak bergantung pada keadaan cache, output
  tetap sama untuk sebarang bilangan thread.
- **Gambar.** Bagi setiap layer gambar, skala paparan terbesar sepanjang video dikira
  daripada scene. Raster dikecilkan sekali dengan Lanczos-3 ke saiz itu; SVG diraster
  terus daripada vektor pada saiz itu. Frame kemudian melukis pada 1:1 atau sedikit di
  bawahnya.

Ini menggantikan peraturan invalidation dalam pelan 9A.3 dan 10.3 dalam satu proses.
Cache merentas job (kunci content-hash, pelan 10.2) belum dilaksanakan; hash kandungan
aset sudah dikira dan dipaparkan oleh `nova inspect`.

### 12. Audio asas disertakan

Track dengan `start_frame` dan `volume` dicampur oleh FFmpeg dalam proses encode yang
sama. Tiada fade, ducking atau normalisasi loudness.

### 13. Sampling gambar bilinear, bukan bicubic

Pembesaran gambar menggunakan bilinear, sama seperti kualiti lalai pelayar. Bicubic pada
CPU kira-kira 3.5 kali lebih mahal sepiksel (diukur) tanpa menyamai Remotion dengan lebih
baik.

### 14. Bayang bentuk legap dilukis sebagai garis luar yang dikaburkan

Drop shadow ialah salinan kabur bagi kawasan yang diliputi nod. Apabila nod itu satu
bentuk legap sepenuhnya (rect atau ellipse dengan fill legap tanpa stroke, atau gambar
tanpa ketelusan) pada opacity 1 tanpa blur, kawasan itu ialah garis luarnya sendiri. Bayang
kemudian dilukis terus sebagai bentuk dengan mask blur Gaussian: tiada layer luar skrin,
tiada penapisan kandungan. Diukur pada bentuk tidak berputar: beza maksimum 2/255
berbanding kaedah filter. Di bawah putaran, kaedah ini malah lebih tajam, kerana kaedah
filter mensampel semula seluruh layer.

Pilihan kaedah dibuat daripada scene sahaja (opacity dan blur mesti malar), jadi satu nod
tidak pernah bertukar kaedah di tengah video. Bitmap yang tiada piksel lutsinar ditanda
legap supaya Skia menyalin, bukan mengadun.

## Definition of Done pelan (seksyen 23)

| Item | Status |
|---|---|
| Workspace Rust dan release build stabil | Ya |
| `validate`, `inspect`, `preview`, `render`, `benchmark`, `doctor` | Ya (tambahan: `sheet`, `schema`) |
| DSL v1 didokumenkan dan tervalidasi | Ya |
| Shapes, paths, teks, gambar, SVG, group | Ya |
| Transform, opacity, keyframe, easing | Ya |
| PNG preview dan MP4 export | Ya |
| Kegagalan FFmpeg dikesan | Ya (diuji: binari tiada, proses mati di tengah) |
| Susunan frame betul walaupun worker siap tidak tertib | Ya (diuji) |
| Had memori dan queue boleh dikonfigurasi | Ya (`--window`, bajet cache layer, `Limits`) |
| Ujian unit, integrasi, golden | Ya (125 ujian) |
| Benchmark 1/2/4/8/12/16 thread direkodkan | **Separa:** 1/2/4/8 pada laptop; 12/16 menunggu VPS |
| Output deterministik merentas bilangan thread | Ya (diuji bit demi bit) |
| Had keselamatan untuk laluan, SVG, aset | Ya; tiada had bilangan nod SVG (hanya saiz fail dan saiz raster) |
| Tiada kebergantungan Chromium atau GPU | Ya |
| README: pasang, render, format, troubleshooting | Ya |
| Parity visual dengan Remotion; 1.25× lebih laju daripada baseline | **Belum diukur** |
