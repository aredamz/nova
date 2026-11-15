//! PNG output for previews, contact sheets and golden tests.

use std::path::Path;

use nova_core::{Error, ErrorCode, Result};

pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(
        encoder,
        rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| Error::new(ErrorCode::Io, format!("PNG encoding failed: {e}")))?;
    Ok(out)
}

pub fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<()> {
    let bytes = encode_png(rgba, width, height)?;
    std::fs::write(path, bytes)
        .map_err(|e| Error::io(format!("cannot write `{}`", path.display()), e))
}

/// Decode a PNG to tightly packed RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| Error::new(ErrorCode::Io, format!("PNG decoding failed: {e}")))?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}
