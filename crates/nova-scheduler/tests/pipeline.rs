//! Pipeline behaviour: ordering, determinism, frame reuse, failure handling,
//! and a real encode through FFmpeg.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nova_core::{Error, ErrorCode, Limits, Result, Scene};
use nova_encoder::{
    EncodeProfile, EncoderConfig, FfmpegSink, FrameSink, HashSink, NullSink, PixelFormat,
};
use nova_raster::{PrepareOptions, SkiaBackend};
use nova_scheduler::{RenderOptions, render};
use serde_json::{Value, json};

fn shared_assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shared")
}

/// 90 frames, 320x180: animated for the first 30 frames, then completely still.
fn scene() -> Arc<Scene> {
    let template = json!({
        "schema_version": "1.0",
        "composition": { "id": "pipe", "width": 320, "height": 180, "fps": 30, "duration_frames": 90, "background": "#101522" },
        "assets": [{ "id": "f", "type": "font", "src": "fonts/NotoSans-Bold.ttf", "family": "Noto Sans" }],
        "layers": [
            { "id": "box", "type": "rect", "size": { "width": 60, "height": 60 }, "corner_radius": 10,
              "fill": { "type": "linear", "stops": [{ "offset": 0, "color": "#ff6b6b" }, { "offset": 1, "color": "#feca57" }] },
              "transform": { "position": { "x": 40, "y": 90 }, "anchor": { "x": 0.5, "y": 0.5 } },
              "animations": [
                { "property": "transform.position.x", "keyframes": [{ "frame": 0, "value": 40, "easing": "ease_in_out_cubic" }, { "frame": 30, "value": 260 }] },
                { "property": "transform.rotation", "keyframes": [{ "frame": 0, "value": 0, "easing": "spring" }, { "frame": 30, "value": 180 }] } ] },
            { "id": "title", "type": "text", "transform": { "position": { "x": 160, "y": 30 }, "anchor": { "x": 0.5, "y": 0.5 } },
              "shadow": { "y": 2, "blur": 6, "color": "#000000" },
              "text": { "value": "Pipeline", "font_family": "Noto Sans", "font_size": 28, "font_weight": 700 },
              "animations": [{ "property": "opacity", "keyframes": [{ "frame": 0, "value": 0 }, { "frame": 20, "value": 1 }] }] }
        ]
    });
    compile(&template)
}

fn compile(template: &Value) -> Arc<Scene> {
    let compiled = nova_template::load_value(template, None, &shared_assets(), &Limits::default())
        .unwrap_or_else(|e| panic!("{e:?}"));
    Arc::new(compiled.scene)
}

fn backend(scene: &Arc<Scene>) -> SkiaBackend {
    let opts = PrepareOptions {
        system_fonts: false,
        ..PrepareOptions::default()
    };
    SkiaBackend::prepare(Arc::clone(scene), &opts)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .0
}

