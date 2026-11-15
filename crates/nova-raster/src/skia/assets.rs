//! Asset preparation for the Skia backend: decode once, share everywhere.
//!
//! Images are never resampled from their full resolution on every frame.
//! For each image layer the largest size it is ever shown at is known from
//! the scene, so a *variant* is prepared once at exactly that size: raster
//! images with a Lanczos filter, SVGs straight from the vector data. Frames
//! then draw the variant at (or just under) 1:1, which is both sharper and
//! several times cheaper than filtering a large source per frame.

use std::io::Cursor;
use std::sync::Arc;

use image::ImageDecoder;
use nova_core::Limits;
use resvg::{tiny_skia, usvg};
use skia_safe::{AlphaType, ColorType, Data, FontMgr, Image, ImageInfo, Typeface, images};

/// Decoded raster image: premultiplied RGBA8, tightly packed.
pub struct Decoded {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Premultiplied RGBA8 pixels -> immutable Skia image, optionally with
/// mipmaps (only needed if it can be drawn at less than half size).
pub fn image_from_premul(
    pixels: &[u8],
    width: u32,
    height: u32,
    mipmaps: bool,
) -> Result<Image, String> {
    // Telling Skia a bitmap has no transparency lets it copy pixels instead
    // of blending them, and lets shadows be drawn without an offscreen pass.
    let opaque = pixels.chunks_exact(4).all(|px| px[3] == 255);
    let alpha = if opaque {
        AlphaType::Opaque
    } else {
        AlphaType::Premul
    };
    let info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::RGBA8888,
        alpha,
        None,
    );
    let image = images::raster_from_data(&info, Data::new_copy(pixels), width as usize * 4)
        .ok_or("could not create an image from the decoded pixels")?;
    Ok(if mipmaps {
        image.with_default_mipmaps().unwrap_or(image)
    } else {
        image
    })
}

fn premultiply(pixels: &mut [u8]) {
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            // exact rounding of c * a / 255
            let v = *c as u32 * a + 128;
            *c = ((v + (v >> 8)) >> 8) as u8;
        }
    }
}

/// Decode PNG/JPEG/WebP, honouring EXIF orientation, within the memory limit.
pub fn decode_image(bytes: &[u8], limits: &Limits) -> Result<Decoded, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("cannot read image: {e}"))?;
    if reader.format().is_none() {
        return Err("unrecognised image format (supported: PNG, JPEG, WebP)".into());
    }
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(limits.max_decoded_image_bytes);
    reader.limits(lim);

    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("cannot decode image: {e}"))?;
    let (w, h) = decoder.dimensions();
    if w as u64 * h as u64 * 4 > limits.max_decoded_image_bytes {
        return Err(format!(
            "image is {w}x{h}; decoded it would need {} bytes, over the limit of {}",
            w as u64 * h as u64 * 4,
            limits.max_decoded_image_bytes
        ));
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("cannot decode image: {e}"))?;
    img.apply_orientation(orientation);
    let mut rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    premultiply(&mut rgba);
    Ok(Decoded {
        pixels: rgba.into_raw(),
        width,
        height,
    })
}

/// Cut `crop` (x, y, w, h in source pixels) out of a decoded image and scale
/// it to `width` x `height` with a Lanczos-3 filter. Filtering premultiplied
/// pixels is what keeps transparent edges free of dark or bright fringes.
pub fn resize(src: &Decoded, crop: (u32, u32, u32, u32), width: u32, height: u32) -> Vec<u8> {
    let view: image::ImageBuffer<image::Rgba<u8>, &[u8]> =
        image::ImageBuffer::from_raw(src.width, src.height, src.pixels.as_slice())
            .expect("decoded buffer size");
    let (x, y, w, h) = crop;
    let cropped = image::imageops::crop_imm(&view, x, y, w, h);
    let mut out = image::imageops::resize(
        &*cropped,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    )
    .into_raw();
    // Lanczos overshoots slightly at hard edges; a premultiplied colour may
    // never exceed its alpha.
    for px in out.chunks_exact_mut(4) {
        let a = px[3];
        px[0] = px[0].min(a);
        px[1] = px[1].min(a);
        px[2] = px[2].min(a);
    }
    out
}

/// A parsed SVG. Rasterised on demand at whatever size a layer needs.
pub struct SvgDoc {
    tree: usvg::Tree,
    pub natural: (f32, f32),
}

/// Parse an SVG. Nothing outside the document is ever loaded: file and URL
/// references in `<image href>` resolve to nothing; only embedded `data:`
/// images work.
pub fn parse_svg(bytes: &[u8], fontdb: Arc<usvg::fontdb::Database>) -> Result<SvgDoc, String> {
    let opt = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_href, _opts| None),
        },
        fontdb,
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| format!("cannot parse SVG: {e}"))?;
    let size = tree.size();
    if !(size.width() > 0.0 && size.height() > 0.0) {
        return Err("SVG has no size".into());
    }
    Ok(SvgDoc {
        tree,
        natural: (size.width(), size.height()),
    })
}

