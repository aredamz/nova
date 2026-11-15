//! Golden-image tests: render fixed scenes and compare with reviewed PNGs.
//!
//! `NOVA_UPDATE_GOLDEN=1 cargo test -p nova-raster --test golden` rewrites the
//! images. Look at every changed PNG before committing it; a golden is only
//! worth something if a human has seen it.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use serde_json::{Value, json};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Fraction of pixels allowed to differ noticeably. The rasteriser is pinned,
/// so on one platform the match is exact; the slack is for other CPUs, where
/// SIMD code paths may round a few antialiased edge pixels differently.
const MAX_BAD_PIXEL_FRACTION: f64 = 0.002;
const CHANNEL_TOLERANCE: u8 = 12;

fn check(name: &str, root: &Path, template: &Value, frame: u32) {
    let scene = compile_at(root, template);
    let actual = render(&scene, frame);
    let path = golden_dir().join(format!("{name}.png"));

    if std::env::var_os("NOVA_UPDATE_GOLDEN").is_some() {
        nova_raster::png::write_png(&path, &actual, scene.width, scene.height).unwrap();
        eprintln!("updated {}", path.display());
        return;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {} ({e}); run with NOVA_UPDATE_GOLDEN=1 and review it",
            path.display()
        )
    });
    let (expected, w, h) = nova_raster::png::decode_png(&bytes).unwrap();
    assert_eq!((w, h), (scene.width, scene.height), "{name}: size changed");

    let mut bad = 0usize;
    let mut worst = 0u8;
    for (a, e) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let d = a.iter().zip(e).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        worst = worst.max(d);
        if d > CHANNEL_TOLERANCE {
            bad += 1;
        }
    }
    let fraction = bad as f64 / (w as f64 * h as f64);
    if fraction > MAX_BAD_PIXEL_FRACTION {
        let failed = golden_dir().join(format!("{name}.actual.png"));
        nova_raster::png::write_png(&failed, &actual, w, h).unwrap();
        panic!(
            "{name}: {bad} pixels ({:.3}%) differ by more than {CHANNEL_TOLERANCE} (worst {worst}). \
             Rendered image saved to {} for comparison.",
            fraction * 100.0,
            failed.display()
        );
    }
}

#[test]
fn shapes() {
    let star = "M50 0 L61 35 L98 35 L68 57 L79 91 L50 70 L21 91 L32 57 L2 35 L39 35 Z";
    let layers = json!([
        { "id": "r1", "type": "rect", "size": { "width": 150, "height": 90 }, "fill": "#e94560",
          "transform": { "position": { "x": 30, "y": 30 } } },
        { "id": "r2", "type": "rect", "size": { "width": 150, "height": 90 }, "corner_radius": 24,
          "fill": { "type": "linear", "from": [0, 0], "to": [1, 1],
                    "stops": [{ "offset": 0, "color": "#00d2ff" }, { "offset": 1, "color": "#3a47d5" }] },
          "stroke": { "color": "#ffffff", "width": 4 },
          "transform": { "position": { "x": 215, "y": 30 } } },
        { "id": "r3", "type": "rect", "size": { "width": 120, "height": 120 }, "corner_radius": 12, "fill": "#f9c74f",
          "transform": { "position": { "x": 500, "y": 90 }, "rotation": 30, "anchor": { "x": 0.5, "y": 0.5 } } },
        { "id": "e1", "type": "ellipse", "size": { "width": 150, "height": 100 },
          "fill": { "type": "radial", "stops": [{ "offset": 0, "color": "#ffffff" }, { "offset": 1, "color": "#7b2ff7" }] },
          "transform": { "position": { "x": 30, "y": 160 } } },
        { "id": "e2", "type": "ellipse", "size": { "width": 110, "height": 110 },
          "stroke": { "color": "#90e0ef", "width": 10, "cap": "round", "trim_start": 0.1, "trim_end": 0.7 },
          "transform": { "position": { "x": 215, "y": 160 } } },
        { "id": "p1", "type": "path", "d": star, "size": { "width": 100, "height": 91 }, "fill": "#ff9f1c",
          "stroke": { "color": "#2ec4b6", "width": 3, "join": "round" },
          "transform": { "position": { "x": 420, "y": 250 }, "scale": 1.4, "rotation": -12, "anchor": { "x": 0.5, "y": 0.5 } } },
        { "id": "p2", "type": "path", "d": "M0 0 H80 V80 H0 Z M20 20 H60 V60 H20 Z", "fill_rule": "evenodd",
          "fill": "#80ed99", "transform": { "position": { "x": 540, "y": 250 } } },
        { "id": "line", "type": "path", "d": "M0 0 C60 -50 120 50 180 0", "stroke": { "color": "#ffffff", "width": 6, "cap": "round" },
          "transform": { "position": { "x": 30, "y": 320 } } }
    ]);
    check("shapes", &shared_assets(), &template(json!([]), layers), 0);
}