fn hashes(
    scene: &Arc<Scene>,
    opts: &RenderOptions,
    format: PixelFormat,
) -> (Vec<(u32, u64)>, nova_scheduler::RenderStats) {
    let backend = backend(scene);
    let mut sink = HashSink::new(format);
    let stats = render(
        scene,
        &backend,
        &mut sink,
        opts,
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    (sink.hashes, stats)
}

fn opts(threads: usize) -> RenderOptions {
    RenderOptions {
        threads,
        ..RenderOptions::default()
    }
}

#[test]
fn frames_arrive_in_order_exactly_once() {
    let scene = scene();
    let (h, stats) = hashes(
        &scene,
        &RenderOptions {
            threads: 8,
            chunk: 1,
            window: 8,
            ..Default::default()
        },
        PixelFormat::Rgba,
    );
    let indices: Vec<u32> = h.iter().map(|(i, _)| *i).collect();
    assert_eq!(indices, (0..90).collect::<Vec<_>>());
    assert_eq!(stats.frames, 90);
    assert_eq!(stats.frames_rendered + stats.frames_reused, 90);
}

#[test]
fn output_is_identical_for_any_thread_count_chunk_and_window() {
    let scene = scene();
    let (reference, _) = hashes(
        &scene,
        &RenderOptions {
            threads: 1,
            chunk: 1,
            ..Default::default()
        },
        PixelFormat::Rgba,
    );
    for (threads, chunk, window) in [(2, 4, 0), (3, 7, 3), (8, 1, 8), (8, 16, 64), (5, 2, 5)] {
        let (h, _) = hashes(
            &scene,
            &RenderOptions {
                threads,
                chunk,
                window,
                ..Default::default()
            },
            PixelFormat::Rgba,
        );
        assert_eq!(
            h, reference,
            "threads={threads} chunk={chunk} window={window}"
        );
    }
}

#[test]
fn reusing_identical_frames_does_not_change_a_single_pixel() {
    let scene = scene();
    for format in [PixelFormat::Rgba, PixelFormat::Yuv420p] {
        let (with, s_with) = hashes(&scene, &opts(4), format);
        let (without, s_without) = hashes(
            &scene,
            &RenderOptions {
                threads: 4,
                reuse_identical_frames: false,
                ..Default::default()
            },
            format,
        );
        assert_eq!(with, without, "{format:?}");
        assert_eq!(s_without.frames_reused, 0);
        // Frames 30..90 are still: 59 of them are repeats of frame 30.
        assert_eq!(s_with.frames_reused, 59, "{format:?}");
        assert_eq!(s_with.frames_rendered, 31);
    }
}

#[test]
fn still_frames_hash_equal_and_moving_frames_do_not() {
    let scene = scene();
    let (h, _) = hashes(&scene, &opts(4), PixelFormat::Rgba);
    assert_ne!(h[5].1, h[6].1);
    assert!(h[30..].iter().all(|(_, x)| *x == h[30].1));
}

#[test]
fn a_frame_range_matches_the_same_frames_of_a_full_render() {
    let scene = scene();
    let (full, _) = hashes(&scene, &opts(4), PixelFormat::Rgba);
    let (part, stats) = hashes(
        &scene,
        &RenderOptions {
            threads: 3,
            start: 20,
            end: Some(50),
            ..Default::default()
        },
        PixelFormat::Rgba,
    );
    assert_eq!(part, full[20..50].to_vec());
    assert_eq!(stats.frames, 30);

    let backend = backend(&scene);
    let mut sink = NullSink::new(PixelFormat::Rgba);
    let empty = RenderOptions {
        start: 50,
        end: Some(50),
        ..Default::default()
    };
    let err = render(
        &scene,
        &backend,
        &mut sink,
        &empty,
        &AtomicBool::new(false),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
}

#[test]
fn single_frame_helper_matches_the_pipeline() {
    let scene = scene();
    let backend = backend(&scene);
    let (h, _) = hashes(&scene, &opts(2), PixelFormat::Rgba);
    for f in [0u32, 13, 45] {
        let rgba = nova_scheduler::render_frame_rgba(&scene, &backend, f).unwrap();
        assert_eq!(
            nova_encoder::hash_frame(&rgba),
            h[f as usize].1,
            "frame {f}"
        );
    }
    assert!(nova_scheduler::render_frame_rgba(&scene, &backend, 90).is_err());
}

/// Fails on a chosen frame, like an encoder that died.
struct FailingSink {
    fail_at: u32,
    seen: Vec<u32>,
}

impl FrameSink for FailingSink {
    fn format(&self) -> PixelFormat {
        PixelFormat::Yuv420p
    }
    fn write(&mut self, index: u32, _frame: &[u8]) -> Result<()> {
        if index == self.fail_at {
            return Err(Error::new(ErrorCode::Encode, "encoder went away"));
        }
        self.seen.push(index);
        Ok(())
    }
}

#[test]
fn a_sink_failure_stops_the_job_without_hanging() {
    let scene = scene();
    let backend = backend(&scene);
    let mut sink = FailingSink {
        fail_at: 17,
        seen: vec![],
    };
    let err = render(
        &scene,
        &backend,
        &mut sink,
        &opts(8),
        &AtomicBool::new(false),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Encode);
    assert_eq!(
        sink.seen,
        (0..17).collect::<Vec<_>>(),
        "nothing is written after the failure"
    );
}

#[test]
fn cancellation_is_honoured() {
    let scene = scene();
    let backend = backend(&scene);
    let cancel = AtomicBool::new(false);
    let mut sink = NullSink::new(PixelFormat::Rgba);
    let mut on_progress = |p: nova_scheduler::Progress| {
        if p.frames_done == 10 {
            cancel.store(true, Ordering::SeqCst);
        }
    };
    let err = render(
        &scene,
        &backend,
        &mut sink,
        &opts(4),
        &cancel,
        Some(&mut on_progress),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Cancelled);
    assert!(
        sink.frames >= 10 && sink.frames < 90,
        "stopped early: {} frames",
        sink.frames
    );
}

#[test]
fn progress_reports_every_frame() {
    let scene = scene();
    let backend = backend(&scene);
    let mut sink = NullSink::new(PixelFormat::Rgba);
    let mut seen = Vec::new();
    let mut on_progress = |p: nova_scheduler::Progress| seen.push((p.frames_done, p.frames_total));
    render(
        &scene,
        &backend,
        &mut sink,
        &opts(4),
        &AtomicBool::new(false),
        Some(&mut on_progress),
    )
    .unwrap();
    assert_eq!(seen, (1..=90).map(|d| (d, 90)).collect::<Vec<_>>());
}

// ---- FFmpeg ----------------------------------------------------------------------

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn encoder_config(scene: &Scene, output: &Path, ffmpeg: &str) -> EncoderConfig {
    EncoderConfig {
        ffmpeg: ffmpeg.into(),
        output: output.to_owned(),
        width: scene.width,
        height: scene.height,
        fps: scene.fps,
        frame_count: scene.duration_frames,
        profile: EncodeProfile::named("preview").unwrap(),
        threads: Some(2),
        audio: vec![],
        extra: vec![],
        resize: None,
    }
}

#[test]
fn renders_an_mp4_that_ffprobe_accepts() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg not installed");
        return;
    }
    let scene = scene();
    let backend = backend(&scene);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("video.mp4");
    let mut sink = FfmpegSink::start(encoder_config(&scene, &out, "ffmpeg")).unwrap();
    render(
        &scene,
        &backend,
        &mut sink,
        &opts(4),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    let report = sink.finish().unwrap();
    assert_eq!(report.frames_written, 90);
    assert!(report.output_bytes > 1000);

    let info = nova_encoder::probe(Path::new("ffprobe"), &out).unwrap();
    assert_eq!((info.width, info.height), (320, 180));
    assert_eq!(info.codec, "h264");
    assert_eq!(info.pix_fmt, "yuv420p");
    assert_eq!(info.frames, 90);
    assert_eq!(info.frame_rate, "30/1");
    assert_eq!(info.color_space.as_deref(), Some("bt709"));
    assert_eq!(info.color_range.as_deref(), Some("tv"));
    assert!((info.duration_seconds - 3.0).abs() < 0.05);
    // No temporary file is left next to the output.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers.len(), 1, "{leftovers:?}");
}

#[test]
fn colours_survive_the_encode() {
    // Flat colour patches through x264 and back to RGB: if the matrix, range
    // or tags were inconsistent anywhere, the decoded colours would be off.
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg not installed");
        return;
    }
    let colours = [
        "#e63946", "#2a9d8f", "#e9c46a", "#264653", "#ffffff", "#101522", "#7b2ff7", "#808080",
    ];
    let layers: Vec<Value> = colours
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({ "id": format!("c{i}"), "type": "rect", "size": { "width": 80, "height": 90 }, "fill": c,
                    "transform": { "position": { "x": (i % 4) * 80, "y": (i / 4) * 90 } } })
        })
        .collect();
    let scene = compile(&json!({
        "schema_version": "1.0",
        "composition": { "id": "colours", "width": 320, "height": 180, "fps": 30, "duration_frames": 5 },
        "layers": layers
    }));
    let backend = backend(&scene);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("colours.mp4");
    let mut cfg = encoder_config(&scene, &out, "ffmpeg");
    cfg.profile = EncodeProfile {
        name: "test".into(),
        preset: "medium".into(),
        crf: 4,
        tune: None,
    };
    let mut sink = FfmpegSink::start(cfg).unwrap();
    render(
        &scene,
        &backend,
        &mut sink,
        &opts(2),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    sink.finish().unwrap();

    // Decode frame 2 the way a player would (honouring the file's colour tags).
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&out)
        .args([
            "-vf",
            "select=eq(n\\,2),scale=in_color_matrix=bt709:in_range=tv:out_range=pc",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert_eq!(decoded.stdout.len(), 320 * 180 * 3);
    for (i, c) in colours.iter().enumerate() {
        let want = nova_core::Color::parse(c).unwrap().to_rgba8();
        let (cx, cy) = ((i % 4) * 80 + 40, (i / 4) * 90 + 45);
        let o = (cy * 320 + cx) * 3;
        let got = &decoded.stdout[o..o + 3];
        for ch in 0..3 {
            assert!(
                got[ch].abs_diff(want[ch]) <= 4,
                "{c}: decoded {got:?}, rendered {:?}",
                &want[..3]
            );
        }
    }
}

#[test]
fn audio_is_muxed_and_trimmed_to_the_video() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // 10 s tone; the video is 3 s, so the output must be 3 s.
    let tone = dir.path().join("tone.wav");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=10",
        ])
        .arg(&tone)
        .status()
        .unwrap();
    assert!(made.success());
    let scene = scene();
    let backend = backend(&scene);
    let out = dir.path().join("with-audio.mp4");
    let mut cfg = encoder_config(&scene, &out, "ffmpeg");
    cfg.audio = vec![
        nova_core::AudioTrack {
            id: "a".into(),
            path: tone.clone(),
            start_frame: 0,
            volume: 0.8,
        },
        nova_core::AudioTrack {
            id: "b".into(),
            path: tone,
            start_frame: 30,
            volume: 0.3,
        },
    ];
    let mut sink = FfmpegSink::start(cfg).unwrap();
    render(
        &scene,
        &backend,
        &mut sink,
        &opts(4),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    sink.finish().unwrap();
    let info = nova_encoder::probe(Path::new("ffprobe"), &out).unwrap();
    assert!(info.has_audio);
    assert_eq!(info.frames, 90);
    assert!(
        (info.duration_seconds - 3.0).abs() < 0.1,
        "duration {}",
        info.duration_seconds
    );
}

