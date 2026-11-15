//! The render pipeline.
//!
//! ```text
//!   worker 0 ─┐
//!   worker 1 ─┼─> reorder buffer ─> writer (caller's thread) ─> FrameSink
//!   worker N ─┘        ^                         │
//!                      └──── in-flight window ───┘
//! ```
//!
//! Frames are independent, so workers render them in parallel in small
//! consecutive chunks. The writer hands them to the sink strictly in order.
//! Memory is bounded by construction: a worker may only start a frame that is
//! less than `window` frames ahead of the next frame the sink is waiting for,
//! so when the encoder is the slow side, rendering pauses instead of piling
//! frames up in RAM.
//!
//! Identical consecutive frames are detected by comparing their frame plans
//! (exact, no heuristics) and are not rendered again; the sink still receives
//! every frame.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nova_core::{Error, ErrorCode, FramePlan, Result, Scene};
use nova_encoder::{FrameSink, PixelFormat, yuv};
use nova_raster::{RasterBackend, RendererStats};

#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Render threads. 0 = one per available CPU.
    pub threads: usize,
    /// Most frames that may be started but not yet written. 0 = 3 x threads.
    pub window: usize,
    /// Consecutive frames a worker takes at a time. 0 = 4.
    pub chunk: usize,
    /// First frame to render.
    pub start: u32,
    /// One past the last frame to render; `None` = end of the composition.
    pub end: Option<u32>,
    /// Skip rendering frames whose plan equals the previous frame's.
    pub reuse_identical_frames: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            threads: 0,
            window: 0,
            chunk: 0,
            start: 0,
            end: None,
            reuse_identical_frames: true,
        }
    }
}

pub fn available_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub frames_done: u32,
    pub frames_total: u32,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct RenderStats {
    pub threads: usize,
    pub window: usize,
    pub chunk: usize,
    pub frames: u32,
    /// Frames that were actually rasterised.
    pub frames_rendered: u32,
    /// Frames reused from the previous one.
    pub frames_reused: u32,
    pub wall_ms: f64,
    /// Sum over workers of time spent evaluating, rasterising and converting.
    pub worker_busy_ms: f64,
    /// Per rendered frame: evaluate + rasterise + colour conversion.
    pub frame_ms_p50: f64,
    pub frame_ms_p95: f64,
    pub frame_ms_p99: f64,
    pub frame_ms_max: f64,
    /// Time the writer spent inside the sink (encoder backpressure).
    pub sink_ms: f64,
    /// Time the writer spent waiting for the next frame (renderer is the slow side).
    pub writer_wait_ms: f64,
    pub renderer: RendererStats,
}

impl RenderStats {
    pub fn fps(&self) -> f64 {
        if self.wall_ms > 0.0 {
            self.frames as f64 * 1000.0 / self.wall_ms
        } else {
            0.0
        }
    }
}

enum Output {
    Pixels(Vec<u8>),
    /// Identical to the frame before it.
    SameAsPrevious,
}

struct State {
    next_to_assign: u32,
    next_to_write: u32,
    ready: BTreeMap<u32, Output>,
    pool: Vec<Vec<u8>>,
    failure: Option<Error>,
    frame_ms: Vec<f32>,
    renderer: RendererStats,
    busy_ms: f64,
}

struct Shared {
    state: Mutex<State>,
    /// Workers wait here for the window to move.
    can_start: Condvar,
    /// The writer waits here for its next frame.
    frame_ready: Condvar,
    stop: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panicking worker is caught outside the lock, but never let a
        // poisoned mutex turn one failure into a hang.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn fail(&self, error: Error) {
        let mut st = self.lock();
        if st.failure.is_none() {
            st.failure = Some(error);
        }
        drop(st);
        self.stop.store(true, Ordering::SeqCst);
        self.can_start.notify_all();
        self.frame_ready.notify_all();
    }
}

