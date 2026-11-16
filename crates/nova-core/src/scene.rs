use std::path::PathBuf;
use std::sync::Arc;

use crate::anim::Prop;
use crate::color::Color;
use crate::geometry::Rect;
use crate::time::Fps;

pub type NodeId = u32;
pub type AssetId = u32;

/// A compiled composition: validated, normalised and immutable.
///
/// Built once per render job by `nova-template`, then shared read-only by all
/// render workers. Nothing here changes while frames are being produced.
#[derive(Debug)]
pub struct Scene {
    pub id: String,
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: Fps,
    pub duration_frames: u32,
    /// Opaque background every frame starts from.
    pub background: Color,
    /// All nodes; children are referenced by index.
    pub nodes: Vec<Node>,
    /// Top-level layers in paint order (first is furthest back).
    pub roots: Vec<NodeId>,
    pub assets: Vec<Asset>,
    pub audio: Vec<AudioTrack>,
}

impl Scene {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    pub fn asset(&self, id: AssetId) -> &Asset {
        &self.assets[id as usize]
    }

    pub fn duration_seconds(&self) -> f64 {
        self.fps.frame_to_seconds(self.duration_frames as i64)
    }

    /// The same composition at `factor` times the size. Nothing is resampled:
    /// every layer sits under one more scale, so vector content (shapes, text,
    /// baked frames) is drawn again at the larger size. Width and height are
    /// rounded to even numbers, which the encoder needs.
    pub fn scaled(&self, factor: f32) -> Scene {
        let even = |v: u32| {
            let x = (v as f32 * factor).round().max(2.0) as u32;
            x + (x & 1)
        };
        let (width, height) = (even(self.width), even(self.height));
        let mut nodes = self.nodes.clone();
        let start = self.roots.iter().map(|&r| self.node(r).start).min().unwrap_or(0);
        let end = self.roots.iter().map(|&r| self.node(r).end).max().unwrap_or(0);
        nodes.push(Node {
            id: "@scale".to_owned(),
            start: start.min(0),
            end: end.max(self.duration_frames as i64),
            origin: 0,
            pos_x: Prop::Const(0.0),
            pos_y: Prop::Const(0.0),
            scale_x: Prop::Const(width as f32 / self.width as f32),
            scale_y: Prop::Const(height as f32 / self.height as f32),
            rotation: Prop::Const(0.0),
            anchor_x: Prop::Const(0.0),
            anchor_y: Prop::Const(0.0),
            opacity: Prop::Const(1.0),
            blur: Prop::Const(0.0),
            shadow: None,
            blend: BlendMode::Normal,
            kind: NodeKind::Group {
                children: self.roots.clone(),
                size: None,
                clip: false,
                radius: 0.0,
            },
        });
        Scene {
            id: self.id.clone(),
            name: self.name.clone(),
            width,
            height,
            fps: self.fps,
            duration_frames: self.duration_frames,
            background: self.background,
            roots: vec![(nodes.len() - 1) as NodeId],
            nodes,
            assets: self.assets.clone(),
            audio: self.audio.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Image,
    Svg,
    Font,
    /// A baked frame sequence (see [`FramePack`]).
    Frames,
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub id: String,
    pub kind: AssetKind,
    /// Path as written in the template (for messages only).
    pub src: String,
    /// File contents, read once at compile time.
    pub bytes: Arc<[u8]>,
    /// Hash of `bytes`; cache keys use this, never the file name.
    pub content_hash: u64,
    /// Family name a font is registered under.
    pub family: Option<String>,
    /// Frame packs only: the parsed manifest. `bytes` then holds the manifest file.
    pub pack: Option<Arc<FramePack>>,
}

/// Where one baked frame lives inside a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameEntry {
    /// Index into [`FramePack::shards`].
    pub shard: u32,
    pub offset: u64,
    /// Compressed (zlib) length in the shard.
    pub len: u32,
    /// Length after decompression; checked before allocating.
    pub raw_len: u32,
    /// Frames with the same content id hold identical drawings, so equal ids
    /// evaluate to equal frame plans and the frame is not rendered twice.
    pub content: u32,
}

/// A baked frame sequence: for every frame a display list (SVG documents and
/// shader passes) produced outside NOVA, for example by running a Remotion
/// composition without a browser. NOVA only rasterises it.
#[derive(Debug, Clone, PartialEq)]
pub struct FramePack {
    pub width: u32,
    pub height: u32,
    /// Shard files, already resolved and checked to lie under the asset root.
    pub shards: Vec<PathBuf>,
    pub index: Vec<FrameEntry>,
    /// (key, SkSL source) of every shader the frames reference.
    pub shaders: Vec<(String, String)>,
    /// Folder that files referenced from the frames (images) resolve against.
    pub dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AudioTrack {
    pub id: String,
    pub path: PathBuf,
    /// Composition frame at which the track starts playing.
    pub start_frame: i64,
    pub volume: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    /// Additive ("plus-lighter").
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Stretch to the box, ignoring aspect ratio.
    Fill,
    /// Scale to fit inside the box, preserving aspect ratio.
    #[default]
    Contain,
    /// Scale to cover the box, cropping the overflow.
    Cover,
    /// Natural size, centred, cropped to the box.
    None,
}

impl Fit {
    /// Which part of an image (`natural` size) is shown, and where in the
    /// layer's box it goes. Returns `(source, destination)`: source in the
    /// image's own units, destination in box units.
    pub fn rects(self, natural: (f32, f32), bx: (f32, f32)) -> (Rect, Rect) {
        let (nw, nh) = natural;
        let (bw, bh) = bx;
        let full = Rect::from_wh(nw, nh);
        match self {
            Fit::Fill => (full, Rect::from_wh(bw, bh)),
            Fit::Contain => {
                let s = (bw / nw).min(bh / nh);
                let (dw, dh) = (nw * s, nh * s);
                (
                    full,
                    Rect::new(
                        (bw - dw) / 2.0,
                        (bh - dh) / 2.0,
                        (bw + dw) / 2.0,
                        (bh + dh) / 2.0,
                    ),
                )
            }
            Fit::Cover => {
                let s = (bw / nw).max(bh / nh);
                let (sw, sh) = (bw / s, bh / s);
                (
                    Rect::new(
                        (nw - sw) / 2.0,
                        (nh - sh) / 2.0,
                        (nw + sw) / 2.0,
                        (nh + sh) / 2.0,
                    ),
                    Rect::from_wh(bw, bh),
                )
            }
            Fit::None => {
                let (dw, dh) = (nw.min(bw), nh.min(bh));
                (
                    Rect::new(
                        (nw - dw) / 2.0,
                        (nh - dh) / 2.0,
                        (nw + dw) / 2.0,
                        (nh + dh) / 2.0,
                    ),
                    Rect::new(
                        (bw - dw) / 2.0,
                        (bh - dh) / 2.0,
                        (bw + dw) / 2.0,
                        (bh + dh) / 2.0,
                    ),
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}

/// How a shape's interior is painted. Gradient geometry is in units of the
/// layer's box (0..=1 on each axis), like SVG `objectBoundingBox`.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    Solid(Prop<Color>),
    Linear {
        from: [f32; 2],
        to: [f32; 2],
        stops: Vec<GradientStop>,
    },
    Radial {
        center: [f32; 2],
        radius: f32,
        stops: Vec<GradientStop>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub color: Prop<Color>,
    pub width: Prop<f32>,
    pub cap: LineCap,
    pub join: LineJoin,
    /// Visible part of the outline, 0..=1 along its length ("draw-on").
    pub trim_start: Prop<f32>,
    pub trim_end: Prop<f32>,
}

/// Drop shadow with CSS semantics: `blur` is the CSS blur radius (2 sigma).
#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    pub x: Prop<f32>,
    pub y: Prop<f32>,
    pub blur: Prop<f32>,
    pub color: Prop<Color>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextNode {
    pub value: String,
    /// Preferred family first, then fallbacks.
    pub families: Vec<String>,
    pub size: Prop<f32>,
    /// CSS weight, 100..=900.
    pub weight: u16,
    pub italic: bool,
    pub color: Prop<Color>,
    /// Multiple of the font size; `None` uses the font's own line spacing.
    pub line_height: Option<f32>,
    pub letter_spacing: Prop<f32>,
    pub align: TextAlign,
    /// Wrap width in pixels; `None` never wraps (explicit `\n` still breaks).
    pub max_width: Option<f32>,
    pub max_lines: Option<u32>,
    /// Shrink the box to the text when it fits in `max_width` without wrapping.
    pub hug: bool,
    /// Ranges painted in their own colour: byte offsets into `value`,
    /// sorted and non-overlapping.
    pub spans: Vec<TextSpan>,
    /// Progressive reveal (karaoke captions, typewriter).
    pub reveal: Option<Reveal>,
    /// Panel drawn behind the text; its padding is part of the layer's box.
    pub background: Option<TextBox>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealUnit {
    Word,
    Char,
}

/// Only the first `count` words or characters are shown in their real
/// colour; the rest are painted in `hidden` (often a dim or clear colour).
/// The layout never changes, so nothing shifts as text is revealed.
#[derive(Debug, Clone, PartialEq)]
pub struct Reveal {
    pub unit: RevealUnit,
    pub count: Prop<f32>,
    pub hidden: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBox {
    pub color: Option<Color>,
    pub radius: f32,
    /// left, top, right, bottom
    pub padding: [f32; 4],
    /// colour and width of an outline drawn inside the panel's edge
    pub border: Option<(Color, f32)>,
}

impl TextNode {
    /// Byte offset in `value` where the hidden part starts when `count`
    /// units are revealed.
    pub fn reveal_offset(&self, unit: RevealUnit, count: u32) -> usize {
        let count = count as usize;
        match unit {
            RevealUnit::Char => self
                .value
                .char_indices()
                .nth(count)
                .map_or(self.value.len(), |(i, _)| i),
            RevealUnit::Word => {
                // the hidden part starts at the first character of word `count`
                let mut words = 0;
                let mut in_word = false;
                for (i, c) in self.value.char_indices() {
                    if c.is_whitespace() {
                        in_word = false;
                    } else if !in_word {
                        if words == count {
                            return i;
                        }
                        words += 1;
                        in_word = true;
                    }
                }
                self.value.len()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// Rectangle the size of the composition.
    Solid {
        color: Prop<Color>,
    },
    Rect {
        width: Prop<f32>,
        height: Prop<f32>,
        radius: Prop<f32>,
        fill: Option<Fill>,
        stroke: Option<Stroke>,
    },
    Ellipse {
        width: Prop<f32>,
        height: Prop<f32>,
        fill: Option<Fill>,
        stroke: Option<Stroke>,
    },
    /// SVG path data. `size` is the box used for the anchor and for gradient
    /// units; it does not scale the path.
    Path {
        data: String,
        size: Option<[f32; 2]>,
        fill: Option<Fill>,
        stroke: Option<Stroke>,
        even_odd: bool,
    },
    Text(TextNode),
    /// Raster image or (pre-rasterised) static SVG.
    Image {
        asset: AssetId,
        size: Option<[f32; 2]>,
        fit: Fit,
    },
    /// One frame of a baked sequence per composition frame; local frame 0 of
    /// the layer shows the pack's first frame, and the last frame is held.
    Frames {
        asset: AssetId,
        size: [f32; 2],
        /// Number of frames in the pack.
        frames: u32,
    },
    /// `radius` rounds the corners of the clip (only used when `clip` is set).
    Group {
        children: Vec<NodeId>,
        size: Option<[f32; 2]>,
        clip: bool,
        radius: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The id given in the template.
    pub id: String,
    /// Composition frames during which the node exists: `start <= f < end`.
    /// Already intersected with every ancestor's range.
    pub start: i64,
    pub end: i64,
    /// Composition frame that is this node's local frame 0. Keyframes and
    /// children are timed relative to it.
    pub origin: i64,
    pub pos_x: Prop<f32>,
    pub pos_y: Prop<f32>,
    pub scale_x: Prop<f32>,
    pub scale_y: Prop<f32>,
    /// Degrees, clockwise on screen.
    pub rotation: Prop<f32>,
    /// Anchor as a fraction of the node's box (0,0 = top-left, 0.5,0.5 = centre).
    pub anchor_x: Prop<f32>,
    pub anchor_y: Prop<f32>,
    pub opacity: Prop<f32>,
    /// Gaussian blur of the whole node, sigma in pixels (CSS `blur()`).
    pub blur: Prop<f32>,
    pub shadow: Option<Shadow>,
    pub blend: BlendMode,
    pub kind: NodeKind,
}

impl Node {
    pub fn is_active(&self, frame: i64) -> bool {
        frame >= self.start && frame < self.end
    }

    pub fn type_name(&self) -> &'static str {
        match self.kind {
            NodeKind::Solid { .. } => "solid",
            NodeKind::Rect { .. } => "rect",
            NodeKind::Ellipse { .. } => "ellipse",
            NodeKind::Path { .. } => "path",
            NodeKind::Text(_) => "text",
            NodeKind::Image { .. } => "image",
            NodeKind::Frames { .. } => "frames",
            NodeKind::Group { .. } => "group",
        }
    }

    /// Composition-frame intervals `[from, to]` during which some property of
    /// this node (not of its children) is changing. With `transform_only`,
    /// only the properties that move the node and everything inside it.
    pub fn animated_spans(&self, transform_only: bool) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let origin = self.origin as f64;
        let mut add = |span: Option<(f64, f64)>| {
            if let Some((a, b)) = span {
                out.push((origin + a, origin + b));
            }
        };
        for p in [
            &self.pos_x,
            &self.pos_y,
            &self.scale_x,
            &self.scale_y,
            &self.rotation,
            &self.anchor_x,
            &self.anchor_y,
        ] {
            add(p.key_span());
        }
        if transform_only {
            return out;
        }
        add(self.opacity.key_span());
        add(self.blur.key_span());
        if let Some(s) = &self.shadow {
            add(s.x.key_span());
            add(s.y.key_span());
            add(s.blur.key_span());
            add(s.color.key_span());
        }
        let (fill, stroke): (Option<&Fill>, Option<&Stroke>) = match &self.kind {
            NodeKind::Solid { color } => {
                add(color.key_span());
                (None, None)
            }
            NodeKind::Rect {
                width,
                height,
                radius,
                fill,
                stroke,
            } => {
                add(width.key_span());
                add(height.key_span());
                add(radius.key_span());
                (fill.as_ref(), stroke.as_ref())
            }
            NodeKind::Ellipse {
                width,
                height,
                fill,
                stroke,
            } => {
                add(width.key_span());
                add(height.key_span());
                (fill.as_ref(), stroke.as_ref())
            }
            NodeKind::Path { fill, stroke, .. } => (fill.as_ref(), stroke.as_ref()),
            NodeKind::Text(t) => {
                add(t.size.key_span());
                add(t.color.key_span());
                add(t.letter_spacing.key_span());
                if let Some(r) = &t.reveal {
                    add(r.count.key_span());
                }
                (None, None)
            }
            NodeKind::Frames { frames, .. } => {
                // The drawing changes every frame for as long as the pack lasts.
                if *frames > 1 {
                    add(Some((0.0, *frames as f64)));
                }
                (None, None)
            }
            NodeKind::Image { .. } | NodeKind::Group { .. } => (None, None),
        };
        if let Some(Fill::Solid(c)) = fill {
            add(c.key_span());
        }
        if let Some(s) = stroke {
            add(s.color.key_span());
            add(s.width.key_span());
            add(s.trim_start.key_span());
            add(s.trim_end.key_span());
        }
        out
    }

    /// True when the node applies a blur or a drop shadow at some point.
    pub fn has_filter(&self) -> bool {
        self.shadow.is_some() || !matches!(self.blur.as_const(), Some(b) if *b <= 0.0)
    }

    /// True when nothing about this node (ignoring children) changes over time.
    pub fn is_static(&self) -> bool {
        let base = self.pos_x.is_const()
            && self.pos_y.is_const()
            && self.scale_x.is_const()
            && self.scale_y.is_const()
            && self.rotation.is_const()
            && self.anchor_x.is_const()
            && self.anchor_y.is_const()
            && self.opacity.is_const()
            && self.blur.is_const()
            && self.shadow.as_ref().is_none_or(|s| {
                s.x.is_const() && s.y.is_const() && s.blur.is_const() && s.color.is_const()
            });
        let fill_const = |f: &Option<Fill>| match f {
            Some(Fill::Solid(c)) => c.is_const(),
            _ => true,
        };
        let stroke_const = |s: &Option<Stroke>| {
            s.as_ref().is_none_or(|s| {
                s.color.is_const()
                    && s.width.is_const()
                    && s.trim_start.is_const()
                    && s.trim_end.is_const()
            })
        };
        base && match &self.kind {
            NodeKind::Solid { color } => color.is_const(),
            NodeKind::Rect {
                width,
                height,
                radius,
                fill,
                stroke,
            } => {
                width.is_const()
                    && height.is_const()
                    && radius.is_const()
                    && fill_const(fill)
                    && stroke_const(stroke)
            }
            NodeKind::Ellipse {
                width,
                height,
                fill,
                stroke,
            } => width.is_const() && height.is_const() && fill_const(fill) && stroke_const(stroke),
            NodeKind::Path { fill, stroke, .. } => fill_const(fill) && stroke_const(stroke),
            NodeKind::Text(t) => {
                t.size.is_const()
                    && t.color.is_const()
                    && t.letter_spacing.is_const()
                    && t.reveal.as_ref().is_none_or(|r| r.count.is_const())
            }
            NodeKind::Frames { frames, .. } => *frames <= 1,
            NodeKind::Image { .. } | NodeKind::Group { .. } => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_offsets_fall_on_word_and_character_starts() {
        let text = TextNode {
            value: "Jam  émas hilang".into(),
            families: vec!["X".into()],
            size: Prop::Const(10.0),
            weight: 400,
            italic: false,
            color: Prop::Const(Color::WHITE),
            line_height: None,
            letter_spacing: Prop::Const(0.0),
            align: TextAlign::Left,
            max_width: None,
            max_lines: None,
            hug: false,
            spans: vec![],
            reveal: None,
            background: None,
        };
        // words: "Jam" at 0, "émas" at 5 (two spaces before it), "hilang" at 11 (é is 2 bytes)
        assert_eq!(text.reveal_offset(RevealUnit::Word, 0), 0);
        assert_eq!(text.reveal_offset(RevealUnit::Word, 1), 5);
        assert_eq!(text.reveal_offset(RevealUnit::Word, 2), 11);
        assert_eq!(text.reveal_offset(RevealUnit::Word, 3), text.value.len());
        assert_eq!(text.reveal_offset(RevealUnit::Word, 99), text.value.len());
        // characters are counted as characters, never splitting a multi-byte one
        assert_eq!(text.reveal_offset(RevealUnit::Char, 5), 5);
        assert_eq!(text.reveal_offset(RevealUnit::Char, 6), 7);
        assert_eq!(text.reveal_offset(RevealUnit::Char, 999), text.value.len());
    }

    #[test]
    fn fit_modes() {
        let natural = (1600.0, 1200.0); // 4:3
        let square = (800.0, 800.0);
        let full = Rect::from_wh(1600.0, 1200.0);

        let (src, dst) = Fit::Fill.rects(natural, square);
        assert_eq!((src, dst), (full, Rect::from_wh(800.0, 800.0)));

        // contain: whole image, letterboxed top and bottom
        let (src, dst) = Fit::Contain.rects(natural, square);
        assert_eq!(src, full);
        assert_eq!(dst, Rect::new(0.0, 100.0, 800.0, 700.0));

        // cover: box filled, the sides of the image cropped
        let (src, dst) = Fit::Cover.rects(natural, square);
        assert_eq!(src, Rect::new(200.0, 0.0, 1400.0, 1200.0));
        assert_eq!(dst, Rect::from_wh(800.0, 800.0));

        // none: natural size, centre crop
        let (src, dst) = Fit::None.rects(natural, square);
        assert_eq!(src, Rect::new(400.0, 200.0, 1200.0, 1000.0));
        assert_eq!(dst, Rect::from_wh(800.0, 800.0));
        // none with a box bigger than the image: centred, uncropped
        let (src, dst) = Fit::None.rects((100.0, 50.0), (300.0, 300.0));
        assert_eq!(src, Rect::from_wh(100.0, 50.0));
        assert_eq!(dst, Rect::new(100.0, 125.0, 200.0, 175.0));
    }
}