#[test]
fn a_missing_ffmpeg_is_a_clear_error() {
    let scene = scene();
    let dir = tempfile::tempdir().unwrap();
    let err = FfmpegSink::start(encoder_config(
        &scene,
        &dir.path().join("x.mp4"),
        "/nonexistent/ffmpeg",
    ))
    .err()
    .unwrap();
    assert_eq!(err.code, ErrorCode::Encode);
    assert!(err.message.contains("cannot start FFmpeg"));
}

#[cfg(unix)]
#[test]
fn an_encoder_that_dies_fails_the_job_and_leaves_no_file() {
    use std::os::unix::fs::PermissionsExt;
    let scene = scene();
    let backend = backend(&scene);
    let dir = tempfile::tempdir().unwrap();
    // Stand-in for an FFmpeg that crashes straight away.
    let fake = dir.path().join("fake-ffmpeg");
    std::fs::write(
        &fake,
        "#!/bin/sh\necho 'Unknown encoder libx264' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = dir.path().join("never.mp4");

    let mut sink = FfmpegSink::start(encoder_config(&scene, &out, fake.to_str().unwrap())).unwrap();
    let result = render(
        &scene,
        &backend,
        &mut sink,
        &opts(4),
        &AtomicBool::new(false),
        None,
    );
    let err = match result {
        Err(e) => e,
        Ok(_) => sink
            .finish()
            .expect_err("a dead encoder cannot produce a video"),
    };
    assert_eq!(err.code, ErrorCode::Encode);
    assert!(
        err.message.contains("Unknown encoder libx264"),
        "stderr is surfaced: {}",
        err.message
    );
    assert!(!out.exists());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        leftovers.len(),
        1,
        "only the fake binary remains: {leftovers:?}"
    );
}
