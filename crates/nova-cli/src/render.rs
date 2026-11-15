//! `render`, `preview` and `sheet`.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use clap::Args;
use nova_core::{Error, ErrorCode, Scene};
use nova_encoder::ffmpeg::tool_path;
use nova_encoder::{EncodeProfile, EncoderConfig, FfmpegSink};
use nova_raster::RasterBackend;
use nova_raster::sheet::{SheetCell, contact_sheet};
use nova_scheduler::{Progress, RenderOptions, RenderStats, render, render_frame_rgba};
use serde_json::{Value, json};

use crate::InputArgs;
use crate::load::{Loaded, load, print_warnings};
use crate::report::{Failure, human_bytes, human_duration, peak_rss_bytes, r3};

/// Scheduler tuning shared by `render` and `benchmark`.
#[derive(Args, Clone)]
pub struct PipelineArgs {
    /// Most frames started but not yet written (bounds memory) [default: 3 x threads]
    #[arg(long, value_name = "FRAMES")]
    pub window: Option<usize>,
    /// Consecutive frames a thread takes at a time [default: 4]
    #[arg(long, value_name = "FRAMES")]
    pub chunk: Option<usize>,
    /// Rasterise every frame, even ones identical to the previous frame
    #[arg(long)]
    pub no_reuse: bool,
}

/// Encoder settings shared by `render` and `benchmark`.
#[derive(Args, Clone)]
pub struct EncodeArgs {
    /// Encoding profile: preview, balanced or high_quality
    #[arg(long, default_value = "balanced")]
    pub profile: String,
    /// Override the profile's x264 preset (ultrafast … veryslow)
    #[arg(long)]
    pub preset: Option<String>,
    /// Override the profile's CRF (lower = better quality, bigger file)
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=51))]
    pub crf: Option<u8>,
    /// x264 threads [default: chosen by x264]
    #[arg(long, value_name = "N")]
    pub encoder_threads: Option<u32>,
}

impl EncodeArgs {
    pub fn profile(&self) -> Result<EncodeProfile, Failure> {
        let mut profile = EncodeProfile::named(&self.profile).ok_or_else(|| {
            Failure::one(Error::new(
                ErrorCode::Validation,
                format!(
                    "unknown profile `{}`; choose one of: {}",
                    self.profile,
                    EncodeProfile::NAMES.join(", ")
                ),
            ))
        })?;
        if let Some(p) = &self.preset {
            const PRESETS: [&str; 9] = [
                "ultrafast",
                "superfast",
                "veryfast",
                "faster",
                "fast",
                "medium",
                "slow",
                "slower",
                "veryslow",
            ];
            if !PRESETS.contains(&p.as_str()) {
                return Err(Failure::one(Error::new(
                    ErrorCode::Validation,
                    format!(
                        "unknown x264 preset `{p}`; choose one of: {}",
                        PRESETS.join(", ")
                    ),
                )));
            }
            profile.preset = p.clone();
        }
        if let Some(crf) = self.crf {
            profile.crf = crf;
        }
        Ok(profile)
    }
}

#[derive(Args)]
pub struct RenderArgs {
    #[command(flatten)]
    pub input: InputArgs,
    /// MP4 file to write
    #[arg(long, short, value_name = "FILE")]
    pub output: PathBuf,
    /// Render threads [default: all CPUs]
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
    #[command(flatten)]
    pub pipeline: PipelineArgs,
    #[command(flatten)]
    pub encode: EncodeArgs,
    /// First frame to render
    #[arg(long, default_value_t = 0)]
    pub start: u32,
    /// Stop before this frame [default: end of the composition]
    #[arg(long)]
    pub end: Option<u32>,
    /// Leave the template's audio tracks out of the file
    #[arg(long)]
    pub no_audio: bool,
    /// Print the job report as JSON on stdout
    #[arg(long)]
    pub json: bool,
    /// No progress line
    #[arg(long, short)]
    pub quiet: bool,
}

