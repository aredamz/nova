# Prestasi

Semua angka di bawah diukur, bukan dianggar, pada **satu mesin**: laptop AMD Ryzen 5 7520U
(4 teras / 8 thread, 15 W), 13 GiB RAM, AVX2, Ubuntu, FFmpeg 8.0.1, tanpa GPU. Tarikh
ukuran: 15 November 2026.

Dua perkara **belum** diukur, jadi tiada dakwaan mengenainya:

- **VPS sasaran (16 vCPU).** Jalankan `scripts/run-benchmarks.sh` di sana.
- **Perbandingan dengan Remotion untuk templat.** Belum ada composition 2D yang sama
  ditulis dalam kedua-dua enjin. Yang ada ialah perbandingan untuk laluan frame bake
  (kod Remotion yang sama, dilukis oleh NOVA), di [hujung dokumen ini](#frame-bake-satu-episod-penuh).

## Job rujukan: video 4 minit

`benchmarks/mixed-4min.json`: 1920×1080, 30 fps, 7200 frame. 30 babak daripada lima jenis
(hero dengan foto zum dan bayang, petikan dengan gelang berputar, dua foto bersebelahan,
carta bar, latar blur Gaussian besar yang sentiasa bergerak), kapsyen bertukar setiap 24
frame, dan bar kemajuan yang berubah pada setiap frame, jadi **tiada frame yang diguna
semula**. 649 layer.

| Tetapan encoder | Masa | Throughput | Laju berbanding masa nyata | Saiz fail |
|---|---:|---:|---:|---:|
| Lalai: x264 `medium`, CRF 18 | **2 min 00 s** | 59.8 fps | 1.99× | 30.0 MiB |
| `--preset veryfast`, CRF 18 | **1 min 16 s** | 94.3 fps | 3.14× | 28.6 MiB |

Memori puncak proses `nova`: 425 MiB. FFmpeg menambah kira-kira 0.8 GiB.

Dalam kedua-dua kes yang mengehadkan ialah **encoder**, bukan renderer: dengan tetapan
lalai, penulis menunggu x264 selama 111.5 s dan menunggu frame hanya 4.8 s.

## Render sahaja berbanding hujung ke hujung

Job rujukan, satu run setiap konfigurasi:

| Thread render | Render sahaja | Hujung ke hujung (x264 medium) |
|---:|---:|---:|
| 1 | 47.7 fps | 37.9 fps |
| 2 | 87.7 fps | 51.7 fps |
| 4 | 140.4 fps | 53.9 fps |
| 8 | 142.3 fps | 52.9 fps |

"Render sahaja" merangkumi evaluate, raster dan penukaran ke I420; frame dibuang.
Mesin ini ada 4 teras fizikal, jadi 8 thread hampir tidak menambah apa-apa. Renderer
sudah 4.7× masa nyata; hujung ke hujung terhenti pada kelajuan x264.

(Jadual ini dan tiga beban kerja pertama di bawah diukur selepas mesin berjalan lama dan
panas; angka job rujukan di atas diukur selepas but semula, sebab itu lebih tinggi.)

## Beban kerja lain

900 frame, 1920×1080, median dua run.

| Beban kerja | Apa | Render sahaja, 1 thread | Render sahaja, 8 thread | Hujung ke hujung, 8 thread |
|---|---|---:|---:|---:|
| `shapes` | 220 bentuk vektor beranimasi | 87.5 fps | 266.2 fps | 46.0 fps |
| `text-heavy` | 36 layer teks beranimasi | 179.7 fps | 313.6 fps | 68.8 fps |
| `static-spans` | Panel pegun, animasi pendek sekali-sekala | 780.9 fps | 1699.6 fps | 112.6 fps |
| `image-heavy` | 14 gambar berbayang yang bergerak, berputar dan diskala serentak | 21.6 fps | 81.4 fps | 40.7 fps |

- `static-spans`: 779 daripada 900 frame serupa dengan frame sebelumnya dan tidak dirender.
- `image-heavy` ialah kes paling berat: setiap frame mensampel semula 14 gambar yang
  berputar. Ia masih 2.7× masa nyata untuk render sahaja pada 8 thread.
- Dalam setiap beban kerja, frame adalah **bit-serupa** untuk 1, 2, 4 dan 8 thread
  (`--verify`).

## Preset x264 pada kualiti yang diukur

40 saat pertama job rujukan, CRF 18, dibandingkan dengan encode lossless bagi frame yang
sama:

| Preset | Throughput | Saiz | SSIM | PSNR |
|---|---:|---:|---:|---:|
| `medium` (lalai) | 58.3 fps | 6.3 MiB | 0.99799 | 53.7 dB |
| `fast` | 65.9 fps | 6.7 MiB | 0.99795 | 53.5 dB |
| `faster` | 69.7 fps | 6.6 MiB | 0.99762 | 52.6 dB |
| `veryfast` | 83.4 fps | 6.1 MiB | 0.99717 | 51.1 dB |
| `superfast` | 87.7 fps | 17.7 MiB | 0.99845 | 55.3 dB |

Semua jauh melebihi paras yang mata boleh bezakan daripada sumber. `veryfast` memberi
kira-kira 40% lebih throughput dengan kehilangan 2.6 dB. Lalai kekal `medium` kerana itulah
tetapan lalai Remotion pada CRF yang sama, jadi perbandingan nanti adil. `superfast`
menghasilkan fail hampir tiga kali lebih besar.

## Ke mana masa pergi

Satu thread, 1920×1080:

| Kerja | Kos |
|---|---:|
| Frame kosong: clear + tukar ke I420 (AVX2) | 2.0 ms |
| Sama, tanpa AVX2 (skalar) | 5.3 ms |
| Gradient linear skrin penuh, dilukis setiap frame | +8.6 ms |
| Gradient yang sama daripada cache layer | kira-kira +1 ms |
| Semua teks, logo SVG dan bentuk contoh `product-ad` | kira-kira +3 ms |
| Foto 760×760 yang sedang zum (bilinear) | kira-kira +6 ms |
| Radial gradient lutsinar 900 px yang bergerak | kira-kira +4 ms |

Kos sampling gambar Skia pada CPU, sepiksel destinasi (diukur pada mesin yang sedang
sibuk, jadi nisbahnya yang penting):

| Sampler | ns/piksel |
|---|---:|
| Salinan 1:1 pada grid piksel | 3 |
| Bilinear | 14–20 |
| Trilinear (mipmap) | 43–52 |
| Bicubic | 60 |

Inilah sebabnya gambar disediakan sekali pada saiz paparan terbesarnya: frame kemudian
menggunakan dua baris pertama jadual, bukan dua yang terakhir.

## Apa yang dibuat oleh pengoptimuman

Diukur sebelum dan selepas, satu thread, p50 masa frame:

| Perubahan | Sebelum | Selepas |
|---|---:|---:|
| Penukaran RGBA→I420 dengan AVX2 (frame kosong) | 5.3 ms | 2.0 ms |
| Cache layer + AVX2 pada `product-ad` | 37.5 ms | 17.4 ms |
| Bayang bentuk legap sebagai garis luar kabur, `image-heavy` | 76 ms | 57 ms |

## Ukur di mesin anda

```bash
nova doctor                                   # kuota CPU dan steal time dahulu
scripts/run-benchmarks.sh                     # semua beban kerja
THREADS=1,2,4,8,12,16 RUNS=5 scripts/run-benchmarks.sh
QUICK=1 scripts/run-benchmarks.sh             # job panjang dipendekkan kepada 20 s

nova benchmark templat.json --threads 4,8,16 --mode both --verify
nova render templat.json -o out.mp4 --json    # laporan penuh satu job
```

Selepas `nova render`, baris "limited by" memberitahu sisi mana yang lambat:

- **encoder**: tambah teras untuk x264, atau `--preset veryfast`. Menambah thread render
  tidak membantu.
- **renderer**: scene itu sendiri berat. Lihat "Menulis templat yang laju" dalam
  `docs/template-format.md`.

Pada VPS berkongsi, ulang ukuran pada masa berlainan jika `nova doctor` melaporkan steal
time yang tinggi.

## Langkah seterusnya jika encoder terus mengehadkan

Belum dilaksanakan, disenaraikan mengikut jangkaan pulangan:

1. **Encode berpecah.** Bahagikan garis masa kepada segmen, encode beberapa segmen serentak
   dengan proses FFmpeg berasingan, kemudian cantum tanpa encode semula. Berguna apabila
   teras lebih banyak daripada yang x264 boleh gunakan dengan cekap; tiada faedah pada
   mesin 4 teras ini kerana CPU sudah penuh.
2. **Agihan thread automatik** antara render dan x264 berdasarkan sisi yang menunggu.

## Frame bake: satu episod penuh

Diukur 16 November 2026 pada mesin lain daripada bahagian di atas: Intel Xeon Gold 6252
2.1 GHz, 16 vCPU, 28 GiB RAM, tanpa GPU. Video: satu episod animasi 1080×1920, 30 fps,
7,504 frame (4 min 10 s), ±1,500–2,000 elemen SVG sebingkai (rig watak prosedur, set 3D
perisian, lorekan cel melalui `clipPath`, filter CSS per watak, grain shader). Kod
sumbernya komposisi Remotion; lihat [`bake-remotion.md`](bake-remotion.md).

| Laluan | Masa | Nota |
|---|---:|---|
| Remotion (Chromium, `REMOTION_GL=swangle`, concurrency 3) | **±101 min** | daripada rekod render asal episod itu; tidak diukur semula di sini |
| NOVA: bundel | 0.4 s | esbuild |
| NOVA: bake (14 proses) | 74.8 s | 100.9 frame/s; pek 1.55 GB |
| NOVA: render (16 thread, x264 `medium`, CRF 18) | 316.3 s | 23.7 frame/s |
| **NOVA: jumlah, dari kod sumber ke MP4** | **6 min 32 s** | ±15× lebih laju; RAM puncak 1.27 GiB |

Setiap frame dalam render: 406 ms raster, 24 ms hurai SVG, 4.6 ms baca dan nyahmampat
(purata sethread, dengan 16 thread berjalan). Yang mengehadkan ialah raster, bukan encoder:
penulis menunggu frame, dan x264 hanya memakai 51 s.

Ke mana 406 ms itu pergi, pada satu frame biasa dalam babak dalaman (1,222 isian, 334
strok): isian ±120 ms, strok ±70 ms, lapisan filter ±75 ms, satu blur halus selebar
skrin ±37 ms, lapisan `opacity` kumpulan ±24 ms.

**Ketepatan.** MP4 NOVA dibanding dengan MP4 Remotion, kesemua 7,504 frame (kedua-duanya
sudah dimampat x264, dan grain ada pada kedua-duanya):

| | Purata | Median | Terendah |
|---|---:|---:|---:|
| PSNR | 35.7 dB | 35.1 dB | 27.9 dB |
| SSIM | 0.971 | 0.971 | 0.931 |

Frame terendah ialah close-up dengan zum kamera tinggi (anjakan beberapa piksel) dan
babak hujan (garis halus yang rawak terhadap pemampatan); tiada elemen yang hilang atau
salah lukis ditemui pada semakan visual 60+ frame merentas semua babak.

**Episod kedua, dengan audio.** Episod lain daripada siri yang sama, lebih berat: 7,247 frame
(4 min 2 s), ±25,000 elemen SVG sebingkai pada babak luar (pokok berakar gantung: 15,000 elips,
10,000 path), 262 kiu audio daripada 99 fail. Diukur hujung ke hujung termasuk campuran audio dan
master −14 LUFS:

| Laluan | Masa | Nota |
|---|---:|---|
| Remotion (Chromium, `REMOTION_GL=swangle`, concurrency 3) | **±76 min** | daripada rekod render asal; termasuk audio |
| NOVA: bundel | 0.5 s | |
| NOVA: bake (14 proses) | 56.5 s | 128 frame/s; pek 1.6 GB |
| NOVA: campur audio + master | 30.9 s | campur sahaja ±5 s; selebihnya `loudnorm` dan semakan puncak |
| NOVA: render (16 thread, x264 `medium`, CRF 18) | 380.3 s | 19.2 frame/s |
| **NOVA: jumlah, dari kod sumber ke MP4** | **7 min 48 s** | ±10× lebih laju; RAM puncak 1.5 GiB |

(Dua larian berturut-turut: 468.7 s dan 468.1 s. Episod pertama di atas, diulang dengan campuran
audio: 7 min 27 s.)

Ketepatan episod ini, kesemua 7,247 frame: PSNR purata 36.2 dB (median 35.6, terendah 27.6),
SSIM purata 0.972 (terendah 0.928). Audio berbanding MP4 Remotion: kelantangan −14.4 LUFS lawan
−14.5, beza aras median 0.07 dB, anjakan masa 0 ms.

Dua perkara yang ditemui oleh episod ini, kedua-duanya dalam bake:

- `@remotion/noise` hanya menyimpan 10 seed. Episod ini memakai ±13,000 seed sebingkai, jadi
  jadual hingar dibina semula pada hampir setiap panggilan: React sahaja ±2 s sebingkai, 65% di
  dalam `createNoise2D`. Bundel bake membesarkan cache itu (nilai yang sama): ±0.35 s sebingkai.
- Pembahagian frame yang tetap antara pekerja membuat satu pekerja ketinggalan pada babak berat
  (12 minit lawan 1–3 minit yang lain). Kerja kini diagih atas permintaan dalam julat 24 frame.

**Master 4K (`--scale 2 --profile master --also`).** Episod kedua yang sama, satu larian yang
menulis master 2160×3840 dan fail 1080p daripada frame yang sama. Diukur semasa mesin turut
menjalankan kerja lain yang berat (purata beban 50 pada 16 vCPU), jadi masa ini had atas:

| | Masa / saiz |
|---|---:|
| Bake + audio | 68 s + 34 s |
| Render 4K + dua pengekodan | 30 min 35 s (4.0 frame/s) |
| **Jumlah** | **32 min 17 s**; RAM puncak 5.0 GiB (termasuk x264) |
| Master 4K | 1.73 GB (57 Mbps) |
| Fail 1080p | 364 MB (12 Mbps) |

Satu frame 4K mengambil ±3.3× masa raster frame 1080p (bukan 4×: kerja laluan tidak
bergantung pada piksel). Berapa dekat setiap fail dengan frame yang dilukis, pada segmen 8 saat
yang sama (frame rujukan disimpan tanpa kehilangan):

| Fail | Lawan frame 1080p asal | Lawan frame 4K asal |
|---|---:|---:|
| 1080p, `balanced` (medium, CRF 18) | 43.5 dB / SSIM 0.972 | 38.9 dB / 0.967 (dibesarkan) |
| 1080p daripada larian 4K, `master` | 42.3 dB / 0.972 * | 39.7 dB / 0.970 (dibesarkan) |
| 4K, `master` | | **48.0 dB / 0.989** |

\* Fail ini bukan salinan frame 1080p: ia frame 4K yang disampel turun, jadi tepinya berbeza
(anti-alias lebih halus) dan PSNR terhadap render 1080p terus sedikit lebih rendah.

Master 4K 9 dB lebih dekat dengan imej yang dilukis berbanding fail 1080p biasa (ralat kuasa
±8× lebih kecil), dengan empat kali piksel. Fail 1080p hanya bertambah baik sedikit; yang
besar ialah 4K. Kandungan disahkan sama pada kesemua 7,247 frame (fail 1080p larian 4K lawan
render 1080p biasa: PSNR purata 43.8 dB, terendah 37.3 dB).

**`resvg` lawan pelukis Skia**, frame yang sama (±2,000 elemen), satu thread:

| Pelukis | Raster satu frame |
|---|---:|
| `resvg` 0.50 (tiny-skia) | 34.9 s |
| Skia, terjemahan terus | 0.80 s |
| Skia, dengan pengoptimuman dalam `bake-remotion.md` | 0.33 s |
