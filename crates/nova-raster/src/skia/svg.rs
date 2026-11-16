//! Drawing a parsed SVG document (a `usvg` tree) with Skia.
//!
//! `usvg` does the parsing and resolves everything the SVG and CSS specs
//! leave implicit (inheritance, units, `use`, text to outlines, CSS filter
//! functions). What is left is a small tree of groups, paths and images, and
//! this module paints it with the same rasteriser the rest of NOVA uses.
//!
//! Compared with rasterising through tiny-skia, three things matter for
//! frames with thousands of elements:
//!
//! * opacity on a single shape is folded into its paint (no offscreen layer);
//! * a `clip-path` made of plain shapes becomes a canvas clip (no mask layer);
//! * filters run as Skia image filters, cropped to the filter region.
//!
//! Semantics follow resvg's renderer, which this is modelled on.

use std::collections::HashMap;
use std::sync::Arc;

use nova_core::Limits;
use resvg::usvg::{self, filter as uf, tiny_skia_path};
use skia_safe::canvas::SaveLayerRec;
use skia_safe::gradient::{Colors, Gradient, Interpolation, shaders as gradient_shaders};
use skia_safe::{
    BlendMode, Canvas, Color, Color4f, ColorChannel, FilterMode, Image, ImageFilter, Matrix,
    MipmapMode, Paint, PaintCap, PaintJoin, PaintStyle, Path, PathBuilder, PathEffect,
    PathFillType, PathOp, PictureRecorder, QuickReject, Rect, RoundOut, SamplingOptions, TileMode,
    color_filters,
    image_filters, luma_color_filter, shaders,
};

/// Decoded raster images, keyed by the address of their encoded bytes (the
/// pack's image cache hands out the same `Arc` for the same file).
pub(crate) type ImageMemo = HashMap<usize, Option<Image>>;

pub(crate) struct SvgPainter<'a> {
    pub images: &'a mut ImageMemo,
    /// Shared cache of screen-sized fills that repeat across frames.
    pub overlays: &'a super::frames::OverlayCache,
    /// Size of the frame being drawn, in device pixels.
    pub frame: (i32, i32),
    pub prof: Prof,
}

/// Where the time of one document went (nanoseconds / counts). Printed when
/// `NOVA_SVG_PROFILE` is set; costs a few clock reads per element otherwise.
#[derive(Default, Debug)]
pub(crate) struct Prof {
    pub fills: u64,
    pub fill_ns: u64,
    pub strokes: u64,
    pub stroke_ns: u64,
    pub clips: u64,
    pub clip_geom_ns: u64,
    pub clip_layers: u64,
    pub filter_layers: u64,
    pub filter_restore_ns: u64,
    pub plain_layers: u64,
    pub plain_restore_ns: u64,
    pub mask_layers: u64,
    pub mask_ns: u64,
    pub alpha_paths: u64,
    pub rejected: u64,
    pub small_blurs: u64,
    pub small_blur_ns: u64,
    pub overlay_hits: u64,
}

fn now() -> std::time::Instant {
    std::time::Instant::now()
}

fn matrix(t: usvg::Transform) -> Matrix {
    Matrix::new_all(t.sx, t.kx, t.tx, t.ky, t.sy, t.ty, 0.0, 0.0, 1.0)
}

fn rect_of(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_xywh(x, y, w, h)
}

fn to_path(data: &tiny_skia_path::Path, rule: usvg::FillRule) -> Path {
    use tiny_skia_path::PathSegment as S;
    let mut b = PathBuilder::new();
    for seg in data.segments() {
        match seg {
            S::MoveTo(p) => b.move_to((p.x, p.y)),
            S::LineTo(p) => b.line_to((p.x, p.y)),
            S::QuadTo(c, p) => b.quad_to((c.x, c.y), (p.x, p.y)),
            S::CubicTo(c1, c2, p) => b.cubic_to((c1.x, c1.y), (c2.x, c2.y), (p.x, p.y)),
            S::Close => b.close(),
        };
    }
    b.set_fill_type(match rule {
        usvg::FillRule::NonZero => PathFillType::Winding,
        usvg::FillRule::EvenOdd => PathFillType::EvenOdd,
    });
    b.detach()
}

fn blend_of(mode: usvg::BlendMode) -> BlendMode {
    use usvg::BlendMode as M;
    match mode {
        M::Normal => BlendMode::SrcOver,
        M::Multiply => BlendMode::Multiply,
        M::Screen => BlendMode::Screen,
        M::Overlay => BlendMode::Overlay,
        M::Darken => BlendMode::Darken,
        M::Lighten => BlendMode::Lighten,
        M::ColorDodge => BlendMode::ColorDodge,
        M::ColorBurn => BlendMode::ColorBurn,
        M::HardLight => BlendMode::HardLight,
        M::SoftLight => BlendMode::SoftLight,
        M::Difference => BlendMode::Difference,
        M::Exclusion => BlendMode::Exclusion,
        M::Hue => BlendMode::Hue,
        M::Saturation => BlendMode::Saturation,
        M::Color => BlendMode::Color,
        M::Luminosity => BlendMode::Luminosity,
        M::PlusLighter => BlendMode::Plus,
    }
}

fn color4(c: usvg::Color, alpha: f32) -> Color4f {
    Color4f::new(
        c.red as f32 / 255.0,
        c.green as f32 / 255.0,
        c.blue as f32 / 255.0,
        alpha.clamp(0.0, 1.0),
    )
}

