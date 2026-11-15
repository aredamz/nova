//! Drawing a frame plan with Skia's CPU rasteriser.
//!
//! Vector content (shapes, paths, text) is drawn directly at the final
//! transform every frame, never from a transformed bitmap, so nothing gets
//! soft under scaling or rotation. Compositing happens in gamma-encoded sRGB
//! with premultiplied alpha, matching what a browser does for CSS.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use nova_core::{
    Affine, BlendMode, Color, Error, ErrorCode, EvalFill, EvalKind, EvalNode, EvalStroke, Fill,
    FramePlan, LineCap, LineJoin, Node, NodeId, NodeKind, Rect, Result, Scene, Stroke,
};
use skia_safe::canvas::{SaveLayerRec, SrcRectConstraint};
use skia_safe::gradient::{
    Colors, Gradient, Interpolation, interpolation::InPremul, shaders as gradient_shaders,
};
use skia_safe::textlayout::FontCollection;
use skia_safe::{
    AlphaType, BlendMode as SkBlend, BlurStyle, Canvas, Color4f, ColorType, FilterMode,
    ImageFilter, ImageInfo, MaskFilter, Matrix, MipmapMode, Paint, PaintCap, PaintJoin, PaintStyle,
    Path, PathEffect, PathFillType, RRect, Rect as SkRect, SamplingOptions, TileMode,
    image_filters, surfaces,
};

use super::Shared;
use super::layers::CachedLayer;
use super::text::{self, TextLayout};
use crate::{FrameRenderer, RendererStats};

/// Upper bound on cached text layouts per thread (animated font sizes or
/// colours create one entry per distinct value).
const MAX_TEXT_LAYOUTS: usize = 512;

#[derive(Hash, PartialEq, Eq)]
struct TextKey {
    node: NodeId,
    size: u32,
    color: [u32; 4],
    letter_spacing: u32,
    reveal: Option<u32>,
}

pub struct SkiaRenderer {
    shared: Arc<Shared>,
    info: ImageInfo,
    cache: Cache,
}

/// Per-thread caches of objects that must not be shared across threads.
struct Cache {
    fonts: FontCollection,
    texts: HashMap<TextKey, Rc<TextLayout>>,
    paths: HashMap<NodeId, Path>,
    stats: RendererStats,
}

impl SkiaRenderer {
    pub(super) fn new(shared: Arc<Shared>) -> SkiaRenderer {
        let scene = &shared.scene;
        let info = ImageInfo::new(
            (scene.width as i32, scene.height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let fonts = text::font_collection(&shared.fonts, shared.system_fonts);
        SkiaRenderer {
            shared,
            info,
            cache: Cache {
                fonts,
                texts: HashMap::new(),
                paths: HashMap::new(),
                stats: RendererStats::default(),
            },
        }
    }
}

impl FrameRenderer for SkiaRenderer {
    fn render(&mut self, plan: &FramePlan, rgba: &mut [u8]) -> Result<()> {
        let shared = Arc::clone(&self.shared);
        let scene = &shared.scene;
        let expected = scene.width as usize * scene.height as usize * 4;
        if rgba.len() != expected {
            return Err(Error::new(
                ErrorCode::Internal,
                format!("frame buffer is {} bytes, expected {expected}", rgba.len()),
            ));
        }
        let mut surface = surfaces::wrap_pixels(&self.info, rgba, None, None)
            .ok_or_else(|| Error::new(ErrorCode::Render, "could not create a raster surface"))?;
        let canvas = surface.canvas();
        canvas.clear(color4f(scene.background, 1.0));
        let mut painter = Painter {
            scene,
            shared: &shared,
            cache: &mut self.cache,
        };
        for node in &plan.nodes {
            painter.draw(canvas, node);
        }
        self.cache.stats.frames += 1;
        Ok(())
    }

    fn stats(&self) -> RendererStats {
        self.cache.stats
    }
}

struct Painter<'a> {
    scene: &'a Scene,
    shared: &'a Shared,
    cache: &'a mut Cache,
}

impl Painter<'_> {
    fn text_layout(
        &mut self,
        id: NodeId,
        node: &Node,
        size: f32,
        color: Color,
        letter_spacing: f32,
        reveal: Option<u32>,
    ) -> Rc<TextLayout> {
        let NodeKind::Text(text) = &node.kind else {
            unreachable!("text plan on a non-text node")
        };
        let key = TextKey {
            node: id,
            size: size.to_bits(),
            color: [
                color.r.to_bits(),
                color.g.to_bits(),
                color.b.to_bits(),
                color.a.to_bits(),
            ],
            letter_spacing: letter_spacing.to_bits(),
            reveal,
        };
        if let Some(hit) = self.cache.texts.get(&key) {
            self.cache.stats.text_cache_hits += 1;
            return Rc::clone(hit);
        }
        if self.cache.texts.len() >= MAX_TEXT_LAYOUTS {
            self.cache.texts.clear();
        }
        self.cache.stats.text_layouts += 1;
        let layout = Rc::new(text::layout(
            &self.cache.fonts,
            text,
            size,
            color,
            letter_spacing,
            reveal,
        ));
        self.cache.texts.insert(key, Rc::clone(&layout));
        layout
    }