/// One-line progress on a terminal; an occasional plain line when piped to a log.
pub struct ProgressPrinter {
    tty: bool,
    quiet: bool,
    last: Instant,
    fps: f64,
}

impl ProgressPrinter {
    pub fn new(quiet: bool, composition_fps: f64) -> ProgressPrinter {
        ProgressPrinter {
            tty: std::io::stderr().is_terminal(),
            quiet,
            last: Instant::now(),
            fps: composition_fps,
        }
    }

    pub fn update(&mut self, p: Progress) {
        if self.quiet {
            return;
        }
        let done = p.frames_done == p.frames_total;
        let interval = if self.tty { 0.2 } else { 5.0 };
        if !done && self.last.elapsed().as_secs_f64() < interval {
            return;
        }
        self.last = Instant::now();
        let secs = p.elapsed.as_secs_f64().max(1e-6);
        let rate = p.frames_done as f64 / secs;
        let eta = (p.frames_total - p.frames_done) as f64 / rate.max(1e-6);
        let line = format!(
            "frame {}/{} ({:.0}%)  {:.1} fps  {:.2}x realtime  eta {}",
            p.frames_done,
            p.frames_total,
            p.frames_done as f64 * 100.0 / p.frames_total as f64,
            rate,
            rate / self.fps,
            human_duration(eta)
        );
        let mut err = std::io::stderr().lock();
        if self.tty {
            let _ = write!(err, "\r\x1b[2K{line}");
            if done {
                let _ = writeln!(err);
            }
        } else {
            let _ = writeln!(err, "{line}");
        }
        let _ = err.flush();
    }
}

/// Ctrl-C sets a flag; the pipeline stops cleanly, FFmpeg is terminated and
/// the partial file removed. A second Ctrl-C exits immediately.
pub fn install_cancel_handler() -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let _ = ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        eprintln!("\ncancelling… (press Ctrl-C again to quit immediately)");
    });
    cancel
}

pub fn render_options(
    threads: Option<usize>,
    pipeline: &PipelineArgs,
    start: u32,
    end: Option<u32>,
) -> RenderOptions {
    RenderOptions {
        threads: threads.unwrap_or(0),
        window: pipeline.window.unwrap_or(0),
        chunk: pipeline.chunk.unwrap_or(0),
        start,
        end,
        reuse_identical_frames: !pipeline.no_reuse,
    }
}

pub fn stats_json(s: &RenderStats) -> Value {
    json!({
        "threads": s.threads,
        "window": s.window,
        "chunk": s.chunk,
        "frames": s.frames,
        "frames_rendered": s.frames_rendered,
        "frames_reused": s.frames_reused,
        "wall_ms": r3(s.wall_ms),
        "fps": r3(s.fps()),
        "worker_busy_ms": r3(s.worker_busy_ms),
        "frame_ms": { "p50": r3(s.frame_ms_p50), "p95": r3(s.frame_ms_p95), "p99": r3(s.frame_ms_p99), "max": r3(s.frame_ms_max) },
        "sink_ms": r3(s.sink_ms),
        "writer_wait_ms": r3(s.writer_wait_ms),
        "cache": {
            "text_layouts": s.renderer.text_layouts,
            "text_cache_hits": s.renderer.text_cache_hits,
            "offscreen_layers": s.renderer.offscreen_layers,
            "layer_cache_hits": s.renderer.layer_cache_hits,
            "layer_cache_misses": s.renderer.layer_cache_misses,
        },
    })
}

pub fn encoder_config(
    scene: &Scene,
    output: &Path,
    frames: u32,
    profile: EncodeProfile,
    threads: Option<u32>,
    audio: bool,
) -> EncoderConfig {
    EncoderConfig {
        ffmpeg: tool_path("ffmpeg"),
        output: output.to_owned(),
        width: scene.width,
        height: scene.height,
        fps: scene.fps,
        frame_count: frames,
        profile,
        threads,
        audio: if audio {
            scene.audio.clone()
        } else {
            Vec::new()
        },
    }
}

