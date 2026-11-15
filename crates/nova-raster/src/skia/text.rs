//! Text shaping and layout through Skia's paragraph module (HarfBuzz + ICU),
//! the same shaping stack a browser uses.

use nova_core::{Color, TextAlign, TextNode};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextAlign as SkTextAlign,
    TextStyle, TypefaceFontProvider,
};
use skia_safe::{FontHinting, FontMgr, FontStyle, Typeface, font::Edging};

/// Build the per-thread font collection: template fonts first, then (only if
/// allowed) the host's fonts as fallback for missing glyphs and families.
pub fn font_collection(fonts: &[(String, Typeface)], system_fonts: bool) -> FontCollection {
    let mut provider = TypefaceFontProvider::new();
    for (family, typeface) in fonts {
        provider.register_typeface(typeface.clone(), Some(family.as_str()));
    }
    let mut collection = FontCollection::new();
    collection.set_asset_font_manager(Some(provider.into()));
    if system_fonts {
        collection.set_default_font_manager(FontMgr::new(), None);
    }
    collection
}

/// A shaped, wrapped paragraph and the box it occupies.
pub struct TextLayout {
    pub paragraph: Paragraph,
    pub width: f32,
    pub height: f32,
}

pub fn layout(
    fonts: &FontCollection,
    text: &TextNode,
    size: f32,
    color: Color,
    letter_spacing: f32,
    reveal: Option<u32>,
) -> TextLayout {
    let mut style = TextStyle::new();
    let [r, g, b, a] = color.to_rgba8();
    style.set_color(skia_safe::Color::from_argb(a, r, g, b));
    style.set_font_size(size);
    style.set_font_families(&text.families);
    style.set_font_style(FontStyle::new(
        Weight::from(text.weight as i32),
        Width::NORMAL,
        if text.italic {
            Slant::Italic
        } else {
            Slant::Upright
        },
    ));
    style.set_letter_spacing(letter_spacing);
    if let Some(line_height) = text.line_height {
        // CSS line-height: the extra leading is split evenly above and below.
        style.set_height(line_height);
        style.set_height_override(true);
        style.set_half_leading(true);
    }
    // Greyscale antialiasing with subpixel positioning and no hinting: glyphs
    // keep their shape while moving or scaling, so animated text never jitters.
    style.set_font_edging(Edging::AntiAlias);
    style.set_subpixel(true);
    style.set_font_hinting(FontHinting::None);

    let mut para_style = ParagraphStyle::new();
    para_style.set_text_align(match text.align {
        TextAlign::Left => SkTextAlign::Left,
        TextAlign::Center => SkTextAlign::Center,
        TextAlign::Right => SkTextAlign::Right,
        TextAlign::Justify => SkTextAlign::Justify,
    });
    if let Some(lines) = text.max_lines {
        para_style.set_max_lines(lines as usize);
        para_style.set_ellipsis("\u{2026}");
    }
    para_style.set_text_style(&style);

    // Split the text wherever the colour changes: at span edges and at the
    // reveal boundary. Only colour differs between runs, so the layout is the
    // same however much has been revealed.
    let hidden_from = match (&text.reveal, reveal) {
        (Some(r), Some(count)) => Some((text.reveal_offset(r.unit, count), r.hidden)),
        _ => None,
    };
    let mut cuts: Vec<usize> = vec![0, text.value.len()];
    for s in &text.spans {
        cuts.push(s.start);
        cuts.push(s.end);
    }
    if let Some((at, _)) = hidden_from {
        cuts.push(at);
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut builder = ParagraphBuilder::new(&para_style, fonts.clone());
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let run_color = match hidden_from {
            Some((at, hidden)) if from >= at => hidden,
            _ => text
                .spans
                .iter()
                .find(|s| s.start <= from && from < s.end)
                .map_or(color, |s| s.color),
        };
        let [r, g, b, a] = run_color.to_rgba8();
        let mut run_style = style.clone();
        run_style.set_color(skia_safe::Color::from_argb(a, r, g, b));
        builder.push_style(&run_style);
        builder.add_text(&text.value[from..to]);
        builder.pop();
    }
    let mut paragraph = builder.build();

    let hug = |paragraph: &mut Paragraph| {
        let w = paragraph.longest_line().ceil() + 1.0;
        paragraph.layout(w);
        w
    };
    let width = match text.max_width {
        Some(w) => {
            paragraph.layout(w);
            // Like CSS shrink-to-fit: text that fits on its lines without
            // wrapping gets a box as wide as itself; text that had to wrap
            // keeps the full width.
            if text.hug && paragraph.max_intrinsic_width() < w {
                hug(&mut paragraph)
            } else {
                w
            }
        }
        None => {
            // No wrap width: lay out unconstrained, then shrink the box to the
            // widest line so alignment and anchors refer to the text itself.
            paragraph.layout(1.0e7);
            hug(&mut paragraph)
        }
    };
    let height = paragraph.height();
    TextLayout {
        paragraph,
        width,
        height,
    }
}
