//! The JSON template DSL, version 1.0.
//!
//! These types are the single source of truth for the format: serde parses
//! with them, and the published JSON Schema (`nova schema`) is generated from
//! them, so the two can never disagree. Doc comments here become the schema's
//! field descriptions.
//!
//! Unknown fields are rejected everywhere. Templates never contain code.

use schemars::JsonSchema;
use serde::Deserialize;

/// A NOVA template: one composition, its assets and its layers.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Template {
    /// DSL version. This build understands "1.0".
    pub schema_version: String,
    pub composition: Composition,
    /// Images, SVGs and fonts referenced by layers.
    #[serde(default)]
    pub assets: Vec<AssetDef>,
    /// Audio tracks mixed into the output video.
    #[serde(default)]
    pub audio: Vec<AudioDef>,
    /// Layers in paint order: the first is furthest back.
    pub layers: Vec<Layer>,
    /// Free-form data ignored by the renderer.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Output width in pixels (must be even).
    pub width: u32,
    /// Output height in pixels (must be even).
    pub height: u32,
    /// Frames per second: a whole number, or `{ "num": 30000, "den": 1001 }`.
    pub fps: FpsDef,
    /// Total length in frames.
    pub duration_frames: u32,
    /// Opaque background colour, default black.
    #[serde(default)]
    pub background: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FpsDef {
    Whole(u32),
    Rational {
        num: u32,
        #[serde(default = "one")]
        den: u32,
    },
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Image,
    Svg,
    Font,
    /// A baked frame sequence: `src` is the pack's `frames.json` manifest,
    /// its shard files lie next to it. Used by `frames` layers.
    Frames,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetDef {
    /// Unique id layers use to reference this asset.
    pub id: String,
    #[serde(rename = "type")]
    pub kind: AssetType,
    /// Path relative to the asset root (the template's folder by default).
    /// Absolute paths, `..` and URLs are rejected.
    pub src: String,
    /// Fonts only: the family name text layers use. Defaults to the asset id.
    #[serde(default)]
    pub family: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioDef {
    #[serde(default)]
    pub id: Option<String>,
    /// Audio file relative to the asset root.
    pub src: String,
    /// Composition frame at which the track starts. Default 0.
    #[serde(default)]
    pub start_frame: Option<u32>,
    /// Linear gain, 1.0 = unchanged. Default 1.0.
    #[serde(default)]
    pub volume: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayerType {
    /// A rectangle the size of the composition. Needs `color`.
    Solid,
    /// Rectangle, optionally rounded. Needs `size`.
    Rect,
    /// Ellipse inscribed in `size`.
    Ellipse,
    /// Vector path from SVG path data in `d`.
    Path,
    /// Typography. Needs `text`.
    Text,
    /// Raster image. Needs `asset`.
    Image,
    /// Static SVG file. Needs `asset`.
    Svg,
    /// A baked frame sequence, one drawing per frame. Needs `asset` (a
    /// `frames` asset). `size` defaults to the pack's own size.
    Frames,
    /// Container for child `layers`; also acts as a sequence (its
    /// `start_frame` shifts the children's time).
    Group,
}

impl LayerType {
    pub fn as_str(self) -> &'static str {
        match self {
            LayerType::Solid => "solid",
            LayerType::Rect => "rect",
            LayerType::Ellipse => "ellipse",
            LayerType::Path => "path",
            LayerType::Text => "text",
            LayerType::Image => "image",
            LayerType::Svg => "svg",
            LayerType::Frames => "frames",
            LayerType::Group => "group",
        }
    }
}

/// One layer. Common fields apply to every type; the rest are only valid for
/// the layer types named in their description.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// Unique across the whole template, including nested layers.
    pub id: String,
    #[serde(rename = "type")]
    pub kind: LayerType,
    #[serde(default)]
    pub name: Option<String>,
    /// `false` removes the layer (and its children) from the render.
    #[serde(default)]
    pub visible: Option<bool>,
    /// First frame the layer exists, in its parent's local time. Default 0.
    /// This is also the layer's local frame 0: its keyframes and children are
    /// timed relative to it.
    #[serde(default)]
    pub start_frame: Option<i64>,
    /// Frame at which the layer stops existing (exclusive), in the parent's
    /// local time. Default: the end of the parent.
    #[serde(default)]
    pub end_frame: Option<i64>,
    #[serde(default)]
    pub transform: Option<TransformDef>,
    /// 0..1. Applies to the layer as a whole, like CSS `opacity`.
    #[serde(default)]
    pub opacity: Option<f32>,
    /// Same as `transform.anchor`.
    #[serde(default)]
    pub anchor: Option<Vec2>,
    #[serde(default)]
    pub blend_mode: Option<BlendModeDef>,
    /// Gaussian blur of the whole layer; standard deviation in pixels, like
    /// CSS `filter: blur()`.
    #[serde(default)]
    pub blur: Option<f32>,
    /// Drop shadow following the layer's alpha, like CSS `drop-shadow()`.
    #[serde(default)]
    pub shadow: Option<ShadowDef>,

    /// rect, ellipse: required. image, svg, path, group: optional box.
    #[serde(default)]
    pub size: Option<SizeDef>,
    /// rect: corner radius in pixels. group: rounds the corners of `clip`.
    #[serde(default)]
    pub corner_radius: Option<f32>,
    /// rect, ellipse, path: interior paint — a colour string or a gradient.
    #[serde(default)]
    pub fill: Option<FillDef>,
    /// rect, ellipse, path: outline.
    #[serde(default)]
    pub stroke: Option<StrokeDef>,
    /// solid: the colour.
    #[serde(default)]
    pub color: Option<String>,
    /// path: SVG path data, e.g. "M0 0 L100 0 L50 80 Z".
    #[serde(default)]
    pub d: Option<String>,
    /// path: fill rule. Default nonzero.
    #[serde(default)]
    pub fill_rule: Option<FillRuleDef>,
    /// text: content and style.
    #[serde(default)]
    pub text: Option<TextDef>,
    /// image, svg, frames: id of the asset to draw.
    #[serde(default)]
    pub asset: Option<String>,
    /// image, svg: how the asset fits `size`. Default contain.
    #[serde(default)]
    pub fit: Option<FitDef>,
    /// group: child layers, positioned and timed relative to the group.
    #[serde(default)]
    pub layers: Option<Vec<Layer>>,
    /// group: clip children to `size`.
    #[serde(default)]
    pub clip: Option<bool>,

    /// Keyframe animations of this layer's properties.
    #[serde(default)]
    pub animations: Vec<AnimationDef>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SizeDef {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransformDef {
    /// Where the layer's anchor point sits, in the parent's pixels.
    /// Default (0, 0).
    #[serde(default)]
    pub position: Option<Vec2>,
    /// A number (uniform) or `{ "x": 1, "y": 1 }`. Default 1.
    #[serde(default)]
    pub scale: Option<ScaleDef>,
    /// Degrees, clockwise. Default 0.
    #[serde(default)]
    pub rotation: Option<f32>,
    /// Point of the layer's own box that `position` refers to and that
    /// rotation/scale pivot around, as a fraction of the box: (0,0) top-left,
    /// (0.5,0.5) centre. Default (0, 0).
    #[serde(default)]
    pub anchor: Option<Vec2>,
    /// Same as the layer-level `opacity`.
    #[serde(default)]
    pub opacity: Option<f32>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ScaleDef {
    Uniform(f32),
    Xy(Vec2),
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlendModeDef {
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
    Add,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FitDef {
    Fill,
    Contain,
    Cover,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FillRuleDef {
    Nonzero,
    Evenodd,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FillDef {
    /// Colour string: "#RRGGBB", "#RRGGBBAA", "rgba(r,g,b,a)".
    Color(String),
    Gradient(GradientDef),
}

/// Gradient geometry is in units of the layer's box: (0,0) is its top-left
/// corner and (1,1) its bottom-right.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GradientDef {
    Linear {
        /// Start point. Default [0, 0].
        #[serde(default)]
        from: Option<[f32; 2]>,
        /// End point. Default [1, 0] (left to right).
        #[serde(default)]
        to: Option<[f32; 2]>,
        stops: Vec<StopDef>,
    },
    Radial {
        /// Default [0.5, 0.5].
        #[serde(default)]
        center: Option<[f32; 2]>,
        /// In units of the box width. Default 0.5.
        #[serde(default)]
        radius: Option<f32>,
        stops: Vec<StopDef>,
    },
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StopDef {
    /// 0..1 along the gradient.
    pub offset: f32,
    pub color: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineCapDef {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineJoinDef {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrokeDef {
    pub color: String,
    /// Line width in pixels, centred on the outline.
    pub width: f32,
    #[serde(default)]
    pub cap: Option<LineCapDef>,
    #[serde(default)]
    pub join: Option<LineJoinDef>,
    /// Start of the visible part of the outline, 0..1. Default 0.
    #[serde(default)]
    pub trim_start: Option<f32>,
    /// End of the visible part of the outline, 0..1. Default 1. Animate it
    /// from 0 to 1 for a "draw-on" effect.
    #[serde(default)]
    pub trim_end: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShadowDef {
    /// Horizontal offset in pixels. Default 0.
    #[serde(default)]
    pub x: Option<f32>,
    /// Vertical offset in pixels. Default 0.
    #[serde(default)]
    pub y: Option<f32>,
    /// Blur radius with CSS semantics (the Gaussian sigma is half of it).
    #[serde(default)]
    pub blur: Option<f32>,
    pub color: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FontFamilyDef {
    One(String),
    /// Preferred family first, then fallbacks.
    Many(Vec<String>),
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignDef {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextDef {
    /// The text. "\n" starts a new line.
    pub value: String,
    /// Family name of a font asset (or a system font), or a list with fallbacks.
    pub font_family: FontFamilyDef,
    /// Pixels.
    pub font_size: f32,
    /// 100..900. Default 400.
    #[serde(default)]
    pub font_weight: Option<u16>,
    #[serde(default)]
    pub italic: Option<bool>,
    /// Default white.
    #[serde(default)]
    pub color: Option<String>,
    /// Line height as a multiple of the font size (CSS unitless line-height).
    #[serde(default)]
    pub line_height: Option<f32>,
    /// Extra space between letters in pixels.
    #[serde(default)]
    pub letter_spacing: Option<f32>,
    /// Default left. Centre/right align within `max_width` when it is set.
    #[serde(default)]
    pub align: Option<TextAlignDef>,
    /// Wrap width in pixels. Without it the text only breaks at "\n".
    #[serde(default)]
    pub max_width: Option<f32>,
    /// Truncate with an ellipsis after this many lines.
    #[serde(default)]
    pub max_lines: Option<u32>,
    /// Shrink the layer's box to the text when it fits within `max_width`
    /// without wrapping (text that wraps keeps the full `max_width`), like a
    /// CSS shrink-to-fit box. Use it with `background` so the panel hugs
    /// short captions.
    #[serde(default)]
    pub hug: Option<bool>,
    /// Parts of the text in their own colour.
    #[serde(default)]
    pub spans: Vec<SpanDef>,
    /// Show the text progressively: karaoke captions, typewriter.
    #[serde(default)]
    pub reveal: Option<RevealDef>,
    /// Panel behind the text (caption box, tag, stamp).
    #[serde(default)]
    pub background: Option<TextBoxDef>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpanDef {
    /// Index of the first character (0-based, counted in characters).
    pub start: u32,
    /// Index one past the last character.
    pub end: u32,
    pub color: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevealUnitDef {
    Word,
    Char,
}

/// Only the first `count` words (or characters) are painted in their real
/// colour; the rest use `hidden_color`. Animate `text.reveal` to move the
/// boundary. Nothing shifts while text is revealed.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevealDef {
    /// Default word.
    #[serde(default)]
    pub unit: Option<RevealUnitDef>,
    /// Units revealed. Fractions are rounded down.
    pub count: f32,
    /// Colour of the part not yet revealed. Default transparent.
    #[serde(default)]
    pub hidden_color: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextBoxDef {
    #[serde(default)]
    pub color: Option<String>,
    /// Corner radius. Default 0.
    #[serde(default)]
    pub radius: Option<f32>,
    /// Space between the text and the panel's edge: a number, `{x, y}` or
    /// `{left, top, right, bottom}`. It is part of the layer's box.
    #[serde(default)]
    pub padding: Option<PaddingDef>,
    #[serde(default)]
    pub border_color: Option<String>,
    #[serde(default)]
    pub border_width: Option<f32>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PaddingDef {
    All(f32),
    Axes(PaddingAxes),
    Sides(PaddingSides),
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaddingAxes {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaddingSides {
    #[serde(default)]
    pub left: f32,
    #[serde(default)]
    pub top: f32,
    #[serde(default)]
    pub right: f32,
    #[serde(default)]
    pub bottom: f32,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnimationDef {
    /// Property to animate, e.g. "transform.position.y", "transform.scale",
    /// "transform.rotation", "opacity", "fill.color", "stroke.trim_end",
    /// "size.width", "corner_radius", "text.font_size", "text.reveal", "blur",
    /// "shadow.blur".
    pub property: String,
    /// At least one keyframe, in increasing frame order.
    pub keyframes: Vec<KeyframeDef>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframeDef {
    /// Frame relative to the layer's `start_frame`.
    pub frame: f64,
    /// A number, or a colour string for colour properties.
    pub value: KeyValue,
    /// Easing from this keyframe to the next. Default linear.
    #[serde(default)]
    pub easing: Option<EasingDef>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum KeyValue {
    Number(f64),
    Text(String),
}

/// A named easing ("linear", "step", "ease_in_quad", "ease_out_quad",
/// "ease_in_out_quad", "ease_in_cubic", "ease_out_cubic", "ease_in_out_cubic",
/// "spring"), `{ "cubic_bezier": [x1, y1, x2, y2] }`, or
/// `{ "spring": { "stiffness": 100, "damping": 10, "mass": 1 } }`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EasingDef {
    Name(String),
    Bezier { cubic_bezier: [f32; 4] },
    Spring { spring: SpringDef },
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpringDef {
    /// Default 100.
    #[serde(default)]
    pub stiffness: Option<f32>,
    /// Default 10.
    #[serde(default)]
    pub damping: Option<f32>,
    /// Default 1.
    #[serde(default)]
    pub mass: Option<f32>,
}
