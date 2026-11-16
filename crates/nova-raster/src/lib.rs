//! NOVA raster layer: turns a [`FramePlan`] into pixels.
//!
//! The rest of the engine only knows the two traits below. A backend receives
//! the immutable scene once (to decode and cache its assets) and then renders
//! plans; it never sees the template DSL, the scheduler or the encoder, and
//! no backend type leaks out of this crate.

pub mod png;
pub mod sheet;
pub mod skia;

use nova_core::{FramePlan, Result};

pub use skia::{PrepareOptions, PrepareReport, SkiaBackend};

/// A prepared backend: assets decoded, ready to hand out per-thread renderers.
pub trait RasterBackend: Send + Sync {
    fn name(&self) -> &'static str;

    /// Create the state one render thread owns. Called on that thread, since
    /// a renderer may hold objects that must not cross threads.
    fn new_renderer(&self) -> Result<Box<dyn FrameRenderer>>;

    /// Hint that no frame before `frame` will be rendered again, so anything
    /// cached only for earlier frames can be freed.
    fn release_frames_before(&self, _frame: u32) {}
}

/// Per-thread renderer. Deterministic: equal plans produce equal pixels, in
/// any order, on any thread.
pub trait FrameRenderer {
    /// Render `plan` into `rgba`: `width * height * 4` bytes, rows tightly
    /// packed, RGBA order, fully opaque.
    fn render(&mut self, plan: &FramePlan, rgba: &mut [u8]) -> Result<()>;

    fn stats(&self) -> RendererStats;
}

/// Counters for one renderer, summed across threads for the job report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RendererStats {
    pub frames: u64,
    /// Text layouts built (cache misses).
    pub text_layouts: u64,
    /// Text draws served from the layout cache.
    pub text_cache_hits: u64,
    /// Offscreen layers allocated for group opacity, blend modes and filters.
    pub offscreen_layers: u64,
    /// Cached layers composited without re-rendering their content.
    pub layer_cache_hits: u64,
    /// Cached layers that had to be rendered (first use, or content changed).
    pub layer_cache_misses: u64,
    /// Baked frames drawn, and where their time went (microseconds).
    pub baked_frames: u64,
    pub baked_read_us: u64,
    pub baked_parse_us: u64,
    pub baked_raster_us: u64,
}

impl RendererStats {
    pub fn add(&mut self, o: &RendererStats) {
        self.frames += o.frames;
        self.text_layouts += o.text_layouts;
        self.text_cache_hits += o.text_cache_hits;
        self.offscreen_layers += o.offscreen_layers;
        self.layer_cache_hits += o.layer_cache_hits;
        self.layer_cache_misses += o.layer_cache_misses;
        self.baked_frames += o.baked_frames;
        self.baked_read_us += o.baked_read_us;
        self.baked_parse_us += o.baked_parse_us;
        self.baked_raster_us += o.baked_raster_us;
    }
}