pub fn cmd_render(args: RenderArgs) -> Result<(), Failure> {
    let json = args.json;
    run_render(args).map_err(|f| f.with_json(json))
}

fn run_render(args: RenderArgs) -> Result<(), Failure> {
    let job_started = Instant::now();
    let profile = args.encode.profile()?;
    let Loaded {
        scene,
        backend,
        warnings,
        compile_ms,
        prepare,
    } = load(&args.input)?;
    if !args.json {
        print_warnings(&warnings);
    }

    let end = args
        .end
        .unwrap_or(scene.duration_frames)
        .min(scene.duration_frames);
    if args.start >= end {
        return Err(Failure::one(Error::new(
            ErrorCode::Validation,
            format!(
                "frame range {}..{end} is empty (the composition has {} frames)",
                args.start, scene.duration_frames
            ),
        )));
    }
    let frames = end - args.start;
    // A partial render has no meaningful audio alignment; keep it video-only.
    let full = args.start == 0 && end == scene.duration_frames;
    let with_audio = !args.no_audio && full && !scene.audio.is_empty();
    if !args.no_audio && !full && !scene.audio.is_empty() && !args.json {
        eprintln!("note: audio is left out when rendering a partial frame range");
    }

    let opts = render_options(args.threads, &args.pipeline, args.start, Some(end));
    let config = encoder_config(
        &scene,
        &args.output,
        frames,
        profile.clone(),
        args.encode.encoder_threads,
        with_audio,
    );
    let cancel = install_cancel_handler();

    let mut sink = FfmpegSink::start(config)?;
    let mut printer = ProgressPrinter::new(args.quiet || args.json, scene.fps.as_f64());
    let mut on_progress = |p: Progress| printer.update(p);
    let stats = render(
        &scene,
        &backend,
        &mut sink,
        &opts,
        &cancel,
        Some(&mut on_progress),
    )?;
    let encode = sink.finish()?;

    // Trust, but verify: the file must really contain every frame.
    let probe = nova_encoder::probe(&tool_path("ffprobe"), &args.output).ok();
    if let Some(info) = &probe
        && info.frames != frames as u64
    {
        return Err(Failure::one(Error::new(
            ErrorCode::Encode,
            format!(
                "the output has {} frames but {frames} were rendered",
                info.frames
            ),
        )));
    }
    let total_ms = job_started.elapsed().as_secs_f64() * 1000.0;
    let video_seconds = scene.fps.frame_to_seconds(frames as i64);

    if args.json {
        let report = json!({
            "ok": true,
            "output": args.output,
            "backend": backend.name(),
            "composition": { "id": scene.id, "width": scene.width, "height": scene.height, "fps": scene.fps.as_f64() },
            "frames": frames,
            "video_seconds": r3(video_seconds),
            "total_ms": r3(total_ms),
            "speed_x_realtime": r3(video_seconds * 1000.0 / total_ms),
            "compile_ms": r3(compile_ms),
            "prepare_ms": r3(prepare.prepare_ms),
            "render": stats_json(&stats),
            "encode": {
                "profile": profile.name, "preset": profile.preset, "crf": profile.crf,
                "pipe_write_ms": r3(encode.pipe_write_ms), "total_ms": r3(encode.total_ms),
                "output_bytes": encode.output_bytes, "command": encode.command,
            },
            "audio_tracks": if with_audio { scene.audio.len() } else { 0 },
            "decoded_asset_bytes": prepare.decoded_bytes,
            "prepared_images": prepare.image_variants,
            "cached_layer_nodes": prepare.cached_layers,
            "peak_rss_bytes": peak_rss_bytes(),
            "verified_frames": probe.as_ref().map(|p| p.frames),
            "warnings": warnings.iter().map(|w| json!({ "path": w.path, "message": w.message })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }

    // The writer is either waiting for frames (rendering is the slow side)
    // or blocked in the pipe (encoding is the slow side).
    let bottleneck = if stats.sink_ms > stats.writer_wait_ms {
        "encoder"
    } else {
        "renderer"
    };
    println!(
        "wrote {} ({})",
        args.output.display(),
        human_bytes(encode.output_bytes)
    );
    println!(
        "  {} frames ({:.2}s of video) in {} — {:.1} fps, {:.2}x realtime",
        frames,
        video_seconds,
        human_duration(total_ms / 1000.0),
        frames as f64 * 1000.0 / total_ms,
        video_seconds * 1000.0 / total_ms
    );
    println!(
        "  rendered {} frames, reused {} identical; {} threads, frame time p50 {:.1} ms / p95 {:.1} ms",
        stats.frames_rendered,
        stats.frames_reused,
        stats.threads,
        stats.frame_ms_p50,
        stats.frame_ms_p95
    );
    println!(
        "  load {:.0} ms, assets {:.0} ms, pipeline {}, limited by the {bottleneck} (writer waited {:.1}s for frames, {:.1}s on the encoder)",
        compile_ms,
        prepare.prepare_ms,
        human_duration(stats.wall_ms / 1000.0),
        stats.writer_wait_ms / 1000.0,
        stats.sink_ms / 1000.0
    );
    println!(
        "  x264 {} crf {}{}; peak memory {}",
        profile.preset,
        profile.crf,
        if with_audio {
            format!(", {} audio track(s)", scene.audio.len())
        } else {
            String::new()
        },
        peak_rss_bytes()
            .map(human_bytes)
            .unwrap_or_else(|| "unknown".into())
    );
    Ok(())
}

pub fn cmd_preview(input: &InputArgs, frame: u32, output: &Path) -> Result<(), Failure> {
    let loaded = load(input)?;
    print_warnings(&loaded.warnings);
    let started = Instant::now();
    let rgba = render_frame_rgba(&loaded.scene, &loaded.backend, frame)?;
    let render_ms = started.elapsed().as_secs_f64() * 1000.0;
    nova_raster::png::write_png(output, &rgba, loaded.scene.width, loaded.scene.height)?;
    println!(
        "wrote {} (frame {frame}, {:.2}s, {}x{}, rendered in {:.1} ms)",
        output.display(),
        loaded.scene.fps.frame_to_seconds(frame as i64),
        loaded.scene.width,
        loaded.scene.height,
        render_ms
    );
    Ok(())
}

pub fn cmd_sheet(
    input: &InputArgs,
    count: u32,
    frames: &[u32],
    columns: u32,
    cell_width: u32,
    output: &Path,
) -> Result<(), Failure> {
    let loaded = load(input)?;
    print_warnings(&loaded.warnings);
    let scene = &loaded.scene;
    let total = scene.duration_frames;
    let picks: Vec<u32> = if !frames.is_empty() {
        frames.to_vec()
    } else {
        let n = count.clamp(1, total);
        // evenly spaced, always including the first and last frame
        (0..n)
            .map(|i| {
                if n == 1 {
                    0
                } else {
                    ((total - 1) as u64 * i as u64 / (n - 1) as u64) as u32
                }
            })
            .collect()
    };
    let mut renderer = loaded.backend.new_renderer()?;
    let mut cells = Vec::with_capacity(picks.len());
    for &f in &picks {
        if f >= total {
            return Err(Failure::one(Error::new(
                ErrorCode::Validation,
                format!("frame {f} is out of range; the composition has frames 0..{total}"),
            )));
        }
        let mut rgba = vec![0u8; scene.width as usize * scene.height as usize * 4];
        renderer.render(&scene.evaluate(f), &mut rgba)?;
        cells.push(SheetCell {
            label: format!("f {f} · {:.2}s", scene.fps.frame_to_seconds(f as i64)),
            rgba,
        });
    }
    let (pixels, w, h) = contact_sheet(&cells, scene.width, scene.height, columns, cell_width)?;
    nova_raster::png::write_png(output, &pixels, w, h)?;
    println!(
        "wrote {} ({} frames, {w}x{h})",
        output.display(),
        cells.len()
    );
    Ok(())
}