/// Render frames `[start, end)` of `scene` into `sink`, in order.
///
/// `cancel` may be set from another thread (or a signal handler) to stop the
/// job; the function then returns a `Cancelled` error after all workers exit.
pub fn render(
    scene: &Arc<Scene>,
    backend: &dyn RasterBackend,
    sink: &mut dyn FrameSink,
    opts: &RenderOptions,
    cancel: &AtomicBool,
    mut on_progress: Option<&mut dyn FnMut(Progress)>,
) -> Result<RenderStats> {
    let end = opts
        .end
        .unwrap_or(scene.duration_frames)
        .min(scene.duration_frames);
    let start = opts.start;
    if start >= end {
        return Err(Error::new(
            ErrorCode::Validation,
            format!(
                "frame range {start}..{end} is empty (the composition has {} frames)",
                scene.duration_frames
            ),
        ));
    }
    let total = end - start;
    let threads = if opts.threads == 0 {
        available_threads()
    } else {
        opts.threads
    }
    .clamp(1, total as usize);
    let chunk = if opts.chunk == 0 { 4 } else { opts.chunk }.max(1);
    let window = if opts.window == 0 {
        threads * 3
    } else {
        opts.window
    }
    .max(threads);
    let format = sink.format();
    let out_len = format.frame_len(scene.width, scene.height);
    let reuse = opts.reuse_identical_frames;

    let shared = Shared {
        state: Mutex::new(State {
            next_to_assign: start,
            next_to_write: start,
            ready: BTreeMap::new(),
            pool: Vec::new(),
            failure: None,
            frame_ms: Vec::with_capacity(total as usize),
            renderer: RendererStats::default(),
            busy_ms: 0.0,
        }),
        can_start: Condvar::new(),
        frame_ready: Condvar::new(),
        stop: AtomicBool::new(false),
    };
    let started = Instant::now();
    let mut stats = RenderStats {
        threads,
        window,
        chunk,
        frames: total,
        ..RenderStats::default()
    };

    std::thread::scope(|scope| {
        for worker in 0..threads {
            let shared = &shared;
            std::thread::Builder::new()
                .name(format!("nova-render-{worker}"))
                .spawn_scoped(scope, move || {
                    let run = || {
                        worker_loop(
                            scene, backend, shared, start, end, chunk, window, format, out_len,
                            reuse,
                        )
                    };
                    match catch_unwind(AssertUnwindSafe(run)) {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => shared.fail(e),
                        Err(panic) => {
                            let what = panic
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_else(|| "unknown panic".into());
                            shared.fail(Error::new(
                                ErrorCode::Internal,
                                format!("render worker {worker} panicked: {what}"),
                            ));
                        }
                    }
                })
                .expect("spawn render worker");
        }

        // ---- writer: this thread -------------------------------------------------
        let mut last: Option<Vec<u8>> = None;
        for frame in start..end {
            let waited = Instant::now();
            let output = {
                let mut st = shared.lock();
                loop {
                    if st.failure.is_some() {
                        break None;
                    }
                    if cancel.load(Ordering::Relaxed) {
                        st.failure = Some(Error::new(ErrorCode::Cancelled, "render cancelled"));
                        break None;
                    }
                    if let Some(out) = st.ready.remove(&frame) {
                        break Some(out);
                    }
                    // Timed wait so a cancel request is noticed promptly.
                    st = shared
                        .frame_ready
                        .wait_timeout(st, Duration::from_millis(100))
                        .unwrap_or_else(|p| p.into_inner())
                        .0;
                }
            };
            stats.writer_wait_ms += waited.elapsed().as_secs_f64() * 1000.0;
            let Some(output) = output else { break };

            let wrote = Instant::now();
            let result = match output {
                Output::Pixels(buf) => {
                    let r = sink.write(frame, &buf);
                    // The previous buffer is no longer needed: recycle it.
                    if let Some(old) = last.replace(buf) {
                        shared.lock().pool.push(old);
                    }
                    stats.frames_rendered += 1;
                    r
                }
                Output::SameAsPrevious => {
                    stats.frames_reused += 1;
                    match &last {
                        Some(buf) => sink.write(frame, buf),
                        None => Err(Error::new(
                            ErrorCode::Internal,
                            "first frame marked as a repeat",
                        )),
                    }
                }
            };
            stats.sink_ms += wrote.elapsed().as_secs_f64() * 1000.0;
            if let Err(e) = result {
                shared.fail(e);
                break;
            }
            {
                let mut st = shared.lock();
                st.next_to_write = frame + 1;
            }
            shared.can_start.notify_all();
            // Frames only move forward: let the backend drop what it cached
            // for layers that have ended.
            if (frame + 1 - start).is_multiple_of(32) {
                backend.release_frames_before(frame + 1);
            }
            if let Some(cb) = on_progress.as_mut() {
                cb(Progress {
                    frames_done: frame + 1 - start,
                    frames_total: total,
                    elapsed: started.elapsed(),
                });
            }
        }
        // Normal completion or failure: either way release any waiting worker.
        shared.stop.store(true, Ordering::SeqCst);
        shared.can_start.notify_all();
    });

    let mut st = shared.state.into_inner().unwrap_or_else(|p| p.into_inner());
    if let Some(e) = st.failure.take() {
        return Err(e);
    }
    stats.wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    stats.worker_busy_ms = st.busy_ms;
    stats.renderer = st.renderer;
    st.frame_ms.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| -> f64 {
        if st.frame_ms.is_empty() {
            return 0.0;
        }
        let i = ((st.frame_ms.len() - 1) as f64 * p).round() as usize;
        st.frame_ms[i] as f64
    };
    stats.frame_ms_p50 = pct(0.50);
    stats.frame_ms_p95 = pct(0.95);
    stats.frame_ms_p99 = pct(0.99);
    stats.frame_ms_max = st.frame_ms.last().copied().unwrap_or(0.0) as f64;
    Ok(stats)
}