    fn path(&mut self, id: NodeId, node: &Node) -> Path {
        if let Some(p) = self.cache.paths.get(&id) {
            return p.clone();
        }
        let NodeKind::Path { data, even_odd, .. } = &node.kind else {
            unreachable!("path plan on a non-path node")
        };
        // Validated when the backend was prepared, so this cannot fail here.
        let parsed = Path::from_svg(data).unwrap_or_default();
        let path = if *even_odd {
            parsed.with_fill_type(PathFillType::EvenOdd)
        } else {
            parsed
        };
        self.cache.paths.insert(id, path.clone());
        path
    }

    /// The node's own box: what `anchor` and gradient coordinates refer to.
    fn box_size(&mut self, n: &EvalNode, node: &Node) -> (f32, f32) {
        match (&n.kind, &node.kind) {
            (EvalKind::Solid { .. }, _) => (self.scene.width as f32, self.scene.height as f32),
            (EvalKind::Rect { width, height, .. }, _)
            | (EvalKind::Ellipse { width, height, .. }, _) => (*width, *height),
            (EvalKind::Path { .. }, NodeKind::Path { size, .. }) => {
                size.map_or((0.0, 0.0), |s| (s[0], s[1]))
            }
            (
                EvalKind::Text {
                    size,
                    color,
                    letter_spacing,
                    reveal,
                },
                _,
            ) => {
                let layout =
                    self.text_layout(n.node, node, *size, *color, *letter_spacing, *reveal);
                // A background's padding is part of the box.
                let [l, t, r, b] = text_padding(node);
                (layout.width + l + r, layout.height + t + b)
            }
            (EvalKind::Image, NodeKind::Image { asset, size, .. }) => match size {
                Some(s) => (s[0], s[1]),
                None => self.shared.naturals[*asset as usize].unwrap_or((0.0, 0.0)),
            },
            (EvalKind::Group { .. }, NodeKind::Group { size, .. }) => {
                size.map_or((0.0, 0.0), |s| (s[0], s[1]))
            }
            _ => (0.0, 0.0),
        }
    }

    fn local_matrix(&mut self, n: &EvalNode, node: &Node) -> (Affine, (f32, f32)) {
        let (bw, bh) = self.box_size(n, node);
        (
            Affine::layer(
                n.position,
                n.rotation,
                n.scale,
                [n.anchor[0] * bw, n.anchor[1] * bh],
            ),
            (bw, bh),
        )
    }

    fn draw(&mut self, canvas: &Canvas, n: &EvalNode) {
        if self.shared.layers.is_isolated(n.node) {
            self.draw_cached(canvas, n);
        } else {
            self.draw_inline(canvas, n);
        }
    }

    /// Composite the node from the layer cache, rendering it first if the
    /// cache has nothing for this exact node value and transform.
    fn draw_cached(&mut self, canvas: &Canvas, n: &EvalNode) {
        let ctm = canvas.local_to_device_as_3x3();
        let Some(key) = ctm.to_affine() else {
            return self.draw_inline(canvas, n);
        };
        let entry = match self.shared.layers.get(n.node, n, &key) {
            Some(entry) => {
                self.cache.stats.layer_cache_hits += 1;
                entry
            }
            None => {
                self.cache.stats.layer_cache_misses += 1;
                let layer = self.render_layer(n, &ctm, &key);
                self.shared.layers.put(n.node, n, key, layer)
            }
        };
        if let Some(layer) = &entry.layer {
            // Opacity and filters are already in the pixels; only the blend
            // with what is behind the node happens here. Identity transform
            // and a whole-pixel position: an exact copy, never a resample.
            let mut paint = Paint::default();
            paint.set_blend_mode(to_blend(self.scene.node(n.node).blend));
            canvas.save();
            canvas.reset_matrix();
            canvas.draw_image(&layer.image, (layer.x as f32, layer.y as f32), Some(&paint));
            canvas.restore();
        }
    }