impl SvgPainter<'_> {
    pub fn render(&mut self, tree: &usvg::Tree, canvas: &Canvas) {
        self.group(canvas, tree.root());
    }

    fn nodes(&mut self, canvas: &Canvas, parent: &usvg::Group) {
        for node in parent.children() {
            match node {
                usvg::Node::Group(g) => self.group(canvas, g),
                usvg::Node::Path(p) => self.path(canvas, p, 1.0),
                usvg::Node::Image(img) => self.image(canvas, img),
                usvg::Node::Text(text) => self.group(canvas, text.flattened()),
            }
        }
    }

    fn group(&mut self, canvas: &Canvas, g: &usvg::Group) {
        let outer = canvas.save();
        canvas.concat(&matrix(g.transform()));
        if !g.should_isolate() {
            self.nodes(canvas, g);
            canvas.restore_to_count(outer);
            return;
        }

        let opacity = g.opacity().get();
        let plain = g.filters().is_empty()
            && g.clip_path().is_none()
            && g.mask().is_none()
            && g.blend_mode() == usvg::BlendMode::Normal;

        // `opacity` on one shape that only fills or only strokes: the result
        // of compositing it as a group is the shape drawn with that alpha.
        if plain
            && let [usvg::Node::Path(p)] = g.children()
            && p.fill().is_some() != p.stroke().is_some()
        {
            self.prof.alpha_paths += 1;
            self.path(canvas, p, opacity);
            canvas.restore_to_count(outer);
            return;
        }

        // A clip made of plain shapes is a canvas clip: free, and exact.
        let mut clip_layer = None;
        if let Some(cp) = g.clip_path() {
            let t = now();
            self.prof.clips += 1;
            match clip_geometry(cp) {
                Some(path) => {
                    canvas.clip_path(&path, None, true);
                }
                None => {
                    self.prof.clip_layers += 1;
                    clip_layer = Some(cp)
                }
            }
            self.prof.clip_geom_ns += t.elapsed().as_nanos() as u64;
        }

        // Where the layer's content can be, and where the group can show.
        // A filter region is often far larger than what is drawn into it
        // (and than the screen): the layer only has to hold the content.
        let (bounds, effect) = if g.filters().is_empty() {
            (layer_bounds(g), Effect::None)
        } else {
            let region = g.filters_bounding_box().map(sk_rect);
            let content = content_bounds(g).and_then(|c| match region {
                Some(r) => {
                    let mut c = c;
                    c.intersect(r).then_some(c)
                }
                None => Some(c),
            });
            match (content, region) {
                (Some(content), Some(region)) => match color_only(g, &content, &region) {
                    Some(cf) => (Some(content), Effect::Color(cf)),
                    None => {
                        // What the filter can touch: everything it is allowed
                        // to if it generates pixels, else the content plus
                        // however far blurs and offsets carry it.
                        let visible = match filter_spread(g) {
                            None => region,
                            Some(spread) => {
                                let mut v = content.with_outset((spread, spread));
                                v.intersect(region);
                                v
                            }
                        };
                        if canvas.quick_reject(&visible) {
                            (None, Effect::None)
                        } else if g.mask().is_none()
                            && clip_layer.is_none()
                            && let Some(sigma) = lone_blur(g)
                            && self.small_blur(canvas, g, sigma, &content, &visible, &region)
                        {
                            self.prof.small_blurs += 1;
                            canvas.restore_to_count(outer);
                            return;
                        } else {
                            match self.filters(g) {
                                Some(f) => (Some(content), Effect::Image(f)),
                                None => (Some(content), Effect::None),
                            }
                        }
                    }
                },
                _ => (None, Effect::None),
            }
        };
        // An isolated group without a bounding box draws nothing.
        let rejected = match (&bounds, &effect) {
            (None, _) => true,
            (Some(_), Effect::Image(_)) => false, // already tested against its spread
            (Some(b), _) => canvas.quick_reject(b),
        };
        let Some(bounds) = bounds.filter(|_| !rejected) else {
            self.prof.rejected += 1;
            canvas.restore_to_count(outer);
            return;
        };

        let masked = g.mask().is_some() || clip_layer.is_some();
        let filtered = !matches!(effect, Effect::None);
        let needs_layer = opacity < 1.0
            || g.blend_mode() != usvg::BlendMode::Normal
            || g.isolate()
            || filtered
            || masked;
        let effect_paint = |paint: &mut Paint, effect: Effect| match effect {
            Effect::None => {}
            Effect::Color(cf) => {
                paint.set_color_filter(cf);
            }
            Effect::Image(f) => {
                paint.set_image_filter(f);
            }
        };

        if !masked {
            if needs_layer {
                let mut paint = Paint::default();
                paint.set_alpha_f(opacity);
                paint.set_blend_mode(blend_of(g.blend_mode()));
                effect_paint(&mut paint, effect);
                canvas.save_layer(&SaveLayerRec::default().bounds(&bounds).paint(&paint));
                if filtered && std::env::var_os("NOVA_SVG_PROFILE").is_some() {
                    eprintln!("  layer clip {:?}", canvas.device_clip_bounds());
                }
            }
            let tn = now();
            self.nodes(canvas, g);
            if filtered && std::env::var_os("NOVA_SVG_PROFILE").is_some() {
                eprintln!("  children {:.1} ms", tn.elapsed().as_secs_f64() * 1e3);
            }
            if needs_layer {
                let t = now();
                canvas.restore_to_count(outer);
                let ns = t.elapsed().as_nanos() as u64;
                if filtered {
                    if std::env::var_os("NOVA_SVG_PROFILE").is_some() {
                        let kinds: Vec<String> = g
                            .filters()
                            .iter()
                            .flat_map(|f| f.primitives())
                            .map(|p| match p.kind() {
                                uf::Kind::GaussianBlur(b) => format!("blur σ={}", b.std_dev_x().get()),
                                k => format!("{k:?}").split('(').next().unwrap_or("").to_owned(),
                            })
                            .collect();
                        let m = canvas.local_to_device_as_3x3();
                        eprintln!(
                            "  filter layer {:.1} ms, scale {:.2}, content {:?} {}",
                            ns as f64 / 1e6,
                            m.max_scale(),
                            m.map_rect(bounds).0,
                            kinds.join(",")
                        );
                    }
                    self.prof.filter_layers += 1;
                    self.prof.filter_restore_ns += ns;
                } else {
                    self.prof.plain_layers += 1;
                    self.prof.plain_restore_ns += ns;
                }
                return;
            }
        } else {
            // SVG order: filter, then clip, then mask, then opacity.
            let t = now();
            self.prof.mask_layers += 1;
            let mut paint = Paint::default();
            paint.set_alpha_f(opacity);
            paint.set_blend_mode(blend_of(g.blend_mode()));
            canvas.save_layer(&SaveLayerRec::default().bounds(&bounds).paint(&paint));
            if filtered {
                let mut fp = Paint::default();
                effect_paint(&mut fp, effect);
                let inner = canvas.save_layer(&SaveLayerRec::default().bounds(&bounds).paint(&fp));
                self.nodes(canvas, g);
                canvas.restore_to_count(inner);
            } else {
                self.nodes(canvas, g);
            }
            if let Some(cp) = clip_layer {
                self.clip_layer(canvas, cp, &bounds);
            }
            if let Some(mask) = g.mask() {
                self.mask_layer(canvas, mask, &bounds);
            }
            canvas.restore_to_count(outer);
            self.prof.mask_ns += t.elapsed().as_nanos() as u64;
            return;
        }
        canvas.restore_to_count(outer);
    }

    /// A light Gaussian blur of a whole group, done here instead of by Skia.
    ///
    /// Skia's CPU backend runs blurs with a sigma of a few pixels through an
    /// interpreted shader, at around 100 ns per pixel: a soft edge on a
    /// screen-wide shape costs a quarter of a second. A separable convolution
    /// over the pixels the group actually touches is 10 to 20 times faster.
    /// Done in the encoded (sRGB) values: at this radius the difference from
    /// blurring in linear light is confined to edges a few pixels wide.
    ///
    /// Returns `false` (nothing drawn) when the case is not this one.
    fn small_blur(
        &mut self,
        canvas: &Canvas,
        g: &usvg::Group,
        sigma: (f32, f32),
        content: &Rect,
        visible: &Rect,
        region: &Rect,
    ) -> bool {
        const MAX_SIGMA: f32 = 4.0;
        let ctm = canvas.local_to_device_as_3x3();
        // Axis-aligned transforms only: the kernel is separable in x and y.
        if ctm.skew_x() != 0.0 || ctm.skew_y() != 0.0 || ctm.has_perspective() {
            return false;
        }
        let (sx, sy) = (sigma.0 * ctm.scale_x().abs(), sigma.1 * ctm.scale_y().abs());
        if sx > MAX_SIGMA || sy > MAX_SIGMA || (sx < 0.05 && sy < 0.05) {
            return false;
        }
        let t = now();
        let pad = (3.0 * sx.max(sy)).ceil() as i32 + 1;
        let clip = canvas.device_clip_bounds().unwrap_or_default();
        use skia_safe::IRect;
        let want: IRect = ctm.map_rect(visible).0.round_out();
        let Some(want) = IRect::intersect(&want, &clip) else {
            return true; // nothing of it can be seen
        };
        // Pixels that can reach the visible part, limited to where content is.
        let reach: IRect = ctm.map_rect(content).0.round_out();
        let Some(area) =
            IRect::intersect(&want.with_outset((pad, pad)), &reach.with_outset((pad, pad)))
        else {
            return true;
        };
        let (w, h) = (area.width(), area.height());
        let info = skia_safe::ImageInfo::new(
            (w, h),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        let Some(mut surface) = skia_safe::surfaces::raster(&info, None, None) else {
            return false;
        };
        {
            let sc = surface.canvas();
            sc.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
            sc.translate((-area.left as f32, -area.top as f32));
            sc.concat(&ctm);
            self.nodes(sc, g);
        }
        // Blur the surface's own pixels, then hand them over as an image.
        {
            let Some(mut pixmap) = surface.peek_pixels() else { return false };
            let row_bytes = pixmap.row_bytes();
            let Some(bytes) = pixmap.bytes_mut() else { return false };
            if row_bytes != w as usize * 4 {
                return false;
            }
            gaussian_blur(bytes, w as usize, h as usize, sx, sy);
        }
        let image = surface.image_snapshot();
        let mut paint = Paint::default();
        paint.set_alpha_f(g.opacity().get());
        paint.set_blend_mode(blend_of(g.blend_mode()));
        canvas.save();
        canvas.clip_rect(region, None, true);
        canvas.reset_matrix();
        canvas.draw_image(&image, (area.left as f32, area.top as f32), Some(&paint));
        canvas.restore();
        self.prof.small_blur_ns += t.elapsed().as_nanos() as u64;
        true
    }

    /// Keep only the part of the current layer that the mask lets through.
    fn mask_layer(&mut self, canvas: &Canvas, mask: &usvg::Mask, bounds: &Rect) {
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::DstIn);
        if mask.kind() == usvg::MaskType::Luminance {
            paint.set_color_filter(luma_color_filter::new());
        }
        let layer = canvas.save_layer(&SaveLayerRec::default().bounds(bounds).paint(&paint));
        let r = mask.rect();
        canvas.save();
        canvas.clip_rect(rect_of(r.x(), r.y(), r.width(), r.height()), None, true);
        self.nodes(canvas, mask.root());
        canvas.restore();
        if let Some(inner) = mask.mask() {
            self.mask_layer(canvas, inner, bounds);
        }
        canvas.restore_to_count(layer);
    }

    /// Clip the current layer by a `clipPath` that could not be reduced to
    /// one outline: draw its shapes into a layer and keep the overlap.
    fn clip_layer(&mut self, canvas: &Canvas, cp: &usvg::ClipPath, bounds: &Rect) {
        let mut paint = Paint::default();
        paint.set_blend_mode(BlendMode::DstIn);
        let layer = canvas.save_layer(&SaveLayerRec::default().bounds(bounds).paint(&paint));
        canvas.save();
        canvas.concat(&matrix(cp.transform()));
        self.clip_shapes(canvas, cp.root());
        canvas.restore();
        canvas.restore_to_count(layer);
        if let Some(inner) = cp.clip_path() {
            self.clip_layer(canvas, inner, bounds);
        }
    }

    fn clip_shapes(&mut self, canvas: &Canvas, parent: &usvg::Group) {
        let mut black = Paint::default();
        black.set_anti_alias(true);
        for child in parent.children() {
            match child {
                usvg::Node::Path(p) => {
                    if let (true, Some(fill)) = (p.is_visible(), p.fill()) {
                        canvas.draw_path(&to_path(p.data(), fill.rule()), &black);
                    }
                }
                usvg::Node::Text(text) => self.clip_shapes(canvas, text.flattened()),
                usvg::Node::Group(sub) => {
                    canvas.save();
                    canvas.concat(&matrix(sub.transform()));
                    if let Some(cp) = sub.clip_path() {
                        if let Some(bounds) = layer_bounds(sub) {
                            let layer =
                                canvas.save_layer(&SaveLayerRec::default().bounds(&bounds));
                            self.clip_shapes(canvas, sub);
                            self.clip_layer(canvas, cp, &bounds);
                            canvas.restore_to_count(layer);
                        }
                    } else {
                        self.clip_shapes(canvas, sub);
                    }
                    canvas.restore();
                }
                usvg::Node::Image(_) => {}
            }
        }
    }

    // ---------------------------------------------------------------- shapes

    fn path(&mut self, canvas: &Canvas, p: &usvg::Path, alpha: f32) {
        if !p.is_visible() {
            return;
        }
        let aa = p.rendering_mode().use_shape_antialiasing();
        let fill_first = p.paint_order() == usvg::PaintOrder::FillAndStroke;
        if fill_first {
            self.fill(canvas, p, alpha, aa);
        }
        if let Some(stroke) = p.stroke()
            && let Some(mut paint) = self.paint(stroke.paint(), stroke.opacity().get() * alpha)
        {
            paint.set_anti_alias(aa);
            paint.set_style(PaintStyle::Stroke);
            paint.set_stroke_width(stroke.width().get());
            paint.set_stroke_miter(stroke.miterlimit().get());
            paint.set_stroke_cap(match stroke.linecap() {
                usvg::LineCap::Butt => PaintCap::Butt,
                usvg::LineCap::Round => PaintCap::Round,
                usvg::LineCap::Square => PaintCap::Square,
            });
            paint.set_stroke_join(match stroke.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => PaintJoin::Miter,
                usvg::LineJoin::Round => PaintJoin::Round,
                usvg::LineJoin::Bevel => PaintJoin::Bevel,
            });
            if let Some(dashes) = stroke.dasharray()
                && dashes.len() >= 2
                && dashes.len() % 2 == 0
            {
                paint.set_path_effect(PathEffect::dash(dashes, stroke.dashoffset()));
            }
            let t = now();
            canvas.draw_path(&to_path(p.data(), usvg::FillRule::NonZero), &paint);
            self.prof.strokes += 1;
            let ns = t.elapsed().as_nanos() as u64;
            self.prof.stroke_ns += ns;
            if ns > 1_500_000 && std::env::var_os("NOVA_SVG_PROFILE").is_some() {
                eprintln!(
                    "  slow stroke {:.1} ms w {} segs {} dash {}",
                    ns as f64 / 1e6,
                    stroke.width().get(),
                    p.data().len(),
                    stroke.dasharray().is_some()
                );
            }
        }
        if !fill_first {
            self.fill(canvas, p, alpha, aa);
        }
    }

    fn fill(&mut self, canvas: &Canvas, p: &usvg::Path, alpha: f32, aa: bool) {
        let Some(fill) = p.fill() else { return };
        // Horizontal and vertical lines have no interior.
        let b = p.data().bounds();
        if b.width() == 0.0 || b.height() == 0.0 {
            return;
        }
        if let Some(mut paint) = self.paint(fill.paint(), fill.opacity().get() * alpha) {
            paint.set_anti_alias(aa);
            let t = now();
            let sk_path = to_path(p.data(), fill.rule());
            if !matches!(fill.paint(), usvg::Paint::Color(_))
                && self.overlay_fill(canvas, &sk_path, &paint, fill, alpha)
            {
                self.prof.overlay_hits += 1;
                self.prof.fills += 1;
                self.prof.fill_ns += t.elapsed().as_nanos() as u64;
                return;
            }
            canvas.draw_path(&sk_path, &paint);
            self.prof.fills += 1;
            let ns = t.elapsed().as_nanos() as u64;
            self.prof.fill_ns += ns;
            if ns > 1_500_000 && std::env::var_os("NOVA_SVG_PROFILE").is_some() {
                let kind = match fill.paint() {
                    usvg::Paint::Color(_) => "color",
                    usvg::Paint::LinearGradient(_) => "linear",
                    usvg::Paint::RadialGradient(_) => "radial",
                    usvg::Paint::Pattern(_) => "pattern",
                };
                eprintln!(
                    "  slow fill {:.1} ms {kind} segs {} dev {:?} clip {:?}",
                    ns as f64 / 1e6,
                    p.data().len(),
                    canvas.local_to_device_as_3x3().map_rect(rect_of(b.x(), b.y(), b.width(), b.height())).0,
                    canvas.device_clip_bounds()
                );
            }
        }
    }

    /// A gradient that fills a screen-sized rectangle costs 15 to 25 ms, and
    /// overlays like a vignette are the same on every frame of a scene.
    /// The second time the identical fill comes up it is rendered into a
    /// picture; from then on every frame composites that picture.
    /// Returns `false` when the fill should just be drawn.
    fn overlay_fill(
        &mut self,
        canvas: &Canvas,
        path: &Path,
        paint: &Paint,
        fill: &usvg::Fill,
        alpha: f32,
    ) -> bool {
        use skia_safe::IRect;
        let Some((rect, _, _)) = path.is_rect() else { return false };
        let ctm = canvas.local_to_device_as_3x3();
        if ctm.skew_x() != 0.0 || ctm.skew_y() != 0.0 || ctm.has_perspective() {
            return false;
        }
        let dev: IRect = ctm.map_rect(rect).0.round_out();
        // The visible part, in device pixels. The device is the surface being drawn to,
        // which is larger than the pack when the frame is drawn at a scale.
        let Some(area) = IRect::intersect(&dev, &IRect::from_size(canvas.base_layer_size())) else {
            return false;
        };
        if (area.width() as i64) * (area.height() as i64) < 400_000 {
            return false;
        }
        // Everything the pixels depend on.
        let mut key = b"fill:".to_vec();
        let mut num = |v: f32| key.extend_from_slice(&v.to_bits().to_le_bytes());
        for i in 0..6 {
            num(ctm[i]);
        }
        for v in [rect.left, rect.top, rect.right, rect.bottom, fill.opacity().get() * alpha] {
            num(v);
        }
        let base = |g: &usvg::BaseGradient, num: &mut dyn FnMut(f32)| {
            let t = g.transform();
            for v in [t.sx, t.kx, t.ky, t.sy, t.tx, t.ty] {
                num(v);
            }
            num(g.spread_method() as u8 as f32);
            for s in g.stops() {
                let c = s.color();
                num(s.offset().get());
                num(u32::from_be_bytes([c.red, c.green, c.blue, 0]) as f32);
                num(s.opacity().get());
            }
        };
        match fill.paint() {
            usvg::Paint::LinearGradient(g) => {
                for v in [0.0, g.x1(), g.y1(), g.x2(), g.y2()] {
                    num(v);
                }
                base(g, &mut num);
            }
            usvg::Paint::RadialGradient(g) => {
                for v in [1.0, g.cx(), g.cy(), g.r().get(), g.fx(), g.fy(), g.fr().get()] {
                    num(v);
                }
                base(g, &mut num);
            }
            _ => return false,
        }
        let image = match self.overlays.get(&key) {
            Some(image) => image,
            None => {
                if !self.overlays.seen_before(&key) {
                    return false;
                }
                let info = skia_safe::ImageInfo::new(
                    (area.width(), area.height()),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Premul,
                    None,
                );
                let Some(mut surface) = skia_safe::surfaces::raster(&info, None, None) else {
                    return false;
                };
                let sc = surface.canvas();
                sc.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
                sc.translate((-area.left as f32, -area.top as f32));
                sc.concat(&ctm);
                sc.draw_path(path, paint);
                let image = surface.image_snapshot();
                self.overlays.put(key, image.clone());
                image
            }
        };
        canvas.save();
        canvas.reset_matrix();
        canvas.draw_image(&image, (area.left as f32, area.top as f32), None);
        canvas.restore();
        true
    }

    fn paint(&mut self, server: &usvg::Paint, opacity: f32) -> Option<Paint> {
        if opacity <= 0.0 {
            return None;
        }
        let mut paint = Paint::default();
        match server {
            usvg::Paint::Color(c) => {
                paint.set_color4f(color4(*c, opacity), None);
            }
            usvg::Paint::LinearGradient(lg) => {
                let (colors, offsets, tile) = stops(lg);
                let gradient = Gradient::new(
                    Colors::new(&colors, Some(&offsets), tile, None),
                    Interpolation::default(),
                );
                let m = matrix(lg.transform());
                match gradient_shaders::linear_gradient(
                    ((lg.x1(), lg.y1()), (lg.x2(), lg.y2())),
                    &gradient,
                    Some(&m),
                ) {
                    Some(shader) => {
                        paint.set_shader(shader);
                        paint.set_alpha_f(opacity);
                        // No dither: Chrome does not dither these either, and it costs a third
                        // of a full-screen gradient fill.
                        paint.set_dither(false);
                    }
                    // A zero-length gradient paints its last stop.
                    None => {
                        let last = *colors.last()?;
                        paint.set_color4f(Color4f::new(last.r, last.g, last.b, last.a * opacity), None);
                    }
                }
            }
            usvg::Paint::RadialGradient(rg) => {
                let (colors, offsets, tile) = stops(rg);
                let gradient = Gradient::new(
                    Colors::new(&colors, Some(&offsets), tile, None),
                    Interpolation::default(),
                );
                let m = matrix(rg.transform());
                let focal = rg.fx() != rg.cx() || rg.fy() != rg.cy() || rg.fr().get() > 0.0;
                let shader = if focal {
                    gradient_shaders::two_point_conical_gradient(
                        ((rg.fx(), rg.fy()), rg.fr().get()),
                        ((rg.cx(), rg.cy()), rg.r().get()),
                        &gradient,
                        Some(&m),
                    )
                } else {
                    gradient_shaders::radial_gradient(
                        ((rg.cx(), rg.cy()), rg.r().get()),
                        &gradient,
                        Some(&m),
                    )
                };
                match shader {
                    Some(shader) => {
                        paint.set_shader(shader);
                        paint.set_alpha_f(opacity);
                        // No dither: Chrome does not dither these either, and it costs a third
                        // of a full-screen gradient fill.
                        paint.set_dither(false);
                    }
                    None => {
                        let last = *colors.last()?;
                        paint.set_color4f(Color4f::new(last.r, last.g, last.b, last.a * opacity), None);
                    }
                }
            }
            usvg::Paint::Pattern(pattern) => {
                let r = pattern.rect();
                let tile = Rect::from_wh(r.width(), r.height());
                let mut recorder = PictureRecorder::new();
                let pc = recorder.begin_recording(tile, false);
                self.nodes(pc, pattern.root());
                let picture = recorder.finish_recording_as_picture(None)?;
                let local = Matrix::concat(
                    &matrix(pattern.transform()),
                    &Matrix::translate((r.x(), r.y())),
                );
                paint.set_shader(picture.to_shader(
                    (TileMode::Repeat, TileMode::Repeat),
                    FilterMode::Linear,
                    Some(&local),
                    Some(&tile),
                ));
                paint.set_alpha_f(opacity);
            }
        }
        Some(paint)
    }

    fn image(&mut self, canvas: &Canvas, img: &usvg::Image) {
        if !img.is_visible() {
            return;
        }
        let bytes: &Arc<Vec<u8>> = match img.kind() {
            usvg::ImageKind::SVG(tree) => {
                self.group(canvas, tree.root());
                return;
            }
            usvg::ImageKind::PNG(b)
            | usvg::ImageKind::JPEG(b)
            | usvg::ImageKind::WEBP(b)
            | usvg::ImageKind::GIF(b) => b,
        };
        let key = Arc::as_ptr(bytes) as usize;
        let decoded = self.images.entry(key).or_insert_with(|| {
            let d = super::assets::decode_image(bytes, &Limits::default()).ok()?;
            super::assets::image_from_premul(&d.pixels, d.width, d.height, true).ok()
        });
        let Some(image) = decoded else { return };
        let sampling = match img.rendering_mode() {
            usvg::ImageRendering::OptimizeSpeed
            | usvg::ImageRendering::CrispEdges
            | usvg::ImageRendering::Pixelated => {
                SamplingOptions::new(FilterMode::Nearest, MipmapMode::None)
            }
            _ => SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear),
        };
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        canvas.draw_image_with_sampling_options(&*image, (0.0, 0.0), sampling, Some(&paint));
    }

    // ---------------------------------------------------------------- filters

    /// All filters of a group as one Skia image filter (applied in order).
    fn filters(&mut self, g: &usvg::Group) -> Option<ImageFilter> {
        // Blurring pixels of one colour gives the same result in any colour
        // space, so the conversion to linear light and back can be skipped.
        let flat = flat_color(g, &mut None);
        let mut source: Option<ImageFilter> = None;
        for f in g.filters() {
            source = self.filter(f, source, flat);
        }
        source
    }

    /// One `<filter>`: `source` is its SourceGraphic (`None` = the layer).
    fn filter(
        &mut self,
        f: &uf::Filter,
        source: Option<ImageFilter>,
        flat: bool,
    ) -> Option<ImageFilter> {
        let fr = f.rect();
        let region = rect_of(fr.x(), fr.y(), fr.width(), fr.height());
        let mut results: Vec<(&str, Option<ImageFilter>)> = Vec::new();
        let mut last = source.clone();
        for prim in f.primitives() {
            let pr = prim.rect();
            let mut crop = rect_of(pr.x(), pr.y(), pr.width(), pr.height());
            if !crop.intersect(region) {
                crop = Rect::new_empty();
            }
            let crop = image_filters::CropRect::from(crop);
            let spatial_only = matches!(
                prim.kind(),
                uf::Kind::GaussianBlur(_) | uf::Kind::Offset(_) | uf::Kind::Merge(_)
            );
            let linear = prim.color_interpolation() == uf::ColorInterpolation::LinearRGB
                && !(flat && spatial_only);
            let input = |i: &uf::Input| -> Option<ImageFilter> {
                match i {
                    uf::Input::SourceGraphic => source.clone(),
                    uf::Input::SourceAlpha => image_filters::color_filter(
                        color_filters::matrix_row_major(&ALPHA_ONLY, None),
                        source.clone(),
                        None,
                    ),
                    uf::Input::Reference(name) => results
                        .iter()
                        .rev()
                        .find(|(n, _)| n == name)
                        .map(|(_, r)| r.clone())
                        .unwrap_or_else(|| last.clone()),
                }
            };
            // Primitives whose arithmetic depends on the colour space take
            // their input converted to linear light and convert the result back.
            let to_linear = |i: Option<ImageFilter>| -> Option<ImageFilter> {
                if linear {
                    image_filters::color_filter(color_filters::srgb_to_linear_gamma(), i, None)
                } else {
                    i
                }
            };
            let to_srgb = |i: Option<ImageFilter>| -> Option<ImageFilter> {
                if linear {
                    image_filters::color_filter(color_filters::linear_to_srgb_gamma(), i, None)
                } else {
                    i
                }
            };
            let out: Option<ImageFilter> = match prim.kind() {
                uf::Kind::GaussianBlur(fe) => {
                    let (sx, sy) = (fe.std_dev_x().get(), fe.std_dev_y().get());
                    if sx <= 0.0 && sy <= 0.0 {
                        image_filters::crop(crop.rect().unwrap_or(region), None, input(fe.input()))
                    } else {
                        to_srgb(image_filters::blur(
                            (sx, sy),
                            TileMode::Decal,
                            to_linear(input(fe.input())),
                            crop,
                        ))
                    }
                }
                uf::Kind::Offset(fe) => {
                    image_filters::offset((fe.dx(), fe.dy()), input(fe.input()), crop)
                }
                uf::Kind::ColorMatrix(fe) => to_srgb(image_filters::color_filter(
                    color_filters::matrix_row_major(&color_matrix(fe.kind()), None),
                    to_linear(input(fe.input())),
                    crop,
                )),
                uf::Kind::ComponentTransfer(fe) => match transfer_filter(fe) {
                    Some(cf) => to_srgb(image_filters::color_filter(
                        cf,
                        to_linear(input(fe.input())),
                        crop,
                    )),
                    None => input(fe.input()),
                },
                uf::Kind::Composite(fe) => {
                    let (fg, bg) = (to_linear(input(fe.input1())), to_linear(input(fe.input2())));
                    use uf::CompositeOperator as Op;
                    to_srgb(match fe.operator() {
                        Op::Arithmetic { k1, k2, k3, k4 } => {
                            image_filters::arithmetic(k1, k2, k3, k4, true, bg, fg, crop)
                        }
                        op => {
                            let mode = match op {
                                Op::In => BlendMode::SrcIn,
                                Op::Out => BlendMode::SrcOut,
                                Op::Atop => BlendMode::SrcATop,
                                Op::Xor => BlendMode::Xor,
                                _ => BlendMode::SrcOver,
                            };
                            image_filters::blend(mode, bg, fg, crop)
                        }
                    })
                }
                uf::Kind::Blend(fe) => to_srgb(image_filters::blend(
                    blend_of(fe.mode()),
                    to_linear(input(fe.input2())),
                    to_linear(input(fe.input1())),
                    crop,
                )),
                uf::Kind::Flood(fe) => {
                    let c = fe.color();
                    let a = (fe.opacity().get().clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    image_filters::shader(
                        shaders::color(Color::from_argb(a, c.red, c.green, c.blue)),
                        crop,
                    )
                }
                uf::Kind::Merge(fe) => {
                    image_filters::merge(fe.inputs().iter().map(&input), crop)
                }
                uf::Kind::DropShadow(fe) => image_filters::drop_shadow(
                    (fe.dx(), fe.dy()),
                    (fe.std_dev_x().get(), fe.std_dev_y().get()),
                    color4(fe.color(), fe.opacity().get()),
                    None,
                    input(fe.input()),
                    crop,
                ),
                uf::Kind::Morphology(fe) => {
                    let r = (fe.radius_x().get(), fe.radius_y().get());
                    match fe.operator() {
                        uf::MorphologyOperator::Dilate => {
                            image_filters::dilate(r, input(fe.input()), crop)
                        }
                        uf::MorphologyOperator::Erode => {
                            image_filters::erode(r, input(fe.input()), crop)
                        }
                    }
                }
                uf::Kind::Turbulence(fe) => {
                    let freq = (fe.base_frequency_x().get(), fe.base_frequency_y().get());
                    let octaves = fe.num_octaves() as usize;
                    let shader = match fe.kind() {
                        uf::TurbulenceKind::FractalNoise => {
                            shaders::fractal_noise(freq, octaves, fe.seed() as f32, None)
                        }
                        uf::TurbulenceKind::Turbulence => {
                            shaders::turbulence(freq, octaves, fe.seed() as f32, None)
                        }
                    };
                    shader.and_then(|s| to_srgb(image_filters::shader(s, crop)))
                }
                uf::Kind::DisplacementMap(fe) => {
                    let ch = |c: uf::ColorChannel| match c {
                        uf::ColorChannel::R => ColorChannel::R,
                        uf::ColorChannel::G => ColorChannel::G,
                        uf::ColorChannel::B => ColorChannel::B,
                        uf::ColorChannel::A => ColorChannel::A,
                    };
                    image_filters::displacement_map(
                        (ch(fe.x_channel_selector()), ch(fe.y_channel_selector())),
                        fe.scale(),
                        to_linear(input(fe.input2())),
                        input(fe.input1()),
                        crop,
                    )
                }
                uf::Kind::Image(fe) => {
                    let mut recorder = PictureRecorder::new();
                    let pc = recorder.begin_recording(region, false);
                    self.nodes(pc, fe.root());
                    recorder
                        .finish_recording_as_picture(None)
                        .and_then(|p| image_filters::picture(p, Some(&region)))
                }
                // Not used by any content NOVA bakes today: pass the input on.
                uf::Kind::Tile(fe) => input(fe.input()),
                uf::Kind::ConvolveMatrix(fe) => input(fe.input()),
                uf::Kind::DiffuseLighting(fe) => input(fe.input()),
                uf::Kind::SpecularLighting(fe) => input(fe.input()),
            };
            results.push((prim.result(), out.clone()));
            last = out;
        }
        // Nothing may show outside the filter region.
        image_filters::crop(region, None, last)
    }
}