#[test]
fn typography() {
    let layers = json!([
        { "id": "h1", "type": "text", "transform": { "position": { "x": 24, "y": 12 } },
          "text": { "value": "NOVA Render", "font_family": "Noto Sans", "font_size": 54, "font_weight": 700, "color": "#ffffff" } },
        { "id": "body", "type": "text", "transform": { "position": { "x": 24, "y": 90 } },
          "text": { "value": "Teks panjang ini mesti dibalut dalam lebar yang ditetapkan, dengan jarak baris yang kemas — café, niño, über.",
                    "font_family": "Noto Sans", "font_size": 20, "color": "#c9d1ff", "line_height": 1.4, "max_width": 300 } },
        { "id": "centered", "type": "text", "transform": { "position": { "x": 480, "y": 130 }, "anchor": { "x": 0.5, "y": 0.5 } },
          "text": { "value": "Tengah\ndua baris", "font_family": "Noto Sans", "font_size": 30, "font_weight": 700,
                    "color": "#f9c74f", "align": "center" } },
        { "id": "right", "type": "text", "transform": { "position": { "x": 340, "y": 210 } },
          "text": { "value": "Kanan\nrata kanan", "font_family": "Noto Sans", "font_size": 22, "color": "#90e0ef",
                    "align": "right", "max_width": 280 } },
        { "id": "spaced", "type": "text", "transform": { "position": { "x": 24, "y": 250 } },
          "text": { "value": "LETTER SPACING", "font_family": "Noto Sans", "font_size": 18, "letter_spacing": 8, "color": "#e94560" } },
        { "id": "clamped", "type": "text", "transform": { "position": { "x": 24, "y": 290 } },
          "text": { "value": "Baris ini terlalu panjang untuk muat jadi ia dipotong dengan elipsis di hujungnya",
                    "font_family": "Noto Sans", "font_size": 18, "color": "#ffffff", "max_width": 300, "max_lines": 1 } },
        { "id": "rotated", "type": "text", "transform": { "position": { "x": 520, "y": 300 }, "rotation": -18, "scale": 1.3, "anchor": { "x": 0.5, "y": 0.5 } },
          "text": { "value": "Putar 18°", "font_family": "Noto Sans", "font_size": 26, "font_weight": 700, "color": "#80ed99" } }
    ]);
    check(
        "typography",
        &shared_assets(),
        &template(fonts(), layers),
        0,
    );
}

