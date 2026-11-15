# Contoh

| Folder | Apa | Arahan |
|---|---|---|
| `product-ad/` | Iklan 1920x1080, 15 s. Data binding (`data.json`), SVG, gambar dengan zum perlahan, bayang, spring, garisan "dilukis", babak penutup. | `nova render examples/product-ad/template.json --data examples/product-ad/data.json -o out/product-ad.mp4` |
| `shorts/` | Video menegak 1080x1920, 10 s, dengan kapsyen bertukar dan bar kemajuan. Menggunakan aset daripada folder contoh lain, jadi akar aset ialah `examples/`. | `nova render examples/shorts/template.json --assets-root examples -o out/shorts.mp4` |
| `kes01/` | Episod penuh "Kes #01 — Jam Emas Dalam Gelap" (1080x1920, 123 s, tanpa suara): port daripada episod Remotion. 11 babak, watak dengan ekspresi/kelip mata/mulut, subtitle karaoke, kad suspek, polaroid, hujan. Templatnya dijana oleh skrip. | `python3 examples/kes01/generate.py && nova render examples/kes01/template.json -o out/kes01.mp4` |
| `shared/fonts/` | Noto Sans Regular dan Bold (SIL Open Font License 1.1). | |

Lihat seluruh video sekilas pandang sebelum render penuh:

```bash
nova sheet examples/shorts/template.json --assets-root examples --count 8 --columns 4 -o out/shorts-sheet.png
```
