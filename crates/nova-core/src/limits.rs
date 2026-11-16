/// Resource limits applied to untrusted templates before any pixel is drawn.
/// Defaults are sized for a 16 vCPU / 32 GB box and stop inputs that could
/// exhaust RAM or disk; all of them are configurable.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_total_pixels: u64,
    pub max_duration_frames: u32,
    pub max_layers: usize,
    pub max_nesting_depth: usize,
    pub max_keyframes_per_track: usize,
    pub max_path_data_bytes: usize,
    pub max_text_bytes: usize,
    pub max_assets: usize,
    pub max_asset_file_bytes: u64,
    /// Decoded size of one raster image (width * height * 4).
    pub max_decoded_image_bytes: u64,
    /// Largest side an SVG is ever rasterised at.
    pub max_svg_raster_side: u32,
    pub max_blur_sigma: f32,
    /// Decompressed size of one baked frame (its whole display list).
    pub max_frame_bytes: u32,
    /// Shard files one frame pack may be split into.
    pub max_pack_shards: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_width: 7680,
            max_height: 7680,
            max_total_pixels: 3840 * 2160,
            max_duration_frames: 60 * 60 * 60, // 1 h at 60 fps
            max_layers: 5_000,
            max_nesting_depth: 32,
            max_keyframes_per_track: 2_000,
            max_path_data_bytes: 256 * 1024,
            max_text_bytes: 16 * 1024,
            max_assets: 512,
            max_asset_file_bytes: 64 * 1024 * 1024,
            max_decoded_image_bytes: 256 * 1024 * 1024,
            max_svg_raster_side: 4096,
            max_blur_sigma: 250.0,
            max_frame_bytes: 64 * 1024 * 1024,
            max_pack_shards: 256,
        }
    }
}