#[test]
fn effects_and_compositing() {
    let circle = |id: &str, x: f32, color: &str| {
        json!({ "id": id, "type": "ellipse", "size": { "width": 90, "height": 90 }, "fill": color,
                "transform": { "position": { "x": x, "y": 0 } } })
    };
    let layers = json!([
        { "id": "bg", "type": "rect", "size": { "width": 640, "height": 360 },
          "fill": { "type": "linear", "from": [0, 0], "to": [0, 1],
                    "stops": [{ "offset": 0, "color": "#1b2a49" }, { "offset": 1, "color": "#3d1e6d" }] } },
        // Group opacity: the overlap must NOT be darker than the rest.
        { "id": "g-opacity", "type": "group", "opacity": 0.5, "transform": { "position": { "x": 20, "y": 20 } },
          "layers": [circle("a1", 0.0, "#ffffff"), circle("a2", 50.0, "#ffffff")] },
        // Same shapes with per-layer opacity: here the overlap IS brighter.
        { "id": "g-plain", "type": "group", "transform": { "position": { "x": 200, "y": 20 } },
          "layers": [
            { "id": "b1", "type": "ellipse", "size": { "width": 90, "height": 90 }, "fill": "#ffffff", "opacity": 0.5 },
            { "id": "b2", "type": "ellipse", "size": { "width": 90, "height": 90 }, "fill": "#ffffff", "opacity": 0.5,
              "transform": { "position": { "x": 50, "y": 0 } } } ] },
        { "id": "shadowed", "type": "rect", "size": { "width": 150, "height": 80 }, "corner_radius": 16, "fill": "#f9c74f",
          "shadow": { "x": 10, "y": 14, "blur": 24, "color": "rgba(0,0,0,0.6)" },
          "transform": { "position": { "x": 420, "y": 30 } } },
        { "id": "blurred", "type": "rect", "size": { "width": 120, "height": 80 }, "fill": "#e94560", "blur": 8,
          "transform": { "position": { "x": 30, "y": 160 } } },
        { "id": "blend-base", "type": "rect", "size": { "width": 130, "height": 90 }, "fill": "#2ec4b6",
          "transform": { "position": { "x": 200, "y": 150 } } },
        { "id": "blend-mult", "type": "ellipse", "size": { "width": 110, "height": 110 }, "fill": "#ff9f1c", "blend_mode": "multiply",
          "transform": { "position": { "x": 260, "y": 180 } } },
        { "id": "blend-screen", "type": "ellipse", "size": { "width": 70, "height": 70 }, "fill": "#7b2ff7", "blend_mode": "screen",
          "transform": { "position": { "x": 180, "y": 200 } } },
        // Clipped group: the big circle is cut to the 120x90 box.
        { "id": "clipped", "type": "group", "size": { "width": 120, "height": 90 }, "clip": true,
          "transform": { "position": { "x": 440, "y": 170 } },
          "layers": [
            { "id": "c-bg", "type": "rect", "size": { "width": 120, "height": 90 }, "fill": "#0b132b" },
            { "id": "c-big", "type": "ellipse", "size": { "width": 160, "height": 160 }, "fill": "#80ed99",
              "transform": { "position": { "x": 30, "y": 20 } } } ] },
        { "id": "text-shadow", "type": "text", "opacity": 0.9, "transform": { "position": { "x": 30, "y": 290 } },
          "shadow": { "x": 0, "y": 4, "blur": 10, "color": "#000000" },
          "text": { "value": "Bayang & legap", "font_family": "Noto Sans", "font_size": 34, "font_weight": 700, "color": "#ffffff" } }
    ]);
    check("effects", &shared_assets(), &template(fonts(), layers), 0);
}

/// A 64x48 test image: colour quadrants with a translucent centre.
fn write_test_png(path: &Path) {
    let (w, h) = (64u32, 48u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let c: [u8; 4] = match (x < w / 2, y < h / 2) {
                (true, true) => [230, 57, 70, 255],
                (false, true) => [42, 157, 143, 255],
                (true, false) => [233, 196, 106, 255],
                (false, false) => [38, 70, 83, 255],
            };
            let centre = (x as i32 - 32).abs() < 10 && (y as i32 - 24).abs() < 8;
            px.extend_from_slice(&if centre { [255, 255, 255, 96] } else { c });
        }
    }
    nova_raster::png::write_png(path, &px, w, h).unwrap();
}

