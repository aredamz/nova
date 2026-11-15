//! NOVA core.
//!
//! Everything in this crate is backend-neutral and free of I/O: a compiled,
//! immutable [`Scene`] plus a pure function `Scene::evaluate(frame)` that turns
//! it into a [`FramePlan`]. A raster backend only ever sees frame plans.

pub mod anim;
pub mod color;
pub mod easing;
pub mod error;
pub mod eval;
pub mod geometry;
pub mod limits;
pub mod scene;
pub mod time;

pub use anim::{Key, Lerp, Prop, Track};
pub use color::Color;
pub use easing::Easing;
pub use error::{Error, ErrorCode, Result};
pub use eval::{EvalFill, EvalKind, EvalNode, EvalShadow, EvalStroke, FramePlan};
pub use geometry::{Affine, Rect};
pub use limits::Limits;
pub use scene::{
    Asset, AssetId, AssetKind, AudioTrack, BlendMode, Fill, Fit, GradientStop, LineCap, LineJoin,
    Node, NodeId, NodeKind, Reveal, RevealUnit, Scene, Shadow, Stroke, TextAlign, TextBox,
    TextNode, TextSpan,
};
pub use time::Fps;
