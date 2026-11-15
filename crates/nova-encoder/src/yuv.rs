//! RGBA -> planar YUV 4:2:0 (I420), BT.709, limited ("TV") range.
//!
//! Done on the render threads, not by FFmpeg: the conversion runs in parallel
//! with rendering, the pipe carries 1.5 bytes per pixel instead of 4, and the
//! colour matrix is ours to define — it matches the `bt709` tags written into
//! the file, so players show the colours that were rendered.

/// Bytes in an I420 frame. Width and height must be even.
pub fn i420_len(width: usize, height: usize) -> usize {
    width * height * 3 / 2
}

// BT.709 R'G'B' -> Y'CbCr, limited range, 16-bit fixed point.
//   Y  = 16  + 219/255 * ( 0.2126 R + 0.7152 G + 0.0722 B)
//   Cb = 128 + 224/255 * (-0.114572 R - 0.385428 G + 0.5 B)
//   Cr = 128 + 224/255 * ( 0.5 R - 0.454153 G - 0.045847 B)
// The chroma rows each sum to zero, so every grey maps to exactly (128, 128).
const YR: i32 = 11966;
const YG: i32 = 40254;
const YB: i32 = 4064;
const UR: i32 = -6596;
const UG: i32 = -22189;
const UB: i32 = 28785;
const VR: i32 = 28785;
const VG: i32 = -26145;
const VB: i32 = -2640;

#[inline(always)]
fn luma_row(rgba: &[u8], y: &mut [u8]) {
    const OFFSET: i32 = (16 << 16) + (1 << 15);
    for (px, out) in rgba.chunks_exact(4).zip(y.iter_mut()) {
        let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
        *out = ((YR * r + YG * g + YB * b + OFFSET) >> 16) as u8;
    }
}

/// Chroma for `u.len()` 2x2 blocks: the average of the four pixels.
#[inline(always)]
fn chroma_row(top: &[u8], bottom: &[u8], u: &mut [u8], v: &mut [u8]) {
    // 128 in the 18-bit fixed point used for 4-sample sums, plus rounding.
    const OFFSET: i32 = (128 << 18) + (1 << 17);
    let blocks = top.chunks_exact(8).zip(bottom.chunks_exact(8));
    for ((t, b), (u, v)) in blocks.zip(u.iter_mut().zip(v.iter_mut())) {
        let r = t[0] as i32 + t[4] as i32 + b[0] as i32 + b[4] as i32;
        let g = t[1] as i32 + t[5] as i32 + b[1] as i32 + b[5] as i32;
        let bl = t[2] as i32 + t[6] as i32 + b[2] as i32 + b[6] as i32;
        *u = ((UR * r + UG * g + UB * bl + OFFSET) >> 18) as u8;
        *v = ((VR * r + VG * g + VB * bl + OFFSET) >> 18) as u8;
    }
}

/// One pair of rows, portable code. This is the reference implementation:
/// the SIMD path must produce exactly the same bytes.
fn convert_row_pair_scalar(
    top: &[u8],
    bottom: &[u8],
    y_top: &mut [u8],
    y_bottom: &mut [u8],
    u: &mut [u8],
    v: &mut [u8],
) {
    luma_row(top, y_top);
    luma_row(bottom, y_bottom);
    chroma_row(top, bottom, u, v);
}