    /// Render the node into its own transparent surface, covering just the
    /// part of the screen it can touch.
    fn render_layer(&mut self, n: &EvalNode, ctm: &Matrix, key: &[f32; 6]) -> Option<CachedLayer> {
        let node = self.scene.node(n.node);
        let parent = Affine {
            a: key[0],
            b: key[1],
            c: key[2],
            d: key[3],
            tx: key[4],
            ty: key[5],
        };
        let (local, size) = self.local_matrix(n, node);
        let screen = Rect::from_wh(self.scene.width as f32, self.scene.height as f32);
        let device = match self.content_bounds(n, node, size) {
            Some(content) => parent
                .then_inner(&local)
                .map_rect(&filter_bounds(n, &content))
                .inflate(2.0)
                .intersect(&screen),
            None => screen,
        };
        let (x0, y0) = (device.left.floor() as i32, device.top.floor() as i32);
        let (x1, y1) = (device.right.ceil() as i32, device.bottom.ceil() as i32);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let info = ImageInfo::new(
            (x1 - x0, y1 - y0),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let mut surface = surfaces::raster(&info, None, None)?;
        let canvas = surface.canvas();
        canvas.clear(Color4f::new(0.0, 0.0, 0.0, 0.0));
        canvas.translate((-x0 as f32, -y0 as f32));
        canvas.concat(ctm);
        self.draw_inline(canvas, n);
        Some(CachedLayer {
            image: surface.image_snapshot(),
            x: x0,
            y: y0,
        })
    }

    fn draw_inline(&mut self, canvas: &Canvas, n: &EvalNode) {
        let node = self.scene.node(n.node);
        let (matrix, size) = self.local_matrix(n, node);
        let outer = canvas.save();
        canvas.concat(&to_matrix(&matrix));

        let outline_shadow = self.shared.outline_shadow[n.node as usize];
        if outline_shadow {
            self.draw_outline_shadow(canvas, n, node, size);
        }
        let has_filter = n.blur > 0.0 || (n.shadow.is_some() && !outline_shadow);
        let single = is_single_draw(&n.kind);
        // One draw call can carry opacity and blend mode in its paint. Anything
        // made of several draws must be composited as a unit, like CSS does.
        let needs_layer =
            has_filter || (!single && (n.opacity < 1.0 || node.blend != BlendMode::Normal));

        if needs_layer {
            let mut paint = Paint::default();
            paint.set_alpha_f(n.opacity);
            paint.set_blend_mode(to_blend(node.blend));
            if has_filter {
                paint.set_image_filter(filter(n, !outline_shadow));
            }
            let bounds = self.content_bounds(n, node, size).map(|r| to_sk_rect(&r));
            let mut rec = SaveLayerRec::default().paint(&paint);
            if let Some(b) = &bounds {
                rec = rec.bounds(b);
            }
            canvas.save_layer(&rec);
            self.cache.stats.offscreen_layers += 1;
            self.draw_content(canvas, n, node, size, 1.0, BlendMode::Normal);
        } else {
            self.draw_content(canvas, n, node, size, n.opacity, node.blend);
        }
        canvas.restore_to_count(outer);
    }

    /// Shadow of a node that is one opaque shape: the shape itself, offset,
    /// in the shadow colour, with a Gaussian mask blur.
    fn draw_outline_shadow(
        &mut self,
        canvas: &Canvas,
        n: &EvalNode,
        node: &Node,
        size: (f32, f32),
    ) {
        let Some(s) = &n.shadow else { return };
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color4f(color4f(s.color, 1.0), None);
        let sigma = s.blur / 2.0;
        if sigma > 0.0 {
            paint.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, sigma, true));
        }
        canvas.save();
        canvas.translate((s.x, s.y));
        match (&n.kind, &node.kind) {
            (
                EvalKind::Rect {
                    width,
                    height,
                    radius,
                    ..
                },
                _,
            ) => {
                let rect = SkRect::from_wh(*width, *height);
                if *radius > 0.0 {
                    canvas.draw_rrect(RRect::new_rect_xy(rect, *radius, *radius), &paint);
                } else {
                    canvas.draw_rect(rect, &paint);
                }
            }
            (EvalKind::Ellipse { width, height, .. }, _) => {
                canvas.draw_oval(SkRect::from_wh(*width, *height), &paint);
            }
            (EvalKind::Image, NodeKind::Image { asset, fit, .. }) => {
                if let Some(natural) = self.shared.naturals[*asset as usize] {
                    let (_, dst) = fit.rects(natural, size);
                    canvas.draw_rect(to_sk_rect(&dst), &paint);
                }
            }
            _ => {}
        }
        canvas.restore();
    }

    fn draw_content(
        &mut self,
        canvas: &Canvas,
        n: &EvalNode,
        node: &Node,
        size: (f32, f32),
        alpha: f32,
        blend: BlendMode,
    ) {
        let (bw, bh) = size;
        match &n.kind {
            EvalKind::Solid { color } => {
                let mut paint = Paint::default();
                paint.set_color4f(color4f(*color, alpha), None);
                paint.set_blend_mode(to_blend(blend));
                canvas.draw_rect(SkRect::from_wh(bw, bh), &paint);
            }
            EvalKind::Rect {
                width,
                height,
                radius,
                fill,
                stroke,
            } => {
                let rect = SkRect::from_wh(*width, *height);
                let (fill_def, stroke_def) = paint_defs(&node.kind);
                let shape = |paint: &Paint| {
                    if *radius > 0.0 {
                        canvas.draw_rrect(RRect::new_rect_xy(rect, *radius, *radius), paint);
                    } else {
                        canvas.draw_rect(rect, paint);
                    }
                };
                if let (Some(f), Some(def)) = (fill, fill_def) {
                    shape(&fill_paint(f, def, size, alpha, blend));
                }
                if let (Some(s), Some(def)) = (stroke, stroke_def) {
                    shape(&stroke_paint(s, def, alpha, blend));
                }
            }
            EvalKind::Ellipse {
                width,
                height,
                fill,
                stroke,
            } => {
                let rect = SkRect::from_wh(*width, *height);
                let (fill_def, stroke_def) = paint_defs(&node.kind);
                if let (Some(f), Some(def)) = (fill, fill_def) {
                    canvas.draw_oval(rect, &fill_paint(f, def, size, alpha, blend));
                }
                if let (Some(s), Some(def)) = (stroke, stroke_def) {
                    canvas.draw_oval(rect, &stroke_paint(s, def, alpha, blend));
                }
            }
            EvalKind::Path { fill, stroke } => {
                let path = self.path(n.node, node);
                let (fill_def, stroke_def) = paint_defs(&node.kind);
                // Without an explicit box, gradients span the path's own bounds.
                let gradient_box = if bw > 0.0 && bh > 0.0 {
                    (bw, bh)
                } else {
                    (path.bounds().right, path.bounds().bottom)
                };
                if let (Some(f), Some(def)) = (fill, fill_def) {
                    canvas.draw_path(&path, &fill_paint(f, def, gradient_box, alpha, blend));
                }
                if let (Some(s), Some(def)) = (stroke, stroke_def) {
                    canvas.draw_path(&path, &stroke_paint(s, def, alpha, blend));
                }
            }
            EvalKind::Text {
                size: font_size,
                color,
                letter_spacing,
                reveal,
            } => {
                // Opacity and blend for text always arrive through a layer.
                let layout =
                    self.text_layout(n.node, node, *font_size, *color, *letter_spacing, *reveal);
                let [left, top, _, _] = text_padding(node);
                if let NodeKind::Text(text) = &node.kind
                    && let Some(panel) = &text.background
                {
                    let rect = SkRect::from_wh(bw, bh);
                    let shape = |rect: SkRect, radius: f32, paint: &Paint| {
                        if radius > 0.0 {
                            canvas.draw_rrect(RRect::new_rect_xy(rect, radius, radius), paint);
                        } else {
                            canvas.draw_rect(rect, paint);
                        }
                    };
                    if let Some(fill) = panel.color {
                        let mut paint = Paint::default();
                        paint.set_anti_alias(true);
                        paint.set_color4f(color4f(fill, 1.0), None);
                        shape(rect, panel.radius, &paint);
                    }
                    if let Some((border, width)) = panel.border {
                        // Like a CSS border: inside the panel's edge.
                        let mut paint = Paint::default();
                        paint.set_anti_alias(true);
                        paint.set_style(PaintStyle::Stroke);
                        paint.set_stroke_width(width);
                        paint.set_color4f(color4f(border, 1.0), None);
                        let inset = rect.with_inset((width / 2.0, width / 2.0));
                        shape(inset, (panel.radius - width / 2.0).max(0.0), &paint);
                    }
                }
                layout.paragraph.paint(canvas, (left, top));
            }
            EvalKind::Image => self.draw_image(canvas, n.node, node, size, alpha, blend),
            EvalKind::Group { children } => {
                if let NodeKind::Group {
                    clip: true,
                    size: Some(s),
                    radius,
                    ..
                } = &node.kind
                {
                    let rect = SkRect::from_wh(s[0], s[1]);
                    if *radius > 0.0 {
                        canvas.clip_rrect(RRect::new_rect_xy(rect, *radius, *radius), None, true);
                    } else {
                        canvas.clip_rect(rect, None, true);
                    }
                }
                for child in children {
                    self.draw(canvas, child);
                }
            }
        }
    }

    fn draw_image(
        &mut self,
        canvas: &Canvas,
        id: NodeId,
        node: &Node,
        size: (f32, f32),
        alpha: f32,
        blend: BlendMode,
    ) {
        let NodeKind::Image { asset, fit, .. } = &node.kind else {
            return;
        };
        let (Some(plan), Some(natural)) = (
            &self.shared.image_plans[id as usize],
            self.shared.naturals[*asset as usize],
        ) else {
            return;
        };
        if size.0 <= 0.0 || size.1 <= 0.0 {
            return;
        }
        let (src, dst) = fit.rects(natural, size);
        let dst = to_sk_rect(&dst);
        let (iw, ih) = (plan.image.width() as f32, plan.image.height() as f32);
        // Where the shown region sits inside the prepared bitmap.
        let src_px = if plan.exact {
            SkRect::from_wh(iw, ih)
        } else {
            let (kx, ky) = (iw / plan.covers.width(), ih / plan.covers.height());
            SkRect::new(
                (src.left - plan.covers.left) * kx,
                (src.top - plan.covers.top) * ky,
                (src.right - plan.covers.left) * kx,
                (src.bottom - plan.covers.top) * ky,
            )
        };
        if src_px.is_empty() || dst.is_empty() {
            return;
        }

        // Choose the cheapest sampler that is exact for how the bitmap lands
        // on screen. The bitmap was prepared at the layer's largest size, so
        // the common cases are a straight copy or a mild reduction.
        let m = canvas.local_to_device_as_3x3();
        let (kx, ky) = (dst.width() / src_px.width(), dst.height() / src_px.height());
        let axis_aligned =
            m.skew_x() == 0.0 && m.skew_y() == 0.0 && m.scale_x() > 0.0 && m.scale_y() > 0.0;
        let one_to_one = axis_aligned
            && (m.scale_x() * kx - 1.0).abs() < 1e-4
            && (m.scale_y() * ky - 1.0).abs() < 1e-4;
        let origin = m.map_point((dst.left, dst.top));
        let on_pixel_grid = (origin.x - origin.x.round()).abs() < 1e-3
            && (origin.y - origin.y.round()).abs() < 1e-3;
        let scale = m.max_scale().max(0.0) * kx.max(ky);
        let sampling = if one_to_one && on_pixel_grid {
            SamplingOptions::new(FilterMode::Nearest, MipmapMode::None)
        } else if scale < 0.5 {
            SamplingOptions::new(FilterMode::Linear, MipmapMode::Nearest)
        } else {
            SamplingOptions::new(FilterMode::Linear, MipmapMode::None)
        };

        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_alpha_f(alpha);
        paint.set_blend_mode(to_blend(blend));
        let whole = src_px == SkRect::from_wh(iw, ih);
        let constraint = if whole {
            SrcRectConstraint::Fast
        } else {
            SrcRectConstraint::Strict
        };
        canvas.draw_image_rect_with_sampling_options(
            &plan.image,
            Some((&src_px, constraint)),
            dst,
            sampling,
            &paint,
        );
    }

    /// Bounds of what a node draws, in its own coordinate space, before its
    /// filters. `None` means unknown. Used to keep offscreen layers small.
    fn content_bounds(&mut self, n: &EvalNode, node: &Node, size: (f32, f32)) -> Option<Rect> {
        let (bw, bh) = size;
        let stroke_pad = |s: &Option<EvalStroke>| s.map_or(0.0, |s| s.width * 2.0 + 1.0);
        match &n.kind {
            EvalKind::Solid { .. } | EvalKind::Image => Some(Rect::from_wh(bw, bh)),
            EvalKind::Rect { stroke, .. } | EvalKind::Ellipse { stroke, .. } => {
                Some(Rect::from_wh(bw, bh).inflate(stroke_pad(stroke)))
            }
            EvalKind::Path { stroke, .. } => {
                let b = *self.path(n.node, node).bounds();
                Some(Rect::new(b.left, b.top, b.right, b.bottom).inflate(stroke_pad(stroke) + 1.0))
            }
            // Glyphs may overhang the layout box (italics, swashes, accents).
            EvalKind::Text {
                size: font_size, ..
            } => Some(Rect::from_wh(bw, bh).inflate(font_size * 0.5)),
            EvalKind::Group { children } => {
                let mut acc: Option<Rect> = None;
                for child in children {
                    let child_node = self.scene.node(child.node);
                    let (matrix, child_size) = self.local_matrix(child, child_node);
                    let inner = self.content_bounds(child, child_node, child_size)?;
                    let outer = matrix.map_rect(&filter_bounds(child, &inner));
                    acc = Some(acc.map_or(outer, |a| a.union(&outer)));
                }
                let all = acc?;
                match &node.kind {
                    NodeKind::Group {
                        clip: true,
                        size: Some(s),
                        ..
                    } => Some(all.intersect(&Rect::from_wh(s[0], s[1]))),
                    _ => Some(all),
                }
            }
        }
    }
}

