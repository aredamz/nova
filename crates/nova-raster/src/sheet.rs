//! Contact sheets: many frames of one composition on a single image, so a
//! person (or an AI agent checking its own template) can judge a whole video
//! at a glance without watching it.

use nova_core::{Error, ErrorCode, Result};
use skia_safe::{
    AlphaType, Color, ColorType, Data, FilterMode, Font, FontMgr, FontStyle, ImageInfo, MipmapMode,
    Paint, Rect, SamplingOptions, images, surfaces,
};

pub struct SheetCell {
    /// Caption drawn under the frame, e.g. "f 120 · 4.00s".
    pub label: String,
    /// Opaque RGBA8 frame.
    pub rgba: Vec<u8>,
}

const GAP: f32 = 12.0;
const LABEL_HEIGHT: f32 = 26.0;

/// Lay the cells out in `columns` columns, each frame scaled to `cell_width`.
/// Returns RGBA8 pixels and the sheet's size.
pub fn contact_sheet(
    cells: &[SheetCell],
    frame_width: u32,
    frame_height: u32,
    columns: u32,
    cell_width: u32,
) -> Result<(Vec<u8>, u32, u32)> {
    if cells.is_empty() || columns == 0 || cell_width == 0 {
        return Err(Error::new(
            ErrorCode::Validation,
            "a contact sheet needs at least one frame and one column",
        ));
    }
    let columns = columns.min(cells.len() as u32);
    let rows = (cells.len() as u32).div_ceil(columns);
    let cw = cell_width as f32;
    let ch = (cw * frame_height as f32 / frame_width as f32).round();
    let sheet_w = (GAP + columns as f32 * (cw + GAP)).ceil() as u32;
    let sheet_h = (GAP + rows as f32 * (ch + LABEL_HEIGHT + GAP)).ceil() as u32;

    let info = ImageInfo::new(
        (sheet_w as i32, sheet_h as i32),
        ColorType::RGBA8888,
        AlphaType::Premul,
        None,
    );
    let mut pixels = vec![0u8; sheet_w as usize * sheet_h as usize * 4];
    {
        let mut surface =
            surfaces::wrap_pixels(&info, &mut pixels, None, None).ok_or_else(|| {
                Error::new(
                    ErrorCode::Render,
                    "could not create the contact sheet surface",
                )
            })?;
        let canvas = surface.canvas();
        canvas.clear(Color::from_rgb(24, 24, 28));

        // Captions use whatever default font the host has; without one the
        // sheet is simply unlabelled.
        let font = FontMgr::new()
            .legacy_make_typeface(None, FontStyle::normal())
            .map(|tf| Font::from_typeface(tf, 15.0));
        let mut label_paint = Paint::default();
        label_paint.set_anti_alias(true);
        label_paint.set_color(Color::from_rgb(200, 200, 210));
        let mut border = Paint::default();
        border.set_color(Color::from_rgb(70, 70, 80));
        border.set_style(skia_safe::PaintStyle::Stroke);

        let frame_info = ImageInfo::new(
            (frame_width as i32, frame_height as i32),
            ColorType::RGBA8888,
            AlphaType::Opaque,
            None,
        );
        let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear);
        for (i, cell) in cells.iter().enumerate() {
            let (col, row) = (i as u32 % columns, i as u32 / columns);
            let x = GAP + col as f32 * (cw + GAP);
            let y = GAP + row as f32 * (ch + LABEL_HEIGHT + GAP);
            let dst = Rect::from_xywh(x, y, cw, ch);
            let image = images::raster_from_data(
                &frame_info,
                Data::new_copy(&cell.rgba),
                frame_width as usize * 4,
            )
            .and_then(|img| img.with_default_mipmaps().or(Some(img)))
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::Render,
                    "frame has the wrong size for the contact sheet",
                )
            })?;
            canvas.draw_image_rect_with_sampling_options(
                &image,
                None,
                dst,
                sampling,
                &Paint::default(),
            );
            canvas.draw_rect(dst.with_outset((0.5, 0.5)), &border);
            if let Some(font) = &font {
                canvas.draw_str(&cell.label, (x + 2.0, y + ch + 18.0), font, &label_paint);
            }
        }
    }
    Ok((pixels, sheet_w, sheet_h))
}

/// Number of font families the host offers as fallback (for `nova doctor`).
pub fn system_font_family_count() -> usize {
    FontMgr::new().count_families()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_a_grid() {
        let cell = |v: u8| SheetCell {
            label: format!("f {v}"),
            rgba: vec![v; 64 * 36 * 4],
        };
        let cells: Vec<SheetCell> = (0..5).map(|i| cell(40 * i + 20)).collect();
        let (px, w, h) = contact_sheet(&cells, 64, 36, 3, 128).unwrap();
        // 3 columns x 2 rows of 128x72 cells
        assert_eq!(w, 12 + 3 * 140);
        assert_eq!(h, 12 + 2 * (72 + 26 + 12));
        assert_eq!(px.len(), (w * h * 4) as usize);
        // centre of the first cell shows the first frame's grey
        let (cx, cy) = (12 + 64, 12 + 36);
        let i = ((cy * w + cx) * 4) as usize;
        assert_eq!(&px[i..i + 3], &[20, 20, 20]);
    }

    #[test]
    fn rejects_empty_input() {
        assert!(contact_sheet(&[], 64, 36, 3, 128).is_err());
    }
}