/// True when the SVG source contains text, which needs fonts to convert.
pub fn svg_has_text(bytes: &[u8]) -> bool {
    bytes.windows(5).any(|w| w == b"<text")
}

/// Rasterise the region `crop` (x, y, w, h in SVG units) of the document
/// into exactly `width` x `height` pixels, directly from the vector data.
pub fn rasterize_svg(
    doc: &SvgDoc,
    crop: (f32, f32, f32, f32),
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let (x, y, w, h) = crop;
    let mut pixmap =
        tiny_skia::Pixmap::new(width, height).ok_or("could not allocate the SVG raster")?;
    let transform = tiny_skia::Transform::from_translate(-x, -y)
        .post_scale(width as f32 / w, height as f32 / h);
    resvg::render(&doc.tree, transform, &mut pixmap.as_mut());
    // tiny-skia pixmaps are premultiplied RGBA8, exactly what Skia expects.
    Ok(pixmap.take())
}

/// Load a font file into a typeface that every thread can share.
pub fn load_typeface(bytes: &Arc<[u8]>) -> Result<Typeface, String> {
    FontMgr::new()
        .new_from_data(Data::new_copy(bytes), None)
        .ok_or_else(|| "not a usable font file (supported: TTF, OTF, WOFF2 is not)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiply_is_exact() {
        let mut px = [255, 128, 0, 128, 10, 20, 30, 255, 200, 200, 200, 0];
        premultiply(&mut px);
        assert_eq!(&px[..4], &[128, 64, 0, 128]);
        assert_eq!(&px[4..8], &[10, 20, 30, 255], "opaque pixels untouched");
        assert_eq!(&px[8..], &[0, 0, 0, 0]);
    }

    #[test]
    fn rejects_non_images() {
        let err = decode_image(b"definitely not an image", &Limits::default())
            .err()
            .unwrap();
        assert!(err.contains("unrecognised image format"), "{err}");
    }

    #[test]
    fn resize_crops_then_scales_and_keeps_premultiplied_pixels_legal() {
        // 8x4: left half opaque red, right half fully transparent.
        let mut pixels = Vec::new();
        for _y in 0..4 {
            for x in 0..8 {
                pixels.extend_from_slice(if x < 4 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        let src = Decoded {
            pixels,
            width: 8,
            height: 4,
        };
        // crop the left half and halve it: solid red
        let left = resize(&src, (0, 0, 4, 4), 2, 2);
        assert!(
            left.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
            "{left:?}"
        );
        // across the edge: colour never exceeds alpha, no fringe values
        let across = resize(&src, (0, 0, 8, 4), 4, 2);
        assert!(
            across
                .chunks_exact(4)
                .all(|p| p[0] <= p[3] && p[1] == 0 && p[2] == 0),
            "{across:?}"
        );
        assert_eq!(across.len(), 4 * 2 * 4);
    }

    #[test]
    fn svg_rasterises_any_region_at_any_size() {
        // left half red, right half blue
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
            <rect width="20" height="20" fill="#f00"/><rect x="20" width="20" height="20" fill="#00f"/></svg>"##;
        let doc = parse_svg(svg, Arc::new(usvg::fontdb::Database::new())).unwrap();
        assert_eq!(doc.natural, (40.0, 20.0));
        let whole = rasterize_svg(&doc, (0.0, 0.0, 40.0, 20.0), 120, 60).unwrap();
        assert_eq!(whole.len(), 120 * 60 * 4);
        assert_eq!(&whole[..4], &[255, 0, 0, 255]);
        assert_eq!(&whole[(119 * 4)..(120 * 4)], &[0, 0, 255, 255]);
        // only the right half, at another size: all blue
        let right = rasterize_svg(&doc, (20.0, 0.0, 20.0, 20.0), 50, 50).unwrap();
        assert!(right.chunks_exact(4).all(|p| p == [0, 0, 255, 255]));
    }

    #[test]
    fn svg_cannot_load_external_files() {
        // An <image> pointing at a local file must resolve to nothing rather than read it.
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="10" height="10">
            <image width="10" height="10" xlink:href="/etc/hostname"/></svg>"##;
        let doc = parse_svg(svg, Arc::new(usvg::fontdb::Database::new())).unwrap();
        let px = rasterize_svg(&doc, (0.0, 0.0, 10.0, 10.0), 64, 64).unwrap();
        assert!(px.iter().all(|&b| b == 0), "external image must not render");
    }
}
