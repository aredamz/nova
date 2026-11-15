#!/usr/bin/env bash
# Generate the image assets used by benchmarks/ and examples/ with FFmpeg's
# built-in sources (nothing is downloaded). Safe to re-run.
set -euo pipefail
cd "$(dirname "$0")/.."
out=benchmarks/assets
mkdir -p "$out"

gradient() { # name seed c0 c1 c2 c3
  ffmpeg -v error -y -f lavfi -i "gradients=s=1600x1200:c0=$3:c1=$4:c2=$5:c3=$6:nb_colors=4:seed=$2:type=radial:speed=0.0001" \
    -frames:v 1 -q:v 2 "$out/$1.jpg"
}
gradient photo1 11 0xff6b6b 0xfeca57 0x48dbfb 0x1dd1a1
gradient photo2 23 0x5f27cd 0x341f97 0xff9ff3 0x00d2d3
gradient photo3 37 0x10ac84 0x2e86de 0xf368e0 0xff9f43
gradient photo4 41 0xee5253 0x222f3e 0x8395a7 0xc8d6e5
# Fine detail, to exercise resampling quality.
ffmpeg -v error -y -f lavfi -i "mandelbrot=s=1600x1200:start_scale=1.4:maxiter=120" -frames:v 1 -q:v 2 "$out/detail1.jpg"
ffmpeg -v error -y -f lavfi -i "testsrc2=s=1600x1200" -frames:v 1 "$out/chart.png"

cp examples/shared/fonts/NotoSans-Regular.ttf examples/shared/fonts/NotoSans-Bold.ttf "$out/"
cp "$out/photo1.jpg" examples/product-ad/assets/product.jpg
ls -la "$out"
