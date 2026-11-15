//! The layer cache must be invisible: same picture with it on or off, and it
//! must actually be used where the policy says so.

mod common;

use std::sync::Arc;

use common::*;
use nova_core::Scene;
use nova_raster::{PrepareOptions, RasterBackend, RendererStats, SkiaBackend};
use serde_json::{Value, json};

fn render_with(
    scene: &Arc<Scene>,
    frames: &[u32],
    layer_cache: bool,
) -> (Vec<Vec<u8>>, RendererStats, usize) {
    let opts = PrepareOptions {
        system_fonts: false,
        layer_cache,
        ..PrepareOptions::default()
    };
    let (backend, report) =
        SkiaBackend::prepare(Arc::clone(scene), &opts).unwrap_or_else(|e| panic!("{e:?}"));
    let mut renderer = backend.new_renderer().unwrap();
    let out = frames
        .iter()
        .map(|&f| {
            let mut rgba = vec![0u8; (W * H * 4) as usize];
            renderer.render(&scene.evaluate(f), &mut rgba).unwrap();
            rgba
        })
        .collect();
    (out, renderer.stats(), report.cached_layers)
}

/// Largest per-channel difference, mean absolute difference and mean signed
/// difference (a non-zero signed mean would be a systematic colour shift).
fn difference(a: &[u8], b: &[u8]) -> (u8, f64, f64) {
    let (mut worst, mut abs, mut signed) = (0u8, 0u64, 0i64);
    for (p, q) in a.iter().zip(b) {
        worst = worst.max(p.abs_diff(*q));
        abs += p.abs_diff(*q) as u64;
        signed += *p as i64 - *q as i64;
    }
    (
        worst,
        abs as f64 / a.len() as f64,
        signed as f64 / a.len() as f64,
    )
}

/// A scene built to exercise every caching rule at once.
fn scene() -> Arc<Scene> {
    let layers: Value = json!([
        // static + gradient -> cached for the whole video
        { "id": "backdrop", "type": "rect", "size": { "width": 640, "height": 360 },
          "fill": { "type": "linear", "from": [0, 0], "to": [1, 1],
                    "stops": [{ "offset": 0, "color": "#1b2a49" }, { "offset": 1, "color": "#3d1e6d" }] } },
        // static, cheap -> drawn directly
        { "id": "plain", "type": "rect", "size": { "width": 80, "height": 40 }, "fill": "#2ec4b6",
          "transform": { "position": { "x": 20, "y": 300 } } },
        // static group with a shadowed, rotated child -> cached as one unit
        { "id": "card", "type": "group", "transform": { "position": { "x": 420, "y": 60 }, "rotation": 6 },
          "layers": [
            { "id": "card-bg", "type": "rect", "size": { "width": 180, "height": 110 }, "corner_radius": 18, "fill": "#f9c74f",
              "shadow": { "x": 6, "y": 10, "blur": 22, "color": "rgba(0,0,0,0.6)" } },
            { "id": "card-text", "type": "text", "transform": { "position": { "x": 16, "y": 30 } },
              "text": { "value": "Statik", "font_family": "Noto Sans", "font_size": 34, "font_weight": 700, "color": "#14182b" } } ] },
        // shadowed node that moves for 20 frames, then holds -> cached during the hold
        { "id": "mover", "type": "ellipse", "size": { "width": 90, "height": 90 }, "fill": "#e94560", "opacity": 0.85,
          "shadow": { "y": 8, "blur": 18, "color": "#000000" },
          "transform": { "position": { "x": 60, "y": 60 } },
          "animations": [{ "property": "transform.position.x",
            "keyframes": [{ "frame": 0, "value": 60, "easing": "ease_out_cubic" }, { "frame": 20, "value": 240 }] }] },
        // blurred node that never stops changing -> not cached (would always miss)
        { "id": "restless", "type": "rect", "size": { "width": 70, "height": 70 }, "fill": "#90e0ef", "blur": 5,
          "transform": { "position": { "x": 300, "y": 240 }, "anchor": { "x": 0.5, "y": 0.5 } },
          "animations": [{ "property": "transform.rotation", "keyframes": [{ "frame": 0, "value": 0 }, { "frame": 120, "value": 360 }] }] },
        // static blend-mode layer -> cached, still blends with what is under it
        { "id": "tint", "type": "ellipse", "size": { "width": 200, "height": 200 }, "blend_mode": "screen",
          "fill": { "type": "radial", "stops": [{ "offset": 0, "color": "#7b2ff7" }, { "offset": 1, "color": "rgba(123,47,247,0)" }] },
          "transform": { "position": { "x": 180, "y": 150 } } },
        // static text partly off-screen with a shadow -> cached, clipped to the screen
        { "id": "edge", "type": "text", "transform": { "position": { "x": 520, "y": 320 } },
          "shadow": { "y": 3, "blur": 8, "color": "#000000" },
          "text": { "value": "Tepi skrin", "font_family": "Noto Sans", "font_size": 40, "font_weight": 700 } }
    ]);
    compile_at(&shared_assets(), &template(fonts(), layers))
}