/// One pair of rows with AVX2, 8 pixels per step. Returns how many pixels
/// (a multiple of 8) were converted; the caller finishes the rest.
///
/// Uses the same 16-bit fixed-point arithmetic as the scalar code, so the
/// output is bit-identical on machines with and without AVX2.
///
/// # Safety
/// The CPU must support AVX2 and the slices must have the lengths
/// `convert_row_pair_scalar` expects for the same width.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn convert_row_pair_avx2(
    top: &[u8],
    bottom: &[u8],
    y_top: &mut [u8],
    y_bottom: &mut [u8],
    u: &mut [u8],
    v: &mut [u8],
) -> usize {
    use std::arch::x86_64::*;
    let width = y_top.len();
    let steps = width / 8;
    // SAFETY: every load reads 32 bytes at pixel offset i*8 < width (so within
    // the `width * 4` byte rows) and every store writes inside the output rows.
    unsafe {
        let byte_mask = _mm256_set1_epi32(0x00FF_00FF);
        // 16-bit lanes are [R, B] and [G, A] per pixel. YG does not fit in an
        // i16, so half of it is used and the product doubled (YG is even).
        let y_rb = _mm256_set1_epi32(((YB as u32) << 16 | YR as u32) as i32);
        let y_g = _mm256_set1_epi32(YG / 2);
        let y_off = _mm256_set1_epi32((16 << 16) + (1 << 15));
        let pack = |lo: i32, hi: i32| ((hi as u16 as u32) << 16 | lo as u16 as u32) as i32;
        let u_rb = _mm256_set1_epi32(pack(UR, UB));
        let u_g = _mm256_set1_epi32(pack(UG, 0));
        let v_rb = _mm256_set1_epi32(pack(VR, VB));
        let v_g = _mm256_set1_epi32(pack(VG, 0));
        let c_off = _mm256_set1_epi32((128 << 18) + (1 << 17));

        for i in 0..steps {
            let t = _mm256_loadu_si256(top.as_ptr().add(i * 32) as *const __m256i);
            let b = _mm256_loadu_si256(bottom.as_ptr().add(i * 32) as *const __m256i);
            let (t_rb, t_ga) = (
                _mm256_and_si256(t, byte_mask),
                _mm256_and_si256(_mm256_srli_epi16(t, 8), byte_mask),
            );
            let (b_rb, b_ga) = (
                _mm256_and_si256(b, byte_mask),
                _mm256_and_si256(_mm256_srli_epi16(b, 8), byte_mask),
            );

            // ---- luma: one value per pixel -----------------------------------
            let luma = |rb: __m256i, ga: __m256i| {
                let sum = _mm256_add_epi32(
                    _mm256_add_epi32(
                        _mm256_madd_epi16(rb, y_rb),
                        _mm256_slli_epi32(_mm256_madd_epi16(ga, y_g), 1),
                    ),
                    y_off,
                );
                let y32 = _mm256_srli_epi32(sum, 16);
                let y16 = _mm256_packus_epi32(y32, y32);
                _mm256_packus_epi16(y16, y16)
            };
            let (yt, yb) = (luma(t_rb, t_ga), luma(b_rb, b_ga));
            // After the packs each 128-bit half holds its four bytes in its low dword.
            let store4 = |dst: *mut u8, v: __m256i| {
                (dst as *mut i32).write_unaligned(_mm256_extract_epi32(v, 0));
                (dst.add(4) as *mut i32).write_unaligned(_mm256_extract_epi32(v, 4));
            };
            store4(y_top.as_mut_ptr().add(i * 8), yt);
            store4(y_bottom.as_mut_ptr().add(i * 8), yb);

            // ---- chroma: sum each 2x2 block, then apply the matrix -----------
            let (rb, ga) = (_mm256_add_epi16(t_rb, b_rb), _mm256_add_epi16(t_ga, b_ga));
            let u_px = _mm256_add_epi32(_mm256_madd_epi16(rb, u_rb), _mm256_madd_epi16(ga, u_g));
            let v_px = _mm256_add_epi32(_mm256_madd_epi16(rb, v_rb), _mm256_madd_epi16(ga, v_g));
            // hadd adds horizontal neighbours: per half -> [u01, u23, v01, v23]
            let uv = _mm256_srai_epi32(_mm256_add_epi32(_mm256_hadd_epi32(u_px, v_px), c_off), 18);
            let uv16 = _mm256_packus_epi32(uv, uv);
            let uv8 = _mm256_packus_epi16(uv16, uv16);
            let lo = _mm256_extract_epi32(uv8, 0) as u32; // bytes: u0 u1 v0 v1
            let hi = _mm256_extract_epi32(uv8, 4) as u32; // bytes: u2 u3 v2 v3
            let u_dst = u.as_mut_ptr().add(i * 4);
            let v_dst = v.as_mut_ptr().add(i * 4);
            (u_dst as *mut u16).write_unaligned(lo as u16);
            (u_dst.add(2) as *mut u16).write_unaligned(hi as u16);
            (v_dst as *mut u16).write_unaligned((lo >> 16) as u16);
            (v_dst.add(2) as *mut u16).write_unaligned((hi >> 16) as u16);
        }
    }
    steps * 8
}

