//! `nova benchmark`: repeatable throughput measurements.
//!
//! Two numbers matter and they are never mixed: *render-only* (frames are
//! rasterised, converted and thrown away) and *end-to-end* (the same frames
//! also go through x264 into a file). Each configuration runs several times
//! and the median is reported.

use clap::{Args, ValueEnum};
use nova_core::{Error, ErrorCode};
use nova_encoder::{FfmpegSink, HashSink, NullSink, PixelFormat};
use nova_scheduler::{RenderStats, available_threads, render};
use serde_json::{Value, json};

use crate::InputArgs;
use crate::load::{load, print_warnings};
use crate::render::{
    EncodeArgs, PipelineArgs, encoder_config, install_cancel_handler, render_options, stats_json,
};
use crate::report::{Failure, human_bytes, peak_rss_bytes, r3};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Rasterise and convert frames, discard them
    Render,
    /// Rasterise, convert and encode with x264
    Encode,
    /// Both, one after the other
    Both,
}

#[derive(Args)]
pub struct BenchArgs {
    #[command(flatten)]
    pub input: InputArgs,
    /// Thread counts to test, e.g. 1,2,4,8 [default: 1, half and all CPUs]
    #[arg(long, value_delimiter = ',', value_name = "N,N,...")]
    pub threads: Vec<usize>,
    /// Measured runs per configuration (the median is reported)
    #[arg(long, default_value_t = 3)]
    pub runs: usize,
    #[arg(long, value_enum, default_value_t = Mode::Both)]
    pub mode: Mode,
    /// Only use the first N frames of the composition
    #[arg(long, value_name = "N")]
    pub frames: Option<u32>,
    /// Also check that every thread count produces bit-identical frames
    #[arg(long)]
    pub verify: bool,
    #[command(flatten)]
    pub pipeline: PipelineArgs,
    #[command(flatten)]
    pub encode: EncodeArgs,
    #[arg(long)]
    pub json: bool,
}

struct Row {
    mode: &'static str,
    threads: usize,
    /// Wall seconds of every measured run.
    runs: Vec<f64>,
    /// Stats of the median run.
    stats: RenderStats,
}

impl Row {
    fn median_seconds(&self) -> f64 {
        self.stats.wall_ms / 1000.0
    }
}

fn median_index(values: &[f64]) -> usize {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    order[(values.len() - 1) / 2]
}

pub fn cmd_benchmark(args: BenchArgs) -> Result<(), Failure> {
    let json = args.json;
    run(args).map_err(|f| f.with_json(json))
}