/// Grow `content` by whatever the node's blur and shadow add around it.
fn filter_bounds(n: &EvalNode, content: &Rect) -> Rect {
    let mut out = if n.blur > 0.0 {
        content.inflate(n.blur * 3.0 + 1.0)
    } else {
        *content
    };
    if let Some(s) = &n.shadow {
        let shadow = out.offset(s.x, s.y).inflate(s.blur * 1.5 + 1.0);
        out = out.union(&shadow);
    }
    out
}

fn filter(n: &EvalNode, with_shadow: bool) -> Option<ImageFilter> {
    let mut f: Option<ImageFilter> = None;
    if n.blur > 0.0 {
        f = image_filters::blur(
            (n.blur, n.blur),
            TileMode::Decal,
            None,
            image_filters::CropRect::NO_CROP_RECT,
        );
    }
    if let Some(s) = n.shadow.as_ref().filter(|_| with_shadow) {
        // CSS blur radius is twice the Gaussian standard deviation.
        let sigma = s.blur / 2.0;
        f = image_filters::drop_shadow(
            (s.x, s.y),
            (sigma, sigma),
            color4f(s.color, 1.0),
            None,
            f,
            image_filters::CropRect::NO_CROP_RECT,
        );
    }
    f
}

/// True when the node is drawn with exactly one draw call.
fn is_single_draw(kind: &EvalKind) -> bool {
    match kind {
        EvalKind::Solid { .. } | EvalKind::Image => true,
        EvalKind::Rect { fill, stroke, .. }
        | EvalKind::Ellipse { fill, stroke, .. }
        | EvalKind::Path { fill, stroke } => fill.is_some() != stroke.is_some(),
        EvalKind::Text { .. } | EvalKind::Group { .. } => false,
    }
}

