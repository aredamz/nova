//! Shared helpers for raster tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nova_core::{Limits, Scene};
use nova_raster::{PrepareOptions, RasterBackend, SkiaBackend};
use serde_json::{Value, json};

pub const W: u32 = 640;
pub const H: u32 = 360;

/// Folder with the bundled fonts; used as asset root so text is reproducible.
pub fn shared_assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shared")
}

pub fn fonts() -> Value {
    json!([
        { "id": "noto", "type": "font", "src": "fonts/NotoSans-Regular.ttf", "family": "Noto Sans" },
        { "id": "noto-bold", "type": "font", "src": "fonts/NotoSans-Bold.ttf", "family": "Noto Sans" }
    ])
}

pub fn template(assets: Value, layers: Value) -> Value {
    json!({
        "schema_version": "1.0",
        "composition": { "id": "test", "width": W, "height": H, "fps": 30, "duration_frames": 120, "background": "#14182b" },
        "assets": assets,
        "layers": layers
    })
}

pub fn compile_at(root: &Path, template: &Value) -> Arc<Scene> {
    let compiled = nova_template::load_value(template, None, root, &Limits::default())
        .unwrap_or_else(|errs| {
            panic!(
                "template rejected:\n{}",
                errs.iter()
                    .map(|e| format!("  {e}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
    Arc::new(compiled.scene)
}

pub fn backend(scene: &Arc<Scene>) -> SkiaBackend {
    // No system fonts: tests must not depend on what the host has installed.
    let opts = PrepareOptions {
        system_fonts: false,
        ..PrepareOptions::default()
    };
    SkiaBackend::prepare(Arc::clone(scene), &opts)
        .unwrap_or_else(|errs| {
            panic!(
                "prepare failed:\n{}",
                errs.iter()
                    .map(|e| format!("  {e}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
        .0
}

pub fn render(scene: &Arc<Scene>, frame: u32) -> Vec<u8> {
    let backend = backend(scene);
    let mut renderer = backend.new_renderer().unwrap();
    let mut rgba = vec![0u8; scene.width as usize * scene.height as usize * 4];
    renderer.render(&scene.evaluate(frame), &mut rgba).unwrap();
    rgba
}

pub fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = (y as usize * width as usize + x as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

pub fn assert_near(actual: [u8; 4], expected: [u8; 4], tolerance: u8, what: &str) {
    let ok = actual
        .iter()
        .zip(expected)
        .all(|(a, e)| a.abs_diff(e) <= tolerance);
    assert!(
        ok,
        "{what}: got {actual:?}, expected {expected:?} (±{tolerance})"
    );
}
