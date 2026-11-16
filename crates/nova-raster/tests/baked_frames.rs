//! Baked frame packs: a `frames` layer draws the pack's display list for
//! the current frame, with SVG semantics that plain layers do not have
//! (arbitrary clip paths, masks, filters) and shader passes.

mod common;

use std::io::Write;
use std::path::Path;

use common::*;
use serde_json::{Value, json};

const OP_SVG: u8 = 1;
const OP_SHADER: u8 = 2;

fn svg(body: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">{body}</svg>"##
    )
}

/// Write a pack (one shard) holding `frames`, each a list of operations.
fn write_pack(dir: &Path, frames: &[Vec<(u8, Vec<u8>)>], shaders: Value) {
    let mut shard = Vec::new();
    let mut index = Vec::new();
    for (f, ops) in frames.iter().enumerate() {
        let mut raw = Vec::new();
        for (kind, payload) in ops {
            raw.push(*kind);
            raw.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            raw.extend_from_slice(payload);
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(&raw).unwrap();
        let packed = z.finish().unwrap();
        index.push(json!([0, shard.len(), packed.len(), raw.len(), f]));
        shard.extend_from_slice(&packed);
    }
    std::fs::write(dir.join("frames-00.bin"), shard).unwrap();
    let manifest = json!({
        "nova_frames": 1, "width": W, "height": H, "fps": 30, "frames": frames.len(),
        "shards": ["frames-00.bin"], "shaders": shaders, "index": index
    });
    std::fs::write(dir.join("frames.json"), manifest.to_string()).unwrap();
}

fn pack_template() -> Value {
    template(
        json!([{ "id": "bake", "type": "frames", "src": "frames.json" }]),
        json!([{ "id": "ep", "type": "frames", "asset": "bake" }]),
    )
}

#[test]
fn each_frame_draws_its_own_document() {
    let dir = tempfile::tempdir().unwrap();
    let red = svg(r##"<rect width="320" height="360" fill="#ff0000"/>"##);
    let blue = svg(r##"<rect x="320" width="320" height="360" fill="#0000ff"/>"##);
    write_pack(
        dir.path(),
        &[vec![(OP_SVG, red.into_bytes())], vec![(OP_SVG, blue.into_bytes())]],
        json!({}),
    );
    let scene = compile_at(dir.path(), &pack_template());
    let bg = [0x14, 0x18, 0x2b, 255];

    let f0 = render(&scene, 0);
    assert_near(pixel(&f0, W, 100, 100), [255, 0, 0, 255], 0, "frame 0, left");
    assert_near(pixel(&f0, W, 500, 100), bg, 0, "frame 0, right is background");
    let f1 = render(&scene, 1);
    assert_near(pixel(&f1, W, 100, 100), bg, 0, "frame 1, left is background");
    assert_near(pixel(&f1, W, 500, 100), [0, 0, 255, 255], 0, "frame 1, right");
    // past the end of the pack the last frame is held
    assert_eq!(render(&scene, 50), f1);
}

#[test]
fn clip_paths_masks_opacity_and_filters_follow_svg() {
    let dir = tempfile::tempdir().unwrap();
    let doc = svg(
        r##"<defs>
          <clipPath id="c"><circle cx="100" cy="100" r="60"/></clipPath>
          <mask id="m"><rect x="300" y="0" width="60" height="360" fill="#fff"/></mask>
          <filter id="recolor" x="0" y="0" width="100%" height="100%" color-interpolation-filters="sRGB">
            <feColorMatrix type="matrix" values="0 0 0 0 0  1 0 0 0 0  0 0 0 0 0  0 0 0 1 0"/>
          </filter>
        </defs>
        <g clip-path="url(#c)"><rect width="640" height="360" fill="#ffffff"/></g>
        <rect x="240" y="40" width="160" height="80" fill="#ffffff" mask="url(#m)"/>
        <rect x="420" y="40" width="80" height="80" fill="#ffffff" opacity="0.5"/>
        <g filter="url(#recolor)"><rect x="520" y="40" width="80" height="80" fill="#ff0000"/></g>
        <rect x="40" y="240" width="200" height="60" fill="#00ff00" style="filter:brightness(0.5)"/>"##,
    );
    write_pack(dir.path(), &[vec![(OP_SVG, doc.into_bytes())]], json!({}));
    let scene = compile_at(dir.path(), &pack_template());
    let px = render(&scene, 0);
    let bg = [0x14, 0x18, 0x2b, 255];
    let white = [255, 255, 255, 255];

    // clip-path: an arbitrary shape, not just a rectangle
    assert_near(pixel(&px, W, 100, 100), white, 0, "inside the clip circle");
    assert_near(pixel(&px, W, 100, 30), bg, 0, "outside the clip circle");
    assert_near(pixel(&px, W, 150, 150), bg, 0, "outside the circle, inside its box");
    // mask: only the part under the white strip of the mask shows
    assert_near(pixel(&px, W, 330, 80), white, 0, "inside the mask");
    assert_near(pixel(&px, W, 270, 80), bg, 0, "outside the mask");
    // opacity on a single shape: half white over the background
    assert_near(pixel(&px, W, 460, 80), [138, 140, 149, 255], 2, "50% white");
    // feColorMatrix moves red into green
    assert_near(pixel(&px, W, 560, 80), [0, 255, 0, 255], 1, "recoloured");
    // CSS filter function
    assert_near(pixel(&px, W, 140, 270), [0, 128, 0, 255], 2, "brightness(0.5)");
}

#[test]
fn a_blur_reaches_beyond_its_content_and_stays_soft() {
    let dir = tempfile::tempdir().unwrap();
    // one light blur (the fast in-engine path) and one heavy blur (Skia's)
    let doc = svg(
        r##"<defs>
          <filter id="soft" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="2"/></filter>
          <filter id="wide" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="12"/></filter>
        </defs>
        <g filter="url(#soft)"><rect x="100" y="100" width="100" height="100" fill="#ffffff"/></g>
        <g filter="url(#wide)"><rect x="400" y="100" width="100" height="100" fill="#ffffff"/></g>"##,
    );
    write_pack(dir.path(), &[vec![(OP_SVG, doc.into_bytes())]], json!({}));
    let scene = compile_at(dir.path(), &pack_template());
    let px = render(&scene, 0);
    let bg = [0x14u8, 0x18, 0x2b, 255];
    for (x0, name, reach) in [(100u32, "light", 3u32), (400, "heavy", 14)] {
        assert_near(pixel(&px, W, x0 + 50, 150), [255, 255, 255, 255], 2, name);
        // on the edge a Gaussian is at half strength: the two pixels either
        // side of it average to half way between background and white
        let edge = (pixel(&px, W, x0 - 1, 150)[0] as u32 + pixel(&px, W, x0, 150)[0] as u32) / 2;
        assert!((130..=146).contains(&edge), "{name} blur edge is {edge}");
        // light spills outside the shape, and has faded out far from it
        let near = pixel(&px, W, x0 - reach, 150);
        assert!(near[0] > bg[0] + 3, "{name} blur does not spread: {near:?}");
        assert_near(pixel(&px, W, x0 - 60, 150), bg, 1, "far from the blur");
    }
}

#[test]
fn shader_passes_run_over_the_frame() {
    let dir = tempfile::tempdir().unwrap();
    let sksl = "uniform float2 res; uniform float level;
        half4 main(float2 xy) { return xy.x < res.x * 0.5 ? half4(level, 0.0, 0.0, 1.0) : half4(0.0); }";
    let op = json!({ "shader": "k1", "uniforms": { "res": [W, H], "level": 1.0 } });
    let base = svg(r##"<rect width="640" height="360" fill="#0000ff"/>"##);
    write_pack(
        dir.path(),
        &[vec![
            (OP_SVG, base.into_bytes()),
            (OP_SHADER, op.to_string().into_bytes()),
        ]],
        json!({ "k1": sksl }),
    );
    let scene = compile_at(dir.path(), &pack_template());
    let px = render(&scene, 0);
    assert_near(pixel(&px, W, 100, 100), [255, 0, 0, 255], 0, "shader covers the left half");
    assert_near(pixel(&px, W, 500, 100), [0, 0, 255, 255], 0, "transparent on the right");
    // the cached pass gives the same pixels as the first one
    assert_eq!(render(&scene, 0), px);
}

#[test]
fn broken_packs_are_rejected_before_rendering() {
    use nova_core::Limits;
    let dir = tempfile::tempdir().unwrap();
    let doc = svg(r##"<rect width="10" height="10" fill="#fff"/>"##);
    write_pack(dir.path(), &[vec![(OP_SVG, doc.into_bytes())]], json!({}));
    let load = |edit: &dyn Fn(&mut Value)| {
        let path = dir.path().join("frames.json");
        let original = std::fs::read_to_string(&path).unwrap();
        let mut m: Value = serde_json::from_str(&original).unwrap();
        edit(&mut m);
        std::fs::write(&path, m.to_string()).unwrap();
        let out = nova_template::load_value(&pack_template(), None, dir.path(), &Limits::default());
        std::fs::write(&path, original).unwrap();
        out.map(|_| ()).map_err(|e| format!("{e:?}"))
    };
    assert!(load(&|_| {}).is_ok());
    // an index entry that points past the end of its shard
    assert!(load(&|m| m["index"][0][2] = json!(9_999_999)).is_err());
    // a shard outside the pack folder
    assert!(load(&|m| m["shards"][0] = json!("../frames-00.bin")).is_err());
    // an unknown version
    assert!(load(&|m| m["nova_frames"] = json!(2)).is_err());
}

#[test]
fn text_skips_families_that_are_not_loaded_and_gradients_repeat() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("fonts")).unwrap();
    std::fs::copy(
        shared_assets().join("fonts/NotoSans-Regular.ttf"),
        dir.path().join("fonts/NotoSans-Regular.ttf"),
    )
    .unwrap();
    // A browser tries each family in turn; a missing first name must not drop the text.
    let doc = svg(
        r##"<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2="40" spreadMethod="repeat">
              <stop offset="0" stop-color="#000"/><stop offset="0.5" stop-color="#000"/>
              <stop offset="0.5" stop-color="#fff"/><stop offset="1" stop-color="#fff"/></linearGradient></defs>
            <text x="10" y="300" font-family="No Such Family, Noto Sans" font-size="300" fill="#ff0000">III</text>
            <rect x="320" width="320" height="360" fill="url(#g)"/>"##,
    );
    write_pack(dir.path(), &[vec![(OP_SVG, doc.into_bytes())]], json!({}));
    let scene = compile_at(
        dir.path(),
        &template(
            json!([
                { "id": "noto", "type": "font", "src": "fonts/NotoSans-Regular.ttf", "family": "Noto Sans" },
                { "id": "bake", "type": "frames", "src": "frames.json" }
            ]),
            json!([{ "id": "ep", "type": "frames", "asset": "bake" }]),
        ),
    );
    let px = render(&scene, 0);
    let red = (0..H)
        .flat_map(|y| (0..320).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = pixel(&px, W, x, y);
            p[0] > 200 && p[1] < 60
        })
        .count();
    assert!(red > 2000, "text drawn with the second family ({red} red pixels)");
    for (y, want) in [(10, 0), (30, 255), (50, 0), (110, 255), (330, 0)] {
        assert_near(pixel(&px, W, 400, y), [want, want, want, 255], 2, "repeating gradient band");
    }
}

#[test]
fn a_scaled_scene_redraws_vectors_instead_of_enlarging_them() {
    let dir = tempfile::tempdir().unwrap();
    // a hairline one unit wide, and a shader that paints odd columns of the pack red
    let doc = svg(r##"<rect x="100" y="0" width="1" height="360" fill="#ffffff"/>"##);
    let sksl = "half4 main(float2 xy) { return mod(floor(xy.x), 2.0) == 1.0 && xy.x > 400.0 ? half4(1.0, 0.0, 0.0, 1.0) : half4(0.0); }";
    let op = json!({ "shader": "cols", "uniforms": {} });
    write_pack(
        dir.path(),
        &[vec![(OP_SVG, doc.into_bytes()), (OP_SHADER, op.to_string().into_bytes())]],
        json!({ "cols": sksl }),
    );
    let scene = compile_at(dir.path(), &pack_template());
    let big = std::sync::Arc::new(scene.scaled(2.0));
    assert_eq!((big.width, big.height), (W * 2, H * 2));
    let px = render(&big, 0);
    let bg = [0x14, 0x18, 0x2b, 255];
    // the hairline is exactly two device pixels wide with hard edges: drawn at the
    // larger size, not a 1px line stretched (which would bleed into its neighbours)
    assert_near(pixel(&px, W * 2, 199, 300), bg, 0, "left of the line");
    assert_near(pixel(&px, W * 2, 200, 300), [255, 255, 255, 255], 0, "line, first pixel");
    assert_near(pixel(&px, W * 2, 201, 300), [255, 255, 255, 255], 0, "line, second pixel");
    assert_near(pixel(&px, W * 2, 202, 300), bg, 0, "right of the line");
    // the shader pass was made at the pack's resolution: its one-pixel columns are
    // two device pixels wide here (centre of a red column is red, of a clear one is not)
    let red = pixel(&px, W * 2, 2 * 501 + 1, 300);
    let clear = pixel(&px, W * 2, 2 * 502 + 1, 300);
    assert!(red[0] > 150 && red[2] < 120, "red column {red:?}");
    assert!(clear[0] < 120, "clear column {clear:?}");
}

#[test]
fn a_repeated_full_frame_gradient_covers_the_whole_scaled_frame() {
    // An overlay such as a vignette is cached the second time it is seen. The cached
    // picture must cover the device, not the pack's own size, when drawn at a scale.
    // (The pack is 1000x500 here: overlays are only cached above 400,000 pixels.)
    let (pw, ph) = (1000u32, 500u32);
    let dir = tempfile::tempdir().unwrap();
    let frames: Vec<Vec<(u8, Vec<u8>)>> = (0..4)
        .map(|i| {
            // the documents differ (a dot moves) so no frame is a copy of another; the gradient repeats
            let doc = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="{pw}" height="{ph}" viewBox="0 0 {pw} {ph}">
                    <defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="{pw}" y2="0">
                      <stop offset="0" stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/></linearGradient></defs>
                    <rect width="{pw}" height="{ph}" fill="url(#g)"/>
                    <circle cx="{}" cy="10" r="2" fill="#ff0000"/></svg>"##,
                20 + i * 10
            );
            vec![(OP_SVG, doc.into_bytes())]
        })
        .collect();
    write_pack(dir.path(), &frames, json!({}));
    let manifest = std::fs::read_to_string(dir.path().join("frames.json")).unwrap();
    let mut manifest: Value = serde_json::from_str(&manifest).unwrap();
    manifest["width"] = json!(pw);
    manifest["height"] = json!(ph);
    std::fs::write(dir.path().join("frames.json"), manifest.to_string()).unwrap();
    let mut t = pack_template();
    t["composition"]["width"] = json!(pw);
    t["composition"]["height"] = json!(ph);
    let scene = std::sync::Arc::new(compile_at(dir.path(), &t).scaled(2.0));
    assert_eq!((scene.width, scene.height), (pw * 2, ph * 2));
    let backend = backend(&scene);
    let mut renderer = nova_raster::RasterBackend::new_renderer(&backend).unwrap();
    let mut first = Vec::new();
    for f in 0..4 {
        let mut px = vec![0u8; scene.width as usize * scene.height as usize * 4];
        nova_raster::FrameRenderer::render(&mut *renderer, &scene.evaluate(f), &mut px).unwrap();
        // right and bottom of the frame, outside the pack's own 1000x500
        let bright = pixel(&px, pw * 2, 1900, 900);
        let mid = pixel(&px, pw * 2, 1000, 900);
        assert!(bright[0] > 220, "frame {f}: gradient reaches the right edge ({bright:?})");
        assert!((110..150).contains(&mid[0]), "frame {f}: gradient at the middle ({mid:?})");
        if f == 0 {
            first = px;
        } else {
            // the cached picture gives the same pixels as drawing the gradient
            assert_eq!(pixel(&px, pw * 2, 1900, 900), pixel(&first, pw * 2, 1900, 900));
        }
    }
}