fn convert(rgba: &[u8], width: usize, height: usize, out: &mut [u8], simd: bool) {
    assert!(
        width.is_multiple_of(2) && height.is_multiple_of(2),
        "I420 needs even dimensions"
    );
    assert_eq!(rgba.len(), width * height * 4, "RGBA buffer size");
    assert_eq!(out.len(), i420_len(width, height), "I420 buffer size");

    let (y_plane, chroma) = out.split_at_mut(width * height);
    let (u_plane, v_plane) = chroma.split_at_mut(width * height / 4);
    let cw = width / 2;
    #[cfg(target_arch = "x86_64")]
    let use_avx2 = simd && std::arch::is_x86_feature_detected!("avx2");
    #[cfg(not(target_arch = "x86_64"))]
    let _ = simd;

    let rows = rgba.chunks_exact(width * 8);
    let y_rows = y_plane.chunks_exact_mut(width * 2);
    let u_rows = u_plane.chunks_exact_mut(cw);
    let v_rows = v_plane.chunks_exact_mut(cw);
    for (((pair, y_pair), u_row), v_row) in rows.zip(y_rows).zip(u_rows).zip(v_rows) {
        let (top, bottom) = pair.split_at(width * 4);
        let (y_top, y_bottom) = y_pair.split_at_mut(width);
        #[allow(unused_mut)]
        let mut done = 0;
        #[cfg(target_arch = "x86_64")]
        if use_avx2 {
            // SAFETY: AVX2 was detected above; slice lengths follow from `width`.
            done = unsafe { convert_row_pair_avx2(top, bottom, y_top, y_bottom, u_row, v_row) };
        }
        // Columns the SIMD step did not cover (all of them without AVX2).
        convert_row_pair_scalar(
            &top[done * 4..],
            &bottom[done * 4..],
            &mut y_top[done..],
            &mut y_bottom[done..],
            &mut u_row[done / 2..],
            &mut v_row[done / 2..],
        );
    }
}

/// Convert tightly packed RGBA8 (alpha ignored) into `out` as I420.
///
/// Chroma is the average of each 2x2 block. Uses AVX2 when the CPU has it;
/// the result is the same bytes either way.
///
/// # Panics
/// If the dimensions are odd or the buffers have the wrong length.
pub fn rgba_to_i420(rgba: &[u8], width: usize, height: usize, out: &mut [u8]) {
    convert(rgba, width, height, out, true);
}