/// What a group's `filter` turns into.
enum Effect {
    None,
    /// The whole chain only recolours pixels: one colour filter on the layer.
    Color(skia_safe::ColorFilter),
    Image(ImageFilter),
}

fn sk_rect(r: usvg::NonZeroRect) -> Rect {
    rect_of(r.x(), r.y(), r.width(), r.height())
}

/// Bounds of everything the group's children draw, in the group's space,
/// including what the children's own filters add.
fn content_bounds(g: &usvg::Group) -> Option<Rect> {
    let mut acc: Option<Rect> = None;
    let mut add = |r: Rect| {
        acc = Some(match acc {
            None => r,
            Some(mut a) => {
                a.join(r);
                a
            }
        });
    };
    for child in g.children() {
        match child {
            usvg::Node::Group(c) => {
                if let Some(b) = c.layer_bounding_box() {
                    add(matrix(c.transform()).map_rect(sk_rect(b)).0);
                }
            }
            usvg::Node::Path(p) => {
                if let Some(b) = p.stroke_bounding_box() {
                    add(rect_of(b.x(), b.y(), b.width(), b.height()));
                }
            }
            usvg::Node::Image(i) => {
                let b = i.bounding_box();
                add(rect_of(b.x(), b.y(), b.width(), b.height()));
            }
            usvg::Node::Text(t) => {
                let f = t.flattened();
                if let Some(b) = f.layer_bounding_box() {
                    add(matrix(f.transform()).map_rect(sk_rect(b)).0);
                }
            }
        }
    }
    acc.map(|r| r.with_outset((2.0, 2.0)))
}