#[test]
fn caching_does_not_change_the_picture() {
    let scene = scene();
    let frames = [0, 7, 20, 21, 60, 119];
    let (cached, _, planned) = render_with(&scene, &frames, true);
    let (direct, _, none) = render_with(&scene, &frames, false);
    assert_eq!(none, 0);
    assert!(
        planned >= 4,
        "the policy should isolate several nodes, got {planned}"
    );
    for (i, f) in frames.iter().enumerate() {
        let (worst, mean_abs, mean_signed) = difference(&cached[i], &direct[i]);
        // Two things legitimately differ by a step of 1/255: the gradient
        // dither pattern (it follows the layer's origin) and the rounding of
        // antialiased edges composited through a layer. Neither may shift
        // colours, and nothing may differ by more than that.
        assert!(worst <= 2, "frame {f}: a channel differs by {worst}");
        assert!(mean_abs < 0.08, "frame {f}: mean difference {mean_abs:.4}");
        assert!(
            mean_signed.abs() < 0.02,
            "frame {f}: colours shifted by {mean_signed:.4}"
        );
    }
}

#[test]
fn layers_are_reused_while_they_hold_and_rerendered_while_they_move() {
    let scene = scene();
    // Frames 30..40: everything cacheable is holding.
    let hold: Vec<u32> = (30..40).collect();
    let (_, stats, _) = render_with(&scene, &hold, true);
    // backdrop, card, mover, tint, edge = 5 layers, each rendered once then reused 9 times.
    assert_eq!(stats.layer_cache_misses, 5, "{stats:?}");
    assert_eq!(stats.layer_cache_hits, 45, "{stats:?}");

    // Frames 2..8: `mover` changes every frame, the other four still hold.
    let moving: Vec<u32> = (2..8).collect();
    let (_, stats, _) = render_with(&scene, &moving, true);
    assert_eq!(stats.layer_cache_misses, 4 + 6, "{stats:?}");
    assert_eq!(stats.layer_cache_hits, 4 * 5, "{stats:?}");
}

#[test]
fn a_render_from_a_cached_layer_is_bit_identical_to_a_fresh_one() {
    // Determinism across threads relies on this: hit or miss, same bytes.
    let scene = scene();
    let (twice, stats, _) = render_with(&scene, &[50, 50], true);
    assert!(stats.layer_cache_hits >= 5);
    assert_eq!(twice[0], twice[1]);
    let (fresh, _, _) = render_with(&scene, &[50], true);
    assert_eq!(fresh[0], twice[1]);
}

