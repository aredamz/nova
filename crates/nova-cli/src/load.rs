//! Loading a template into a ready-to-render state, plus the two commands
//! that only look at it: `validate` and `inspect`.

use std::sync::Arc;
use std::time::Instant;

use nova_core::{AssetKind, Limits, Node, NodeId, NodeKind, Scene};
use nova_raster::{PrepareOptions, PrepareReport, SkiaBackend};
use nova_template::{LoadOptions, Warning};
use serde_json::{Value, json};

use crate::InputArgs;
use crate::report::{Failure, human_bytes, r3};

pub struct Loaded {
    pub scene: Arc<Scene>,
    /// Width and height the template asks for, before `--scale`.
    pub template_size: (u32, u32),
    pub backend: SkiaBackend,
    pub warnings: Vec<Warning>,
    /// Read + bind + validate + compile, including reading asset files.
    pub compile_ms: f64,
    pub prepare: PrepareReport,
}

/// Everything from the template file to decoded assets. Any problem with the
/// template is found here, before a single frame is drawn.
pub fn load(input: &InputArgs) -> Result<Loaded, Failure> {
    let started = Instant::now();
    let data: Option<Value> = match &input.data {
        Some(path) => Some(nova_template::read_json_file(path, "data file")?),
        None => None,
    };
    let limits = Limits::default();
    let opts = LoadOptions {
        asset_root: input.assets_root.clone(),
        limits: limits.clone(),
    };
    let compiled = nova_template::load_file(&input.template, data.as_ref(), &opts)?;
    let compile_ms = started.elapsed().as_secs_f64() * 1000.0;

    let template_size = (compiled.scene.width, compiled.scene.height);
    let scene = if input.scale == 1.0 {
        compiled.scene
    } else {
        if !(input.scale.is_finite() && (0.05..=16.0).contains(&input.scale)) {
            return Err(Failure::one(nova_core::Error::new(
                nova_core::ErrorCode::Validation,
                format!("--scale must be between 0.05 and 16 (got {})", input.scale),
            )));
        }
        let scaled = compiled.scene.scaled(input.scale);
        if scaled.width > limits.max_width || scaled.height > limits.max_height {
            return Err(Failure::one(nova_core::Error::new(
                nova_core::ErrorCode::Validation,
                format!(
                    "--scale {} gives {}x{}; the limit is {}x{}",
                    input.scale, scaled.width, scaled.height, limits.max_width, limits.max_height
                ),
            )));
        }
        scaled
    };
    let scene = Arc::new(scene);
    let prepare_opts = PrepareOptions {
        system_fonts: !input.no_system_fonts,
        limits,
        ..PrepareOptions::default()
    };
    let (backend, prepare) = SkiaBackend::prepare(Arc::clone(&scene), &prepare_opts)?;
    Ok(Loaded {
        scene,
        template_size,
        backend,
        warnings: compiled.warnings,
        compile_ms,
        prepare,
    })
}

pub fn print_warnings(warnings: &[Warning]) {
    for w in warnings {
        eprintln!("warning {}: {}", w.path, w.message);
    }
}