#[allow(clippy::too_many_arguments)]
fn worker_loop(
    scene: &Arc<Scene>,
    backend: &dyn RasterBackend,
    shared: &Shared,
    start: u32,
    end: u32,
    chunk: usize,
    window: usize,
    format: PixelFormat,
    out_len: usize,
    reuse: bool,
) -> Result<()> {
    let mut renderer = backend.new_renderer()?;
    let (w, h) = (scene.width as usize, scene.height as usize);
    // Scratch RGBA surface, only needed when the sink wants another format.
    let mut rgba = if format == PixelFormat::Rgba {
        Vec::new()
    } else {
        vec![0u8; w * h * 4]
    };
    let mut times: Vec<f32> = Vec::new();
    let mut busy = 0.0f64;

    let result = (|| -> Result<()> {
        loop {
            // ---- take a chunk, respecting the in-flight window --------------------
            let (first, last) = {
                let mut st = shared.lock();
                loop {
                    if shared.stop.load(Ordering::SeqCst)
                        || st.failure.is_some()
                        || st.next_to_assign >= end
                    {
                        return Ok(());
                    }
                    if (st.next_to_assign as u64) < st.next_to_write as u64 + window as u64 {
                        break;
                    }
                    st = shared.can_start.wait(st).unwrap_or_else(|p| p.into_inner());
                }
                let first = st.next_to_assign;
                let last = (first + chunk as u32).min(end);
                st.next_to_assign = last;
                (first, last)
            };

            // The frame before the chunk decides whether the first one is a repeat.
            let mut previous: Option<FramePlan> = if reuse && first > start {
                Some(scene.evaluate(first - 1))
            } else {
                None
            };

            for frame in first..last {
                if shared.stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let t0 = Instant::now();
                let plan = scene.evaluate(frame);
                let output = if reuse && previous.as_ref() == Some(&plan) {
                    Output::SameAsPrevious
                } else {
                    let mut buf = shared.lock().pool.pop().unwrap_or_default();
                    buf.resize(out_len, 0);
                    match format {
                        PixelFormat::Rgba => renderer.render(&plan, &mut buf)?,
                        PixelFormat::Yuv420p => {
                            renderer.render(&plan, &mut rgba)?;
                            yuv::rgba_to_i420(&rgba, w, h, &mut buf);
                        }
                    }
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    times.push(ms as f32);
                    Output::Pixels(buf)
                };
                busy += t0.elapsed().as_secs_f64() * 1000.0;
                previous = Some(plan);

                shared.lock().ready.insert(frame, output);
                shared.frame_ready.notify_one();
            }
        }
    })();

    let mut st = shared.lock();
    st.frame_ms.extend_from_slice(&times);
    st.busy_ms += busy;
    st.renderer.add(&renderer.stats());
    drop(st);
    result
}

/// Render one frame to RGBA (previews, contact sheets, tests).
pub fn render_frame_rgba(
    scene: &Arc<Scene>,
    backend: &dyn RasterBackend,
    frame: u32,
) -> Result<Vec<u8>> {
    if frame >= scene.duration_frames {
        return Err(Error::new(
            ErrorCode::Validation,
            format!(
                "frame {frame} is out of range; the composition has frames 0..{}",
                scene.duration_frames
            ),
        ));
    }
    let mut renderer = backend.new_renderer()?;
    let mut rgba = vec![0u8; PixelFormat::Rgba.frame_len(scene.width, scene.height)];
    renderer.render(&scene.evaluate(frame), &mut rgba)?;
    Ok(rgba)
}