fn run(args: BenchArgs) -> Result<(), Failure> {
    if args.runs == 0 {
        return Err(Failure::one(Error::new(
            ErrorCode::Validation,
            "--runs must be at least 1",
        )));
    }
    let profile = args.encode.profile()?;
    let loaded = load(&args.input)?;
    if !args.json {
        print_warnings(&loaded.warnings);
    }
    let scene = &loaded.scene;
    let backend = &loaded.backend;
    let end = args
        .frames
        .map_or(scene.duration_frames, |n| n.clamp(1, scene.duration_frames));
    let cpus = available_threads();
    let mut thread_counts = if args.threads.is_empty() {
        vec![1, (cpus / 2).max(1), cpus]
    } else {
        args.threads.clone()
    };
    thread_counts.retain(|&t| t > 0);
    thread_counts.dedup();
    if thread_counts.is_empty() {
        return Err(Failure::one(Error::new(
            ErrorCode::Validation,
            "--threads needs at least one positive number",
        )));
    }
    let cancel = install_cancel_handler();
    let say = |line: String| {
        if !args.json {
            eprintln!("{line}");
        }
    };
    say(format!(
        "benchmarking `{}`: {}x{} @ {} fps, {end} frames ({:.1}s of video), {} run(s) per configuration, {cpus} CPUs",
        scene.id,
        scene.width,
        scene.height,
        scene.fps.to_ffmpeg(),
        scene.fps.frame_to_seconds(end as i64),
        args.runs
    ));

    // One untimed pass so page cache, CPU frequency and allocator are warm.
    {
        let warm_end = end.min(30);
        let mut sink = NullSink::new(PixelFormat::Yuv420p);
        let opts = render_options(Some(cpus), &args.pipeline, 0, Some(warm_end));
        render(scene, backend, &mut sink, &opts, &cancel, None)?;
    }

    let tmp_dir = std::env::temp_dir();
    let mut rows: Vec<Row> = Vec::new();
    for &threads in &thread_counts {
        let opts = render_options(Some(threads), &args.pipeline, 0, Some(end));
        if args.mode != Mode::Encode {
            let mut all: Vec<RenderStats> = Vec::new();
            for _ in 0..args.runs {
                let mut sink = NullSink::new(PixelFormat::Yuv420p);
                all.push(render(scene, backend, &mut sink, &opts, &cancel, None)?);
            }
            let secs: Vec<f64> = all.iter().map(|s| s.wall_ms / 1000.0).collect();
            let stats = all.swap_remove(median_index(&secs));
            say(format!(
                "  render-only  {threads:>2} threads: {:>7.1} fps",
                stats.fps()
            ));
            rows.push(Row {
                mode: "render-only",
                threads,
                runs: secs,
                stats,
            });
        }
        if args.mode != Mode::Render {
            let mut all: Vec<RenderStats> = Vec::new();
            for run in 0..args.runs {
                let out = tmp_dir.join(format!(
                    "nova-bench-{}-{threads}-{run}.mp4",
                    std::process::id()
                ));
                let config = encoder_config(
                    scene,
                    &out,
                    end,
                    profile.clone(),
                    args.encode.encoder_threads,
                    false,
                );
                let mut sink = FfmpegSink::start(config)?;
                let result = render(scene, backend, &mut sink, &opts, &cancel, None);
                let finished = match result {
                    Ok(mut stats) => sink.finish().map(|report| {
                        // End-to-end time includes x264 flushing after the last frame.
                        stats.wall_ms = report.total_ms.max(stats.wall_ms);
                        stats
                    }),
                    Err(e) => Err(e),
                };
                let _ = std::fs::remove_file(&out);
                all.push(finished?);
            }
            let secs: Vec<f64> = all.iter().map(|s| s.wall_ms / 1000.0).collect();
            let stats = all.swap_remove(median_index(&secs));
            say(format!(
                "  end-to-end   {threads:>2} threads: {:>7.1} fps",
                stats.fps()
            ));
            rows.push(Row {
                mode: "end-to-end",
                threads,
                runs: secs,
                stats,
            });
        }
    }

    let verified = if args.verify {
        let mut digests = Vec::new();
        for &threads in &thread_counts {
            let mut sink = HashSink::new(PixelFormat::Rgba);
            render(
                scene,
                backend,
                &mut sink,
                &render_options(Some(threads), &args.pipeline, 0, Some(end)),
                &cancel,
                None,
            )?;
            digests.push(sink.digest());
        }
        Some(digests.windows(2).all(|w| w[0] == w[1]))
    } else {
        None
    };

    let comp_fps = scene.fps.as_f64();
    let baseline = |mode: &str| {
        rows.iter()
            .find(|r| r.mode == mode)
            .map(|r| r.median_seconds())
    };
    if args.json {
        let out: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "mode": r.mode,
                    "threads": r.threads,
                    "median_seconds": r3(r.median_seconds()),
                    "run_seconds": r.runs.iter().map(|s| r3(*s)).collect::<Vec<_>>(),
                    "fps": r3(r.stats.fps()),
                    "x_realtime": r3(r.stats.fps() / comp_fps),
                    "speedup_vs_first": baseline(r.mode).map(|b| r3(b / r.median_seconds())),
                    "stats": stats_json(&r.stats),
                })
            })
            .collect();
        let report = json!({
            "ok": true,
            "composition": { "id": scene.id, "width": scene.width, "height": scene.height, "fps": comp_fps },
            "frames": end,
            "cpus": cpus,
            "encoder": { "preset": profile.preset, "crf": profile.crf, "threads": args.encode.encoder_threads },
            "identical_across_thread_counts": verified,
            "peak_rss_bytes": peak_rss_bytes(),
            "results": out,
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return Ok(());
    }

    println!();
    println!(
        "{:<12} {:>7} {:>9} {:>8} {:>10} {:>8} {:>9} {:>9} {:>8}",
        "mode", "threads", "median s", "fps", "x realtime", "speedup", "p50 ms", "p95 ms", "reused"
    );
    for r in &rows {
        println!(
            "{:<12} {:>7} {:>9.2} {:>8.1} {:>10.2} {:>8.2} {:>9.2} {:>9.2} {:>8}",
            r.mode,
            r.threads,
            r.median_seconds(),
            r.stats.fps(),
            r.stats.fps() / comp_fps,
            baseline(r.mode).map_or(1.0, |b| b / r.median_seconds()),
            r.stats.frame_ms_p50,
            r.stats.frame_ms_p95,
            r.stats.frames_reused
        );
    }
    println!();
    if rows.iter().any(|r| r.mode == "end-to-end") {
        println!(
            "end-to-end encoder: x264 {} crf {}",
            profile.preset, profile.crf
        );
    }
    if let Some(same) = verified {
        println!(
            "frames identical across thread counts: {}",
            if same { "yes" } else { "NO — this is a bug" }
        );
    }
    if let Some(rss) = peak_rss_bytes() {
        println!("peak memory: {}", human_bytes(rss));
    }
    if verified == Some(false) {
        return Err(Failure::one(Error::new(
            ErrorCode::Internal,
            "output differs between thread counts",
        )));
    }
    Ok(())
}