/// Padding of a text layer's background panel: left, top, right, bottom.
fn text_padding(node: &Node) -> [f32; 4] {
    match &node.kind {
        NodeKind::Text(text) => text.background.map_or([0.0; 4], |b| b.padding),
        _ => [0.0; 4],
    }
}

fn paint_defs(kind: &NodeKind) -> (Option<&Fill>, Option<&Stroke>) {
    match kind {
        NodeKind::Rect { fill, stroke, .. }
        | NodeKind::Ellipse { fill, stroke, .. }
        | NodeKind::Path { fill, stroke, .. } => (fill.as_ref(), stroke.as_ref()),
        _ => (None, None),
    }
}

fn fill_paint(
    fill: &EvalFill,
    def: &Fill,
    size: (f32, f32),
    alpha: f32,
    blend: BlendMode,
) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(to_blend(blend));
    match (fill, def) {
        (EvalFill::Solid(color), _) => {
            paint.set_color4f(color4f(*color, alpha), None);
        }
        (EvalFill::Gradient, def) => {
            let (w, h) = size;
            let stops = match def {
                Fill::Linear { stops, .. } | Fill::Radial { stops, .. } => stops.as_slice(),
                Fill::Solid(_) => &[],
            };
            let colors: Vec<Color4f> = stops.iter().map(|s| color4f(s.color, 1.0)).collect();
            let offsets: Vec<f32> = stops.iter().map(|s| s.offset).collect();
            // Interpolate premultiplied, as CSS gradients do, so a fade to
            // transparent does not pass through grey.
            let interpolation = Interpolation {
                in_premul: InPremul::Yes,
                ..Interpolation::default()
            };
            let gradient = Gradient::new(
                Colors::new(&colors, Some(&offsets), TileMode::Clamp, None),
                interpolation,
            );
            let shader = match def {
                Fill::Linear { from, to, .. } => gradient_shaders::linear_gradient(
                    ((from[0] * w, from[1] * h), (to[0] * w, to[1] * h)),
                    &gradient,
                    None,
                ),
                Fill::Radial { center, radius, .. } => gradient_shaders::radial_gradient(
                    ((center[0] * w, center[1] * h), radius * w),
                    &gradient,
                    None,
                ),
                Fill::Solid(_) => None,
            };
            paint.set_shader(shader);
            paint.set_alpha_f(alpha);
            // 8-bit gradients band; dithering hides it at no visible cost.
            paint.set_dither(true);
        }
    }
    paint
}