pub fn cmd_validate(input: &InputArgs, json: bool) -> Result<(), Failure> {
    let loaded = load(input).map_err(|f| f.with_json(json))?;
    let s = &loaded.scene;
    if json {
        let out = json!({
            "ok": true,
            "composition": { "id": s.id, "width": s.width, "height": s.height, "fps": s.fps.as_f64(),
                             "duration_frames": s.duration_frames, "duration_seconds": r3(s.duration_seconds()) },
            "layers": s.nodes.len(),
            "assets": s.assets.len(),
            "warnings": loaded.warnings.iter().map(|w| json!({ "path": w.path, "message": w.message })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        print_warnings(&loaded.warnings);
        println!(
            "ok: `{}` {}x{} @ {} fps, {} frames ({:.2}s), {} layers, {} assets{}",
            s.id,
            s.width,
            s.height,
            s.fps.to_ffmpeg(),
            s.duration_frames,
            s.duration_seconds(),
            s.nodes.len(),
            s.assets.len(),
            if loaded.warnings.is_empty() {
                String::new()
            } else {
                format!(", {} warning(s)", loaded.warnings.len())
            }
        );
    }
    Ok(())
}

/// How many frames differ from the one before them. Everything else is
/// rendered once and reused.
fn count_unique_frames(scene: &Scene) -> u32 {
    let mut unique = 0;
    let mut previous = None;
    for f in 0..scene.duration_frames {
        let plan = scene.evaluate(f);
        if previous.as_ref() != Some(&plan) {
            unique += 1;
        }
        previous = Some(plan);
    }
    unique
}

fn describe(node: &Node) -> String {
    match &node.kind {
        NodeKind::Text(t) => {
            let mut preview: String = t.value.chars().take(32).collect();
            if t.value.chars().count() > 32 {
                preview.push('…');
            }
            format!("\"{}\"", preview.replace('\n', "⏎"))
        }
        NodeKind::Path { data, .. } => format!("{} bytes of path data", data.len()),
        NodeKind::Group { children, .. } => format!("{} children", children.len()),
        _ => String::new(),
    }
}

fn print_tree(scene: &Scene, ids: &[NodeId], depth: usize) {
    for &id in ids {
        let n = scene.node(id);
        let motion = if n.is_static() { "static" } else { "animated" };
        let fx = [
            (!matches!(n.blur.as_const(), Some(b) if *b == 0.0)).then_some("blur"),
            n.shadow.is_some().then_some("shadow"),
            (n.blend != nova_core::BlendMode::Normal).then_some("blend"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("+");
        println!(
            "  {:indent$}{:<8} {:<24} frames {:>5}..{:<5} {:<8} {} {}",
            "",
            n.type_name(),
            n.id,
            n.start,
            n.end,
            motion,
            fx,
            describe(n),
            indent = depth * 2
        );
        if let NodeKind::Group { children, .. } = &n.kind {
            print_tree(scene, children, depth + 1);
        }
    }
}

pub fn cmd_inspect(input: &InputArgs, json: bool) -> Result<(), Failure> {
    // Slots are read from the raw template, so `inspect` can list what data a
    // template needs even when none was supplied.
    let raw = nova_template::read_json_file(&input.template, "template")
        .map_err(|e| Failure::one(e).with_json(json))?;
    let slots = nova_template::binding::list_slots(&raw);
    if input.data.is_none() && !slots.is_empty() {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({ "ok": true, "needs_data": true, "data_slots": slots })
                )
                .unwrap()
            );
        } else {
            println!("This template needs data. Pass --data <file.json> providing:");
            for s in &slots {
                println!("  {{{{{s}}}}}");
            }
        }
        return Ok(());
    }

    let loaded = load(input).map_err(|f| f.with_json(json))?;
    let s = &loaded.scene;
    let unique = count_unique_frames(s);
    let animated = s.nodes.iter().filter(|n| !n.is_static()).count();
    let asset_bytes: u64 = s.assets.iter().map(|a| a.bytes.len() as u64).sum();

    if json {
        let layers: Vec<Value> = s
            .nodes
            .iter()
            .map(|n| json!({ "id": n.id, "type": n.type_name(), "start": n.start, "end": n.end, "animated": !n.is_static() }))
            .collect();
        let assets: Vec<Value> = s
            .assets
            .iter()
            .map(|a| {
                let kind = match a.kind {
                    AssetKind::Image => "image",
                    AssetKind::Svg => "svg",
                    AssetKind::Font => "font",
                    AssetKind::Frames => "frames",
                };
                json!({ "id": a.id, "type": kind, "src": a.src, "bytes": a.bytes.len(), "hash": format!("{:016x}", a.content_hash) })
            })
            .collect();
        let out = json!({
            "ok": true,
            "composition": { "id": s.id, "name": s.name, "width": s.width, "height": s.height, "fps": s.fps.as_f64(),
                             "duration_frames": s.duration_frames, "duration_seconds": r3(s.duration_seconds()) },
            "frames": { "total": s.duration_frames, "unique": unique, "reused": s.duration_frames - unique },
            "layers": layers,
            "assets": assets,
            "audio_tracks": s.audio.len(),
            "data_slots": slots,
            "decoded_asset_bytes": loaded.prepare.decoded_bytes,
            "warnings": loaded.warnings.iter().map(|w| json!({ "path": w.path, "message": w.message })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    print_warnings(&loaded.warnings);
    println!(
        "Composition  {}{}",
        s.id,
        s.name
            .as_ref()
            .map(|n| format!(" — {n}"))
            .unwrap_or_default()
    );
    println!("  size       {}x{}", s.width, s.height);
    println!("  rate       {} fps", s.fps.to_ffmpeg());
    println!(
        "  length     {} frames ({:.2}s)",
        s.duration_frames,
        s.duration_seconds()
    );
    println!(
        "  frames     {unique} unique, {} reused ({:.0}% of the video is rendered once and repeated)",
        s.duration_frames - unique,
        (s.duration_frames - unique) as f64 * 100.0 / s.duration_frames as f64
    );
    println!("Layers       {} ({animated} animated)", s.nodes.len());
    print_tree(s, &s.roots, 0);
    println!(
        "Assets       {} ({} on disk, ~{} decoded)",
        s.assets.len(),
        human_bytes(asset_bytes),
        human_bytes(loaded.prepare.decoded_bytes)
    );
    for a in &s.assets {
        let kind = match a.kind {
            AssetKind::Image => "image",
            AssetKind::Svg => "svg",
            AssetKind::Font => "font",
                    AssetKind::Frames => "frames",
        };
        println!(
            "  {:<6} {:<20} {:<36} {:>10}  {:016x}",
            kind,
            a.id,
            a.src,
            human_bytes(a.bytes.len() as u64),
            a.content_hash
        );
    }
    if !s.audio.is_empty() {
        println!("Audio        {} track(s)", s.audio.len());
        for t in &s.audio {
            println!(
                "  {:<20} starts at frame {:<6} volume {:.2}  {}",
                t.id,
                t.start_frame,
                t.volume,
                t.path.display()
            );
        }
    }
    if !slots.is_empty() {
        println!("Data slots   {}", slots.join(", "));
    }
    Ok(())
}