#[test]
fn images_and_svg() {
    let dir = tempfile::tempdir().unwrap();
    write_test_png(&dir.path().join("quad.png"));
    std::fs::write(
        dir.path().join("logo.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48">
             <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
               <stop offset="0" stop-color="#ffd166"/><stop offset="1" stop-color="#ef476f"/></linearGradient></defs>
             <circle cx="24" cy="24" r="22" fill="url(#g)" stroke="#fff" stroke-width="2"/>
             <path d="M14 25 l7 7 l14 -16" fill="none" stroke="#073b4c" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>
           </svg>"##,
    )
    .unwrap();
    let assets = json!([
        { "id": "quad", "type": "image", "src": "quad.png" },
        { "id": "logo", "type": "svg", "src": "logo.svg" }
    ]);
    let frame = |id: &str, x: f32, y: f32| {
        json!({ "id": id, "type": "rect", "size": { "width": 140, "height": 140 }, "fill": "#0b132b",
                "stroke": { "color": "#3a506b", "width": 1 }, "transform": { "position": { "x": x, "y": y } } })
    };
    let img = |id: &str, x: f32, y: f32, fit: &str| {
        json!({ "id": id, "type": "image", "asset": "quad", "size": { "width": 140, "height": 140 }, "fit": fit,
                "transform": { "position": { "x": x, "y": y } } })
    };
    let layers = json!([
        frame("f1", 20.0, 20.0), img("contain", 20.0, 20.0, "contain"),
        frame("f2", 175.0, 20.0), img("cover", 175.0, 20.0, "cover"),
        frame("f3", 330.0, 20.0), img("fill", 330.0, 20.0, "fill"),
        frame("f4", 485.0, 20.0), img("none", 485.0, 20.0, "none"),
        // SVG: natural size, then scaled up 4x — must stay sharp, not blurry.
        { "id": "svg-small", "type": "svg", "asset": "logo", "transform": { "position": { "x": 30, "y": 200 } } },
        { "id": "svg-big", "type": "svg", "asset": "logo",
          "transform": { "position": { "x": 220, "y": 265 }, "scale": 3, "rotation": 15, "anchor": { "x": 0.5, "y": 0.5 } } },
        { "id": "img-rot", "type": "image", "asset": "quad", "opacity": 0.8,
          "transform": { "position": { "x": 480, "y": 265 }, "scale": 2.5, "rotation": -20, "anchor": { "x": 0.5, "y": 0.5 } } }
    ]);
    check("images", dir.path(), &template(assets, layers), 0);
}

#[test]
fn captions() {
    let cap = |id: &str, y: f32, revealed: u32| {
        json!({ "id": id, "type": "text",
          "transform": { "position": { "x": 320, "y": y }, "anchor": { "x": 0.5, "y": 0.5 } },
          "text": { "value": "Lima minit bekalan elektrik terputus...", "font_family": "Noto Sans", "font_size": 26,
                    "font_weight": 700, "color": "#f7f4ee", "align": "center", "line_height": 1.28,
                    "max_width": 420, "hug": true,
                    "spans": [{ "start": 11, "end": 27, "color": "#f2c14e" }],
                    "reveal": { "count": revealed, "hidden_color": "rgba(255,255,255,0.3)" },
                    "background": { "color": "rgba(8,10,16,0.72)", "radius": 14, "padding": { "x": 20, "y": 12 } } } })
    };
    let layers = json!([
        { "id": "bg", "type": "rect", "size": { "width": 640, "height": 360 },
          "fill": { "type": "linear", "from": [0, 0], "to": [1, 1],
                    "stops": [{ "offset": 0, "color": "#3a4a6a" }, { "offset": 1, "color": "#6a3a4a" }] } },
        cap("none", 50.0, 0), cap("some", 130.0, 3), cap("all", 210.0, 9),
        // a tag and a stamp: panels with padding, radius and border
        { "id": "tag", "type": "text", "transform": { "position": { "x": 30, "y": 290 } },
          "text": { "value": "MAK CIK ROSNAH", "font_family": "Noto Sans", "font_size": 22, "font_weight": 700, "color": "#1a1c23",
                    "background": { "color": "#c08bff", "radius": 6, "padding": { "left": 14, "top": 2, "right": 14, "bottom": 2 } } } },
        { "id": "stamp", "type": "text", "shadow": { "y": 4, "blur": 12, "color": "rgba(0,0,0,0.35)" },
          "transform": { "position": { "x": 470, "y": 305 }, "anchor": { "x": 0.5, "y": 0.5 }, "rotation": -8 },
          "text": { "value": "KOSONG", "font_family": "Noto Sans", "font_size": 30, "font_weight": 700, "color": "#ff5a4e",
                    "letter_spacing": 3,
                    "background": { "color": "rgba(255,250,240,0.9)", "radius": 8, "padding": { "x": 16, "y": 4 },
                                    "border_color": "#ff5a4e", "border_width": 5 } } },
        // typewriter: characters appear, the rest stay invisible
        { "id": "typed", "type": "text", "transform": { "position": { "x": 250, "y": 285 } },
          "text": { "value": "FAIL KES #01", "font_family": "Noto Sans", "font_size": 24, "color": "#ffffff",
                    "reveal": { "unit": "char", "count": 8 } } }
    ]);
    check("captions", &shared_assets(), &template(fonts(), layers), 0);
}