/// The portable implementation, for testing and comparison.
pub fn rgba_to_i420_scalar(rgba: &[u8], width: usize, height: usize, out: &mut [u8]) {
    convert(rgba, width, height, out, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert_flat(rgb: [u8; 3]) -> (u8, u8, u8) {
        let (w, h) = (4, 2);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect();
        let mut out = vec![0u8; i420_len(w, h)];
        rgba_to_i420(&rgba, w, h, &mut out);
        assert!(out[..w * h].iter().all(|&y| y == out[0]), "flat luma");
        (out[0], out[w * h], out[w * h + w * h / 4])
    }

    #[test]
    fn reference_colours_match_bt709_limited() {
        // Expected values computed from the BT.709 equations in floating point.
        let cases: [([u8; 3], (u8, u8, u8)); 8] = [
            ([0, 0, 0], (16, 128, 128)),
            ([255, 255, 255], (235, 128, 128)),
            ([255, 0, 0], (63, 102, 240)),
            ([0, 255, 0], (173, 42, 26)),
            ([0, 0, 255], (32, 240, 118)),
            ([128, 128, 128], (126, 128, 128)),
            ([255, 255, 0], (219, 16, 138)),
            ([16, 21, 34], (34, 134, 125)),
        ];
        for (rgb, (y, u, v)) in cases {
            let got = convert_flat(rgb);
            assert!(
                got.0.abs_diff(y) <= 1 && got.1.abs_diff(u) <= 1 && got.2.abs_diff(v) <= 1,
                "{rgb:?}: got {got:?}, expected ({y}, {u}, {v})"
            );
        }
    }

    #[test]
    fn every_grey_has_neutral_chroma_and_legal_luma() {
        for level in 0..=255u8 {
            let (y, u, v) = convert_flat([level, level, level]);
            assert_eq!((u, v), (128, 128), "grey {level}");
            assert!((16..=235).contains(&y));
        }
    }

    #[test]
    fn output_stays_in_legal_range_for_extreme_colours() {
        for r in [0u8, 255] {
            for g in [0u8, 255] {
                for b in [0u8, 255] {
                    let (y, u, v) = convert_flat([r, g, b]);
                    assert!(
                        (16..=235).contains(&y)
                            && (16..=240).contains(&u)
                            && (16..=240).contains(&v),
                        "{r},{g},{b}"
                    );
                }
            }
        }
    }

    #[test]
    fn chroma_averages_each_2x2_block() {
        // 2x2 image: two red and two blue pixels -> chroma of their average.
        let rgba = [
            255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255, 255, 0, 0, 255,
        ];
        let mut out = vec![0u8; i420_len(2, 2)];
        rgba_to_i420(&rgba, 2, 2, &mut out);
        let (_, u_avg, v_avg) = convert_flat([128, 0, 128]);
        assert!(out[4].abs_diff(u_avg) <= 1 && out[5].abs_diff(v_avg) <= 1);
        // luma is per pixel, not averaged
        assert_eq!(out[0], out[3]);
        assert_ne!(out[0], out[1]);
    }

    #[test]
    fn plane_layout_is_y_then_u_then_v() {
        let (w, h) = (4, 4);
        let mut rgba = vec![0u8; w * h * 4];
        // make only the bottom-right 2x2 block red
        for y in 2..4 {
            for x in 2..4 {
                rgba[(y * w + x) * 4] = 255;
            }
        }
        let mut out = vec![0u8; i420_len(w, h)];
        rgba_to_i420(&rgba, w, h, &mut out);
        let (u, v) = (&out[16..20], &out[20..24]);
        assert_eq!(&u[..3], &[128, 128, 128]);
        assert!(
            u[3] < 110 && v[3] > 230,
            "red block lands in the last chroma sample"
        );
    }

    #[test]
    #[should_panic(expected = "even dimensions")]
    fn odd_sizes_are_rejected() {
        rgba_to_i420(&[0; 12], 3, 1, &mut [0; 4]);
    }

    #[test]
    fn simd_and_scalar_produce_identical_bytes() {
        // Pseudo-random pixels, several widths including ones that leave a
        // scalar tail after the 8-pixel SIMD steps.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 24) as u8
        };
        for (w, h) in [
            (2, 2),
            (6, 4),
            (8, 2),
            (10, 6),
            (16, 8),
            (34, 4),
            (1920, 4),
            (126, 10),
        ] {
            let rgba: Vec<u8> = (0..w * h * 4).map(|_| next()).collect();
            let mut fast = vec![0u8; i420_len(w, h)];
            let mut reference = vec![1u8; i420_len(w, h)];
            rgba_to_i420(&rgba, w, h, &mut fast);
            rgba_to_i420_scalar(&rgba, w, h, &mut reference);
            assert_eq!(fast, reference, "{w}x{h}");
        }
        // Extremes: all-white and all-black rows hit the coefficient limits.
        for value in [0u8, 255] {
            let rgba = vec![value; 64 * 2 * 4];
            let mut fast = vec![0u8; i420_len(64, 2)];
            let mut reference = vec![1u8; i420_len(64, 2)];
            rgba_to_i420(&rgba, 64, 2, &mut fast);
            rgba_to_i420_scalar(&rgba, 64, 2, &mut reference);
            assert_eq!(fast, reference);
        }
    }
}
