//! Layer cache: render an expensive subtree once, reuse the pixels while it
//! does not change.
//!
//! Two decisions are kept strictly apart:
//!
//! * **Policy** — *which* nodes are rendered through an offscreen layer — is
//!   decided once from the scene alone ([`plan_isolation`]). It never depends
//!   on timing, thread count or what happens to be cached, so the output is
//!   identical no matter how the cache behaves.
//! * **Reuse** — *whether* a stored layer can be used — is an exact value
//!   comparison: same evaluated node, same device transform. There is no
//!   invalidation logic to get wrong.
//!
//! A cached layer holds device-space pixels and is only ever composited at
//! the integer position it was rendered for. It is never scaled, rotated or
//! resampled, so caching cannot soften anything.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nova_core::{BlendMode, EvalNode, Fill, NodeId, NodeKind, Scene};
use skia_safe::Image;

/// Pixels of one isolated node, positioned in device space.
pub struct CachedLayer {
    pub image: Image,
    pub x: i32,
    pub y: i32,
}

impl CachedLayer {
    fn bytes(&self) -> u64 {
        self.image.width() as u64 * self.image.height() as u64 * 4
    }
}

pub struct Entry {
    node: EvalNode,
    ctm: [f32; 6],
    /// `None` when the node draws nothing visible on screen.
    pub layer: Option<CachedLayer>,
}

pub struct LayerCache {
    isolate: Vec<bool>,
    /// Last frame (exclusive) each node can appear in.
    ends: Vec<i64>,
    slots: Vec<Mutex<Option<Arc<Entry>>>>,
    bytes: AtomicU64,
    budget: u64,
}

impl LayerCache {
    pub fn new(scene: &Scene, enabled: bool, budget: u64) -> LayerCache {
        let isolate = if enabled {
            plan_isolation(scene)
        } else {
            vec![false; scene.nodes.len()]
        };
        LayerCache {
            ends: scene.nodes.iter().map(|n| n.end).collect(),
            slots: (0..scene.nodes.len()).map(|_| Mutex::new(None)).collect(),
            isolate,
            bytes: AtomicU64::new(0),
            budget,
        }
    }

    pub fn is_isolated(&self, id: NodeId) -> bool {
        self.isolate[id as usize]
    }

    pub fn isolated_count(&self) -> usize {
        self.isolate.iter().filter(|b| **b).count()
    }

    fn slot(&self, id: NodeId) -> std::sync::MutexGuard<'_, Option<Arc<Entry>>> {
        self.slots[id as usize]
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// The stored layer, if it was rendered from exactly this node value at
    /// exactly this transform.
    pub fn get(&self, id: NodeId, node: &EvalNode, ctm: &[f32; 6]) -> Option<Arc<Entry>> {
        let slot = self.slot(id);
        slot.as_ref()
            .filter(|e| e.ctm == *ctm && e.node == *node)
            .cloned()
    }

    /// Remember a freshly rendered layer (replacing the node's previous one)
    /// unless that would exceed the memory budget. Returns it either way.
    pub fn put(
        &self,
        id: NodeId,
        node: &EvalNode,
        ctm: [f32; 6],
        layer: Option<CachedLayer>,
    ) -> Arc<Entry> {
        let new_bytes = layer.as_ref().map_or(0, CachedLayer::bytes);
        let entry = Arc::new(Entry {
            node: node.clone(),
            ctm,
            layer,
        });
        let mut slot = self.slot(id);
        let old_bytes = slot
            .as_ref()
            .and_then(|e| e.layer.as_ref())
            .map_or(0, CachedLayer::bytes);
        let without_old = self.bytes.load(Ordering::Relaxed).saturating_sub(old_bytes);
        if without_old + new_bytes <= self.budget {
            *slot = Some(Arc::clone(&entry));
            self.bytes.fetch_add(new_bytes, Ordering::Relaxed);
            self.bytes.fetch_sub(old_bytes, Ordering::Relaxed);
        } else if slot.take().is_some() {
            self.bytes.fetch_sub(old_bytes, Ordering::Relaxed);
        }
        entry
    }