#[test]
fn a_tiny_budget_only_costs_speed() {
    let scene = scene();
    let opts = PrepareOptions {
        system_fonts: false,
        layer_cache_bytes: 1024,
        ..PrepareOptions::default()
    };
    let (backend, _) =
        SkiaBackend::prepare(Arc::clone(&scene), &opts).unwrap_or_else(|e| panic!("{e:?}"));
    let mut renderer = backend.new_renderer().unwrap();
    let mut starved = vec![0u8; (W * H * 4) as usize];
    renderer.render(&scene.evaluate(50), &mut starved).unwrap();
    renderer.render(&scene.evaluate(51), &mut starved).unwrap();
    assert_eq!(
        renderer.stats().layer_cache_hits,
        0,
        "nothing fits in 1 KiB"
    );
    assert!(backend.layer_cache_bytes() <= 1024);
    let (normal, _, _) = render_with(&scene, &[51], true);
    assert_eq!(starved, normal[0]);
}

#[test]
fn layers_of_finished_nodes_are_released() {
    let layers = json!([
        { "id": "early", "type": "rect", "size": { "width": 640, "height": 360 }, "end_frame": 30,
          "fill": { "type": "linear", "stops": [{ "offset": 0, "color": "#000" }, { "offset": 1, "color": "#fff" }] } },
        { "id": "late", "type": "rect", "size": { "width": 320, "height": 180 }, "start_frame": 30,
          "fill": { "type": "linear", "stops": [{ "offset": 0, "color": "#f00" }, { "offset": 1, "color": "#00f" }] } }
    ]);
    let scene = compile_at(&shared_assets(), &template(json!([]), layers));
    let backend = backend(&scene);
    let mut renderer = backend.new_renderer().unwrap();
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    renderer.render(&scene.evaluate(10), &mut rgba).unwrap();
    assert_eq!(backend.layer_cache_bytes(), 640 * 360 * 4);
    renderer.render(&scene.evaluate(40), &mut rgba).unwrap();
    // a layer keeps a 2 px safety margin where the screen allows it
    let late = 322 * 182 * 4;
    assert_eq!(backend.layer_cache_bytes(), 640 * 360 * 4 + late);
    backend.release_frames_before(30);
    assert_eq!(
        backend.layer_cache_bytes(),
        late,
        "`early` ended at frame 30"
    );
}

/// A shadow drawn as a blurred outline must look like the same shadow made
/// by filtering the layer. `opacity: 0.999` forces the filter method.
#[test]
fn outline_shadows_match_filtered_shadows() {
    let build = |opacity: f64| {
        let layers = json!([
            { "id": "bg", "type": "solid", "color": "#dfe6f0" },
            { "id": "card", "type": "rect", "size": { "width": 220, "height": 130 }, "corner_radius": 24, "fill": "#f9c74f",
              "opacity": opacity, "shadow": { "x": 8, "y": 14, "blur": 36, "color": "rgba(0,0,0,0.6)" },
              "transform": { "position": { "x": 90, "y": 70 } } },
            { "id": "dot", "type": "ellipse", "size": { "width": 120, "height": 120 }, "fill": "#e94560",
              "opacity": opacity, "shadow": { "y": 10, "blur": 20, "color": "#000000" },
              "transform": { "position": { "x": 420, "y": 150 } } }
        ]);
        compile_at(&shared_assets(), &template(json!([]), layers))
    };
    let (outline, _, _) = render_with(&build(1.0), &[0], false);
    let (filtered, _, _) = render_with(&build(0.999), &[0], false);
    let (worst, mean_abs, mean_signed) = difference(&outline[0], &filtered[0]);
    // Two Gaussian implementations: allow a few levels in the soft falloff,
    // but no visible change in size, position or darkness. (Unrotated on
    // purpose: under rotation the filter method resamples the whole layer
    // and softens the shape's own edge, which the outline method does not.)
    eprintln!(
        "outline vs filtered shadow: worst {worst}, mean {mean_abs:.4}, signed {mean_signed:.4}"
    );
    assert!(worst <= 10, "worst difference {worst}");
    assert!(mean_abs < 0.6, "mean difference {mean_abs:.3}");
    assert!(
        mean_signed.abs() < 0.3,
        "shadow darkness shifted by {mean_signed:.3}"
    );
}
