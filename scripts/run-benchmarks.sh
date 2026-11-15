#!/usr/bin/env bash
# Run the standard NOVA benchmark set and print one report.
#
#   scripts/run-benchmarks.sh                 # 1, half and all CPUs; 3 runs each
#   THREADS=1,2,4,8,12,16 RUNS=5 scripts/run-benchmarks.sh
#   QUICK=1 scripts/run-benchmarks.sh         # first 20 s of the long job only
#
# Render-only and end-to-end are measured separately and never mixed.
# Run it on an otherwise idle machine; check `nova doctor` for CPU steal first.
set -euo pipefail
cd "$(dirname "$0")/.."

NOVA=${NOVA:-./target/release/nova}
RUNS=${RUNS:-3}
THREADS=${THREADS:-}
[[ -x "$NOVA" ]] || { echo "build first: cargo build --release"; exit 1; }
[[ -f benchmarks/assets/photo1.jpg ]] || scripts/make-bench-assets.sh >/dev/null
[[ -f benchmarks/mixed-4min.json ]] || python3 benchmarks/generate.py >/dev/null

threads_arg=()
[[ -n "$THREADS" ]] && threads_arg=(--threads "$THREADS")

echo "# NOVA benchmark report — $(date -u +%Y-%m-%dT%H:%MZ)"
echo
"$NOVA" doctor
echo

for name in shapes text-heavy image-heavy static-spans; do
  echo "## $name"
  "$NOVA" benchmark "benchmarks/$name.json" --no-system-fonts --mode both --runs "$RUNS" "${threads_arg[@]}" --verify 2>/dev/null
  echo
done

echo "## mixed-4min (the reference job: 7200 frames, 1080p30)"
frames_arg=()
[[ -n "${QUICK:-}" ]] && frames_arg=(--frames 600)
# The long job: one run per configuration is enough, it averages 240 s of video.
"$NOVA" benchmark benchmarks/mixed-4min.json --no-system-fonts --mode both --runs 1 "${threads_arg[@]}" "${frames_arg[@]}" 2>/dev/null