fn stroke_paint(stroke: &EvalStroke, def: &Stroke, alpha: f32, blend: BlendMode) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_blend_mode(to_blend(blend));
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(stroke.width);
    paint.set_stroke_cap(match def.cap {
        LineCap::Butt => PaintCap::Butt,
        LineCap::Round => PaintCap::Round,
        LineCap::Square => PaintCap::Square,
    });
    paint.set_stroke_join(match def.join {
        LineJoin::Miter => PaintJoin::Miter,
        LineJoin::Round => PaintJoin::Round,
        LineJoin::Bevel => PaintJoin::Bevel,
    });
    paint.set_color4f(color4f(stroke.color, alpha), None);
    if stroke.trim_start > 0.0 || stroke.trim_end < 1.0 {
        paint.set_path_effect(PathEffect::trim(stroke.trim_start, stroke.trim_end, None));
    }
    paint
}

fn color4f(c: Color, alpha: f32) -> Color4f {
    Color4f::new(
        c.r.clamp(0.0, 1.0),
        c.g.clamp(0.0, 1.0),
        c.b.clamp(0.0, 1.0),
        (c.a * alpha).clamp(0.0, 1.0),
    )
}

fn to_matrix(m: &Affine) -> Matrix {
    Matrix::new_all(m.a, m.c, m.tx, m.b, m.d, m.ty, 0.0, 0.0, 1.0)
}

fn to_sk_rect(r: &Rect) -> SkRect {
    SkRect::new(r.left, r.top, r.right, r.bottom)
}

fn to_blend(b: BlendMode) -> SkBlend {
    match b {
        BlendMode::Normal => SkBlend::SrcOver,
        BlendMode::Multiply => SkBlend::Multiply,
        BlendMode::Screen => SkBlend::Screen,
        BlendMode::Overlay => SkBlend::Overlay,
        BlendMode::Darken => SkBlend::Darken,
        BlendMode::Lighten => SkBlend::Lighten,
        BlendMode::ColorDodge => SkBlend::ColorDodge,
        BlendMode::ColorBurn => SkBlend::ColorBurn,
        BlendMode::HardLight => SkBlend::HardLight,
        BlendMode::SoftLight => SkBlend::SoftLight,
        BlendMode::Difference => SkBlend::Difference,
        BlendMode::Exclusion => SkBlend::Exclusion,
        BlendMode::Add => SkBlend::Plus,
    }
}