/// When every primitive of every filter only maps colours, each fed by the
/// one before it, and the filter region covers the content, the whole chain
/// is one colour filter. That is what CSS `brightness() saturate() sepia()`
/// compiles to, and it is by far the cheapest way to run it.
fn color_only(g: &usvg::Group, content: &Rect, region: &Rect) -> Option<skia_safe::ColorFilter> {
    let covers = region.left <= content.left
        && region.top <= content.top
        && region.right >= content.right
        && region.bottom >= content.bottom;
    if !covers {
        return None;
    }
    let mut total: Option<skia_safe::ColorFilter> = None;
    for f in g.filters() {
        let mut prev: Option<&str> = None;
        for prim in f.primitives() {
            let (input, cf) = match prim.kind() {
                uf::Kind::ColorMatrix(fe) => (
                    fe.input(),
                    color_filters::matrix_row_major(&color_matrix(fe.kind()), None),
                ),
                uf::Kind::ComponentTransfer(fe) => (fe.input(), transfer_filter(fe)?),
                _ => return None,
            };
            let chained = match (input, prev) {
                (uf::Input::SourceGraphic, None) => true,
                (uf::Input::Reference(name), Some(p)) => name == p,
                _ => false,
            };
            if !chained {
                return None;
            }
            let cf = if prim.color_interpolation() == uf::ColorInterpolation::LinearRGB {
                color_filters::linear_to_srgb_gamma()
                    .composed(cf.composed(color_filters::srgb_to_linear_gamma())?)?
            } else {
                cf
            };
            total = Some(match total {
                None => cf,
                Some(inner) => cf.composed(inner)?,
            });
            prev = Some(prim.result());
        }
    }
    total
}

