//! Per-frame evaluation: `Scene` + frame index -> `FramePlan`.
//!
//! A frame plan is plain data holding every value that can differ between two
//! frames. It is the only thing a raster backend needs besides the scene, and
//! because it derives `PartialEq`, "these two frames are pixel-identical" is
//! an exact comparison rather than dependency tracking: equal plans always
//! render to equal pixels.

use crate::color::Color;
use crate::scene::{Fill, Node, NodeId, NodeKind, Scene, Stroke};

/// Everything visible in one frame, in paint order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FramePlan {
    pub nodes: Vec<EvalNode>,
}

impl FramePlan {
    /// Number of drawable nodes including nested ones.
    pub fn node_count(&self) -> usize {
        fn count(nodes: &[EvalNode]) -> usize {
            nodes
                .iter()
                .map(|n| match &n.kind {
                    EvalKind::Group { children } => 1 + count(children),
                    _ => 1,
                })
                .sum()
        }
        count(&self.nodes)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvalNode {
    pub node: NodeId,
    pub position: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
    pub anchor: [f32; 2],
    /// 0 < opacity <= 1 (fully transparent nodes are not in the plan).
    pub opacity: f32,
    pub blur: f32,
    pub shadow: Option<EvalShadow>,
    pub kind: EvalKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvalShadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EvalFill {
    Solid(Color),
    /// Static gradient; read its definition from the scene node.
    Gradient,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvalStroke {
    pub color: Color,
    pub width: f32,
    pub trim_start: f32,
    pub trim_end: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvalKind {
    Solid {
        color: Color,
    },
    Rect {
        width: f32,
        height: f32,
        radius: f32,
        fill: Option<EvalFill>,
        stroke: Option<EvalStroke>,
    },
    Ellipse {
        width: f32,
        height: f32,
        fill: Option<EvalFill>,
        stroke: Option<EvalStroke>,
    },
    Path {
        fill: Option<EvalFill>,
        stroke: Option<EvalStroke>,
    },
    Text {
        size: f32,
        color: Color,
        letter_spacing: f32,
        /// Whole units revealed so far, when the text has a reveal.
        reveal: Option<u32>,
    },
    Image,
    /// Baked frame to draw, as its content id in the pack.
    Frames {
        content: u32,
    },
    Group {
        children: Vec<EvalNode>,
    },
}

impl Scene {
    /// Evaluate the composition at `frame`. Pure and random-access: frame 450
    /// never needs frame 449.
    pub fn evaluate(&self, frame: u32) -> FramePlan {
        let frame = frame as i64;
        let fps = self.fps.as_f64();
        let mut nodes = Vec::with_capacity(self.roots.len());
        for &id in &self.roots {
            if let Some(n) = self.eval_node(id, frame, fps) {
                nodes.push(n);
            }
        }
        FramePlan { nodes }
    }

    fn eval_node(&self, id: NodeId, frame: i64, fps: f64) -> Option<EvalNode> {
        let node: &Node = self.node(id);
        if !node.is_active(frame) {
            return None;
        }
        let t = (frame - node.origin) as f64;

        let opacity = node.opacity.eval(t, fps).clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return None;
        }
        let scale = [node.scale_x.eval(t, fps), node.scale_y.eval(t, fps)];
        if scale[0] == 0.0 || scale[1] == 0.0 {
            return None;
        }

        let fill = |f: &Option<Fill>| -> Option<EvalFill> {
            match f.as_ref()? {
                Fill::Solid(c) => {
                    let c = c.eval(t, fps);
                    (c.a > 0.0).then_some(EvalFill::Solid(c))
                }
                Fill::Linear { .. } | Fill::Radial { .. } => Some(EvalFill::Gradient),
            }
        };
        let stroke = |s: &Option<Stroke>| -> Option<EvalStroke> {
            let s = s.as_ref()?;
            let out = EvalStroke {
                color: s.color.eval(t, fps),
                width: s.width.eval(t, fps),
                trim_start: s.trim_start.eval(t, fps).clamp(0.0, 1.0),
                trim_end: s.trim_end.eval(t, fps).clamp(0.0, 1.0),
            };
            (out.width > 0.0 && out.color.a > 0.0 && out.trim_end > out.trim_start).then_some(out)
        };

        let kind = match &node.kind {
            NodeKind::Solid { color } => {
                let color = color.eval(t, fps);
                if color.a <= 0.0 {
                    return None;
                }
                EvalKind::Solid { color }
            }
            NodeKind::Rect {
                width,
                height,
                radius,
                fill: f,
                stroke: s,
            } => {
                let (w, h) = (width.eval(t, fps), height.eval(t, fps));
                let (f, s) = (fill(f), stroke(s));
                if w <= 0.0 || h <= 0.0 || (f.is_none() && s.is_none()) {
                    return None;
                }
                let radius = radius.eval(t, fps).clamp(0.0, w.min(h) / 2.0);
                EvalKind::Rect {
                    width: w,
                    height: h,
                    radius,
                    fill: f,
                    stroke: s,
                }
            }
            NodeKind::Ellipse {
                width,
                height,
                fill: f,
                stroke: s,
            } => {
                let (w, h) = (width.eval(t, fps), height.eval(t, fps));
                let (f, s) = (fill(f), stroke(s));
                if w <= 0.0 || h <= 0.0 || (f.is_none() && s.is_none()) {
                    return None;
                }
                EvalKind::Ellipse {
                    width: w,
                    height: h,
                    fill: f,
                    stroke: s,
                }
            }
            NodeKind::Path {
                fill: f, stroke: s, ..
            } => {
                let (f, s) = (fill(f), stroke(s));
                if f.is_none() && s.is_none() {
                    return None;
                }
                EvalKind::Path { fill: f, stroke: s }
            }
            NodeKind::Text(text) => {
                let size = text.size.eval(t, fps);
                let color = text.color.eval(t, fps);
                // Spans and a background can show even when the base colour is clear.
                let has_other_ink = !text.spans.is_empty() || text.background.is_some();
                if size <= 0.0 || text.value.is_empty() || (color.a <= 0.0 && !has_other_ink) {
                    return None;
                }
                EvalKind::Text {
                    size,
                    color,
                    letter_spacing: text.letter_spacing.eval(t, fps),
                    // Whole units only: frames between two steps stay identical.
                    reveal: text
                        .reveal
                        .as_ref()
                        .map(|r| r.count.eval(t, fps).floor().max(0.0) as u32),
                }
            }
            NodeKind::Image { .. } => EvalKind::Image,
            NodeKind::Frames { asset, frames, .. } => {
                let pack = self.asset(*asset).pack.as_ref()?;
                let local = (frame - node.origin).clamp(0, (*frames as i64 - 1).max(0)) as usize;
                EvalKind::Frames {
                    content: pack.index.get(local)?.content,
                }
            }
            NodeKind::Group { children, .. } => {
                let children: Vec<EvalNode> = children
                    .iter()
                    .filter_map(|&c| self.eval_node(c, frame, fps))
                    .collect();
                if children.is_empty() {
                    return None;
                }
                EvalKind::Group { children }
            }
        };

        let shadow = node.shadow.as_ref().and_then(|s| {
            let out = EvalShadow {
                x: s.x.eval(t, fps),
                y: s.y.eval(t, fps),
                blur: s.blur.eval(t, fps).max(0.0),
                color: s.color.eval(t, fps),
            };
            (out.color.a > 0.0).then_some(out)
        });

        Some(EvalNode {
            node: id,
            position: [node.pos_x.eval(t, fps), node.pos_y.eval(t, fps)],
            scale,
            rotation: node.rotation.eval(t, fps),
            anchor: [node.anchor_x.eval(t, fps), node.anchor_y.eval(t, fps)],
            opacity,
            blur: node.blur.eval(t, fps).max(0.0),
            shadow,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{Key, Prop, Track};
    use crate::easing::Easing;
    use crate::scene::BlendMode;
    use crate::time::Fps;

    fn base_node(id: &str, start: i64, end: i64, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            start,
            end,
            origin: start,
            pos_x: Prop::Const(0.0),
            pos_y: Prop::Const(0.0),
            scale_x: Prop::Const(1.0),
            scale_y: Prop::Const(1.0),
            rotation: Prop::Const(0.0),
            anchor_x: Prop::Const(0.0),
            anchor_y: Prop::Const(0.0),
            opacity: Prop::Const(1.0),
            blur: Prop::Const(0.0),
            shadow: None,
            blend: BlendMode::Normal,
            kind,
        }
    }

    fn rect() -> NodeKind {
        NodeKind::Rect {
            width: Prop::Const(100.0),
            height: Prop::Const(50.0),
            radius: Prop::Const(0.0),
            fill: Some(Fill::Solid(Prop::Const(Color::WHITE))),
            stroke: None,
        }
    }

    fn scene(nodes: Vec<Node>, roots: Vec<NodeId>) -> Scene {
        Scene {
            id: "t".into(),
            name: None,
            width: 1920,
            height: 1080,
            fps: Fps::new(30, 1).unwrap(),
            duration_frames: 300,
            background: Color::BLACK,
            nodes,
            roots,
            assets: vec![],
            audio: vec![],
        }
    }

    fn ramp(from: f32, to: f32, frames: f64) -> Prop<f32> {
        Prop::Anim(
            Track::new(vec![
                Key {
                    frame: 0.0,
                    value: from,
                    easing: Easing::Linear,
                },
                Key {
                    frame: frames,
                    value: to,
                    easing: Easing::Linear,
                },
            ])
            .unwrap(),
        )
    }

    #[test]
    fn active_range_is_start_inclusive_end_exclusive() {
        let s = scene(vec![base_node("a", 10, 20, rect())], vec![0]);
        assert!(s.evaluate(9).nodes.is_empty());
        assert_eq!(s.evaluate(10).nodes.len(), 1);
        assert_eq!(s.evaluate(19).nodes.len(), 1);
        assert!(s.evaluate(20).nodes.is_empty());
    }

    #[test]
    fn keyframes_run_in_local_time() {
        let mut n = base_node("a", 100, 200, rect());
        n.pos_x = ramp(0.0, 300.0, 30.0);
        let s = scene(vec![n], vec![0]);
        assert_eq!(s.evaluate(100).nodes[0].position[0], 0.0);
        assert_eq!(s.evaluate(115).nodes[0].position[0], 150.0);
        assert_eq!(s.evaluate(190).nodes[0].position[0], 300.0);
    }

    #[test]
    fn invisible_nodes_are_dropped() {
        let mut faded = base_node("a", 0, 100, rect());
        faded.opacity = ramp(0.0, 1.0, 10.0);
        let mut collapsed = base_node("b", 0, 100, rect());
        collapsed.scale_x = Prop::Const(0.0);
        let s = scene(vec![faded, collapsed], vec![0, 1]);
        assert!(s.evaluate(0).nodes.is_empty());
        assert_eq!(s.evaluate(5).nodes.len(), 1);
        assert_eq!(s.evaluate(5).nodes[0].opacity, 0.5);
    }

    #[test]
    fn groups_nest_and_empty_groups_vanish() {
        let child = base_node("c", 50, 60, rect());
        let group = base_node(
            "g",
            0,
            100,
            NodeKind::Group {
                children: vec![0],
                size: None,
                clip: false,
                radius: 0.0,
            },
        );
        let s = scene(vec![child, group], vec![1]);
        assert!(
            s.evaluate(10).nodes.is_empty(),
            "group with no active child draws nothing"
        );
        let plan = s.evaluate(55);
        assert_eq!(plan.node_count(), 2);
        match &plan.nodes[0].kind {
            EvalKind::Group { children } => assert_eq!(children[0].node, 0),
            other => panic!("expected group, got {other:?}"),
        }
    }

    #[test]
    fn held_frames_produce_equal_plans() {
        let mut n = base_node("a", 0, 300, rect());
        n.pos_x = ramp(0.0, 100.0, 30.0);
        let s = scene(vec![n], vec![0]);
        assert_ne!(s.evaluate(10), s.evaluate(11), "still animating");
        assert_eq!(
            s.evaluate(30),
            s.evaluate(31),
            "animation finished: identical frames"
        );
        assert_eq!(s.evaluate(31), s.evaluate(299));
    }

    #[test]
    fn evaluation_is_random_access() {
        let mut n = base_node("a", 0, 300, rect());
        n.rotation = Prop::Anim(
            Track::new(vec![
                Key {
                    frame: 0.0,
                    value: 0.0,
                    easing: Easing::DEFAULT_SPRING,
                },
                Key {
                    frame: 60.0,
                    value: 360.0,
                    easing: Easing::Linear,
                },
            ])
            .unwrap(),
        );
        let s = scene(vec![n], vec![0]);
        let forward: Vec<FramePlan> = (0..120).map(|f| s.evaluate(f)).collect();
        for f in (0..120).rev().step_by(7) {
            assert_eq!(s.evaluate(f), forward[f as usize]);
        }
    }

    #[test]
    fn fully_trimmed_stroke_is_not_drawn() {
        let kind = NodeKind::Ellipse {
            width: Prop::Const(10.0),
            height: Prop::Const(10.0),
            fill: None,
            stroke: Some(Stroke {
                color: Prop::Const(Color::WHITE),
                width: Prop::Const(4.0),
                cap: Default::default(),
                join: Default::default(),
                trim_start: Prop::Const(0.0),
                trim_end: ramp(0.0, 1.0, 10.0),
            }),
        };
        let s = scene(vec![base_node("a", 0, 100, kind)], vec![0]);
        assert!(s.evaluate(0).nodes.is_empty());
        assert_eq!(s.evaluate(5).nodes.len(), 1);
    }
}
