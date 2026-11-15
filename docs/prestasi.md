# Prestasi

Semua angka di bawah diukur, bukan dianggar, pada **satu mesin**: laptop AMD Ryzen 5 7520U
(4 teras / 8 thread, 15 W), 13 GiB RAM, AVX2, Ubuntu, FFmpeg 8.0.1, tanpa GPU. Tarikh
ukuran: 15 November 2026.

Dua perkara **belum** diukur, jadi tiada dakwaan mengenainya:

- **VPS sasaran (16 vCPU).** Jalankan `scripts/run-benchmarks.sh` di sana.
- **Perbandingan dengan Remotion.** Belum ada composition 2D yang sama dalam kedua-dua
  enjin. Lihat `docs/keputusan.md`.

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