/// The sigma of a group whose whole filter is one Gaussian blur of itself.
fn lone_blur(g: &usvg::Group) -> Option<(f32, f32)> {
    let [f] = g.filters() else { return None };
    let [prim] = f.primitives() else { return None };
    match prim.kind() {
        uf::Kind::GaussianBlur(fe) if matches!(fe.input(), uf::Input::SourceGraphic) => {
            Some((fe.std_dev_x().get(), fe.std_dev_y().get()))
        }
        _ => None,
    }
}

/// True when everything the group draws has one colour (alpha may vary).
fn flat_color(g: &usvg::Group, seen: &mut Option<(u8, u8, u8)>) -> bool {
    fn same(paint: &usvg::Paint, seen: &mut Option<(u8, u8, u8)>) -> bool {
        match paint {
            usvg::Paint::Color(c) => {
                let rgb = (c.red, c.green, c.blue);
                *seen.get_or_insert(rgb) == rgb
            }
            _ => false,
        }
    }
    for child in g.children() {
        let ok = match child {
            usvg::Node::Group(c) => {
                c.filters().is_empty()
                    && c.blend_mode() == usvg::BlendMode::Normal
                    && flat_color(c, seen)
            }
            usvg::Node::Path(p) => {
                p.fill().is_none_or(|f| same(f.paint(), seen))
                    && p.stroke().is_none_or(|s| same(s.paint(), seen))
            }
            usvg::Node::Text(t) => flat_color(t.flattened(), seen),
            usvg::Node::Image(_) => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Separable Gaussian blur of premultiplied RGBA8 pixels, in place. Pixels
/// outside the buffer count as transparent.
pub(crate) fn gaussian_blur(pixels: &mut [u8], w: usize, h: usize, sx: f32, sy: f32) {
    // 8-bit weights that sum to exactly 256: a u16 accumulator never overflows.
    fn kernel(sigma: f32) -> Vec<u16> {
        if sigma < 0.05 {
            return vec![256];
        }
        let r = (3.0 * sigma).ceil() as i32;
        let raw: Vec<f32> = (-r..=r)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum: f32 = raw.iter().sum();
        let mut k: Vec<u16> = raw.iter().map(|v| (v / sum * 256.0).round() as u16).collect();
        let total: i32 = k.iter().map(|v| *v as i32).sum();
        let mid = k.len() / 2;
        k[mid] = (k[mid] as i32 + 256 - total) as u16;
        k
    }
    let row = w * 4;
    let mut acc = vec![0u16; row];
    // horizontal
    let kx = kernel(sx);
    if kx.len() > 1 {
        let r = kx.len() / 2;
        let mut src = vec![0u8; row + 2 * r * 4];
        for y in 0..h {
            let line = &mut pixels[y * row..(y + 1) * row];
            src[r * 4..r * 4 + row].copy_from_slice(line);
            acc.fill(128);
            for (i, &wgt) in kx.iter().enumerate() {
                let tap = &src[i * 4..i * 4 + row];
                for (a, &p) in acc.iter_mut().zip(tap) {
                    *a += p as u16 * wgt;
                }
            }
            for (d, &a) in line.iter_mut().zip(&acc) {
                *d = (a >> 8) as u8;
            }
        }
    }
    // vertical
    let ky = kernel(sy);
    if ky.len() > 1 {
        let r = ky.len() / 2;
        // ring of the last `2r + 1` source rows, so the pass can run in place
        let mut ring: Vec<Vec<u8>> = (0..ky.len()).map(|_| vec![0u8; row]).collect();
        let mut filled = 0usize; // rows of the image pushed so far
        let push = |ring: &mut Vec<Vec<u8>>, filled: &mut usize, data: Option<&[u8]>| {
            let slot = *filled % ring.len();
            match data {
                Some(d) => ring[slot].copy_from_slice(d),
                None => ring[slot].fill(0),
            }
            *filled += 1;
        };
        // window for output row y covers source rows y-r ..= y+r
        for _ in 0..r {
            push(&mut ring, &mut filled, None);
        }
        for y in 0..r.min(h) {
            push(&mut ring, &mut filled, Some(&pixels[y * row..(y + 1) * row]));
        }
        for _ in r.min(h)..r {
            push(&mut ring, &mut filled, None);
        }
        for y in 0..h {
            let incoming = y + r;
            if incoming < h {
                let (done, rest) = pixels.split_at(incoming * row);
                let _ = done;
                push(&mut ring, &mut filled, Some(&rest[..row]));
            } else {
                push(&mut ring, &mut filled, None);
            }
            // oldest row in the ring is source row y - r
            acc.fill(128);
            for (i, &wgt) in ky.iter().enumerate() {
                let tap = &ring[(filled + i) % ring.len()];
                for (a, &p) in acc.iter_mut().zip(tap.iter()) {
                    *a += p as u16 * wgt;
                }
            }
            for (d, &a) in pixels[y * row..(y + 1) * row].iter_mut().zip(&acc) {
                *d = (a >> 8) as u8;
            }
        }
    }
}

/// How far (in the group's units) the filters can carry pixels beyond the
/// content. `None` when a primitive produces pixels on its own, so the
/// whole filter region can show.
fn filter_spread(g: &usvg::Group) -> Option<f32> {
    let mut spread = 0.0f32;
    for f in g.filters() {
        for prim in f.primitives() {
            spread += match prim.kind() {
                uf::Kind::GaussianBlur(fe) => 3.0 * fe.std_dev_x().get().max(fe.std_dev_y().get()),
                uf::Kind::DropShadow(fe) => {
                    3.0 * fe.std_dev_x().get().max(fe.std_dev_y().get())
                        + fe.dx().abs().max(fe.dy().abs())
                }
                uf::Kind::Offset(fe) => fe.dx().abs().max(fe.dy().abs()),
                uf::Kind::Morphology(fe) => fe.radius_x().get().max(fe.radius_y().get()),
                uf::Kind::DisplacementMap(fe) => fe.scale().abs(),
                uf::Kind::ColorMatrix(fe) => match fe.kind() {
                    uf::ColorMatrixKind::Matrix(m) if m[19] != 0.0 => return None,
                    _ => 0.0,
                },
                uf::Kind::ComponentTransfer(fe) => {
                    if matches!(fe.func_a(), uf::TransferFunction::Identity) {
                        0.0
                    } else {
                        return None;
                    }
                }
                uf::Kind::Composite(_) | uf::Kind::Blend(_) | uf::Kind::Merge(_) => 0.0,
                uf::Kind::ConvolveMatrix(_)
                | uf::Kind::DiffuseLighting(_)
                | uf::Kind::SpecularLighting(_) => 0.0,
                uf::Kind::Flood(_)
                | uf::Kind::Turbulence(_)
                | uf::Kind::Image(_)
                | uf::Kind::Tile(_) => return None,
            };
        }
    }
    Some(spread + 2.0)
}

const ALPHA_ONLY: [f32; 20] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
];

fn color_matrix(kind: &uf::ColorMatrixKind) -> [f32; 20] {
    match kind {
        uf::ColorMatrixKind::Matrix(m) => {
            let mut out = [0.0; 20];
            out.copy_from_slice(&m[..20]);
            out
        }
        uf::ColorMatrixKind::Saturate(v) => {
            let s = v.get();
            [
                0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s, 0.0, 0.0,
                0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s, 0.0, 0.0,
                0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s, 0.0, 0.0,
                0.0, 0.0, 0.0, 1.0, 0.0,
            ]
        }
        uf::ColorMatrixKind::HueRotate(deg) => {
            let (s, c) = deg.to_radians().sin_cos();
            [
                0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928, 0.0, 0.0,
                0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283, 0.0, 0.0,
                0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072, 0.0, 0.0,
                0.0, 0.0, 0.0, 1.0, 0.0,
            ]
        }
        uf::ColorMatrixKind::LuminanceToAlpha => [
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.2125, 0.7154, 0.0721, 0.0, 0.0,
        ],
    }
}

/// `feComponentTransfer` whose colour channels are all `linear` (what CSS
/// `brightness()`, `contrast()`, `invert()` and `opacity()` compile to) is a
/// colour matrix: cheaper than three table lookups per pixel.
fn transfer_matrix(fe: &uf::ComponentTransfer) -> Option<[f32; 20]> {
    use uf::TransferFunction as T;
    let lin = |f: &T| match f {
        T::Identity => Some((1.0, 0.0)),
        T::Linear { slope, intercept } => Some((*slope, *intercept)),
        _ => None,
    };
    let (r, g, b, a) = (
        lin(fe.func_r())?,
        lin(fe.func_g())?,
        lin(fe.func_b())?,
        lin(fe.func_a())?,
    );
    Some([
        r.0, 0.0, 0.0, 0.0, r.1, 0.0, g.0, 0.0, 0.0, g.1, 0.0, 0.0, b.0, 0.0, b.1, 0.0, 0.0, 0.0,
        a.0, a.1,
    ])
}

/// The colour filter of a component transfer; `None` when it is the identity.
fn transfer_filter(fe: &uf::ComponentTransfer) -> Option<skia_safe::ColorFilter> {
    if let Some(m) = transfer_matrix(fe) {
        return Some(color_filters::matrix_row_major(&m, None));
    }
    let (a, r, g, b) = (
        transfer_table(fe.func_a()),
        transfer_table(fe.func_r()),
        transfer_table(fe.func_g()),
        transfer_table(fe.func_b()),
    );
    color_filters::table_argb(a.as_ref(), r.as_ref(), g.as_ref(), b.as_ref())
}

/// A transfer function as a 256-entry table; `None` for the identity.
fn transfer_table(func: &uf::TransferFunction) -> Option<[u8; 256]> {
    use uf::TransferFunction as T;
    if matches!(func, T::Identity) {
        return None;
    }
    let mut table = [0u8; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        let v = match func {
            T::Identity => c,
            T::Table(values) if values.is_empty() => c,
            T::Discrete(values) if values.is_empty() => c,
            T::Table(values) => {
                let n = values.len() - 1;
                let k = ((c * n as f32).floor() as usize).min(n);
                if k == n {
                    values[k]
                } else {
                    values[k] + (c - k as f32 / n as f32) * n as f32 * (values[k + 1] - values[k])
                }
            }
            T::Discrete(values) => {
                let n = values.len();
                values[((c * n as f32).floor() as usize).min(n - 1)]
            }
            T::Linear { slope, intercept } => slope * c + intercept,
            T::Gamma {
                amplitude,
                exponent,
                offset,
            } => amplitude * c.powf(*exponent) + offset,
        };
        *slot = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    Some(table)
}

fn stops(g: &usvg::BaseGradient) -> (Vec<Color4f>, Vec<f32>, TileMode) {
    let colors = g
        .stops()
        .iter()
        .map(|s| color4(s.color(), s.opacity().get()))
        .collect();
    let offsets = g.stops().iter().map(|s| s.offset().get()).collect();
    let tile = match g.spread_method() {
        usvg::SpreadMethod::Pad => TileMode::Clamp,
        usvg::SpreadMethod::Reflect => TileMode::Mirror,
        usvg::SpreadMethod::Repeat => TileMode::Repeat,
    };
    (colors, offsets, tile)
}

/// Where a group's isolated layer can have pixels, in the group's own space.
fn layer_bounds(g: &usvg::Group) -> Option<Rect> {
    // With filters the layer holds the filter *input*: the filter region.
    let b = if g.filters().is_empty() {
        g.layer_bounding_box()
    } else {
        g.filters_bounding_box().or(g.layer_bounding_box())
    }?;
    Some(rect_of(b.x(), b.y(), b.width(), b.height()).with_outset((2.0, 2.0)))
}

/// The outline of a `clipPath`, when it is made only of plain shapes: the
/// union of its children, intersected with its own `clip-path` if it has one.
/// `None` when path operations cannot express it (the caller then masks).
fn clip_geometry(cp: &usvg::ClipPath) -> Option<Path> {
    let mut acc: Option<Path> = None;
    collect_clip(cp.root(), &Matrix::new_identity(), &mut acc)?;
    let mut path = acc.unwrap_or_default().with_transform(&matrix(cp.transform()));
    if let Some(inner) = cp.clip_path() {
        path = path.op(&clip_geometry(inner)?, PathOp::Intersect)?;
    }
    Some(path)
}

fn collect_clip(g: &usvg::Group, m: &Matrix, acc: &mut Option<Path>) -> Option<()> {
    let add = |p: Path, acc: &mut Option<Path>| -> Option<()> {
        *acc = Some(match acc.take() {
            None => p,
            Some(a) => a.op(&p, PathOp::Union)?,
        });
        Some(())
    };
    for child in g.children() {
        match child {
            usvg::Node::Path(p) => {
                if let (true, Some(fill)) = (p.is_visible(), p.fill()) {
                    add(to_path(p.data(), fill.rule()).with_transform(m), acc)?;
                }
            }
            usvg::Node::Text(text) => collect_clip(text.flattened(), m, acc)?,
            usvg::Node::Group(sub) => {
                let m2 = Matrix::concat(m, &matrix(sub.transform()));
                if let Some(cp) = sub.clip_path() {
                    let mut inner = None;
                    collect_clip(sub, &Matrix::new_identity(), &mut inner)?;
                    let clipped = inner
                        .unwrap_or_default()
                        .op(&clip_geometry(cp)?, PathOp::Intersect)?;
                    add(clipped.with_transform(&m2), acc)?;
                } else {
                    collect_clip(sub, &m2, acc)?;
                }
            }
            usvg::Node::Image(_) => {}
        }
    }
    Some(())
}