    /// Drop layers of nodes that end before `frame`: rendering only moves
    /// forward, so they can never be needed again.
    pub fn release_before(&self, frame: i64) {
        for (id, end) in self.ends.iter().enumerate() {
            if *end <= frame && self.isolate[id] {
                let mut slot = self.slots[id].lock().unwrap_or_else(|p| p.into_inner());
                if let Some(old) = slot.take() {
                    self.bytes.fetch_sub(
                        old.layer.as_ref().map_or(0, CachedLayer::bytes),
                        Ordering::Relaxed,
                    );
                }
            }
        }
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
}

/// What the policy needs to know about a node together with its descendants.
#[derive(Clone, Default)]
struct Subtree {
    /// Nothing in the subtree is animated.
    is_static: bool,
    /// Contains something costly to rasterise: a gradient, an image or a filter.
    expensive: bool,
    /// A strict descendant uses a non-normal blend mode. Such a subtree must
    /// keep blending with what is behind it, so it cannot be flattened.
    blends_below: bool,
    /// Composition-frame intervals in which anything in the subtree changes.
    spans: Vec<(f64, f64)>,
}

fn analyse(scene: &Scene, id: NodeId, out: &mut [Subtree]) {
    let node = scene.node(id);
    let mut sub = Subtree {
        spans: node.animated_spans(false),
        ..Subtree::default()
    };
    let gradient =
        |f: &Option<Fill>| matches!(f, Some(Fill::Linear { .. }) | Some(Fill::Radial { .. }));
    sub.expensive = node.has_filter()
        || match &node.kind {
            NodeKind::Image { .. } => true,
            NodeKind::Rect { fill, .. }
            | NodeKind::Ellipse { fill, .. }
            | NodeKind::Path { fill, .. } => gradient(fill),
            _ => false,
        };
    if let NodeKind::Group { children, .. } = &node.kind {
        for &child in children {
            analyse(scene, child, out);
            let c = &out[child as usize];
            sub.expensive |= c.expensive;
            sub.blends_below |= c.blends_below || scene.node(child).blend != BlendMode::Normal;
            sub.spans.extend_from_slice(&c.spans);
        }
    }
    sub.is_static = sub.spans.is_empty();
    out[id as usize] = sub;
}

/// Frames of `[start, end)` not covered by any of `spans`.
fn hold_frames(start: i64, end: i64, spans: &[(f64, f64)]) -> f64 {
    let (start, end) = (start as f64, end as f64);
    let mut clipped: Vec<(f64, f64)> = spans
        .iter()
        .map(|&(a, b)| (a.max(start), b.min(end)))
        .filter(|(a, b)| b > a)
        .collect();
    clipped.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut busy = 0.0;
    let mut cursor = start;
    for (a, b) in clipped {
        let a = a.max(cursor);
        if b > a {
            busy += b - a;
            cursor = b;
        }
    }
    (end - start) - busy
}

fn decide(
    scene: &Scene,
    id: NodeId,
    ancestors_fixed: bool,
    ancestor_moves: &[(f64, f64)],
    parent_flattened: bool,
    info: &[Subtree],
    out: &mut [bool],
) {
    let node = scene.node(id);
    let sub = &info[id as usize];

    // (1) A subtree that never changes, under ancestors that never move it:
    // one render serves its whole lifetime. Take the largest such subtree.
    let flattenable = sub.is_static && ancestors_fixed && !sub.blends_below;
    let as_static = flattenable && !parent_flattened && sub.expensive;

    // (2) A blurred or shadowed node needs an offscreen layer anyway, so
    // keeping the result is nearly free — worth it when the node sits still
    // for a meaningful part of its life. Not inside a flattened subtree:
    // that is already a single cached layer.
    let as_filter = !flattenable && !parent_flattened && node.has_filter() && {
        let mut spans = sub.spans.clone();
        spans.extend_from_slice(ancestor_moves);
        let life = (node.end - node.start) as f64;
        hold_frames(node.start, node.end, &spans) >= (life / 4.0).max(8.0)
    };
    out[id as usize] = as_static || as_filter;

    if let NodeKind::Group { children, .. } = &node.kind {
        let own_moves = node.animated_spans(true);
        let children_fixed = ancestors_fixed && own_moves.is_empty();
        let mut moves = ancestor_moves.to_vec();
        moves.extend(own_moves);
        for &child in children {
            decide(
                scene,
                child,
                children_fixed,
                &moves,
                flattenable || parent_flattened,
                info,
                out,
            );
        }
    }
}

/// Decide, from the scene alone, which nodes are rendered through a cached
/// offscreen layer.
pub fn plan_isolation(scene: &Scene) -> Vec<bool> {
    let mut info = vec![Subtree::default(); scene.nodes.len()];
    let mut out = vec![false; scene.nodes.len()];
    for &root in &scene.roots {
        analyse(scene, root, &mut info);
    }
    for &root in &scene.roots {
        decide(scene, root, true, &[], false, &info, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_frames_ignores_overlap_and_out_of_range_spans() {
        assert_eq!(hold_frames(0, 100, &[]), 100.0);
        assert_eq!(hold_frames(0, 100, &[(0.0, 20.0)]), 80.0);
        // overlapping spans are not counted twice
        assert_eq!(
            hold_frames(0, 100, &[(0.0, 20.0), (10.0, 30.0), (25.0, 26.0)]),
            70.0
        );
        // spans outside the lifetime do not matter
        assert_eq!(hold_frames(50, 100, &[(0.0, 40.0), (90.0, 400.0)]), 40.0);
        assert_eq!(hold_frames(0, 10, &[(-5.0, 50.0)]), 0.0);
    }
}
