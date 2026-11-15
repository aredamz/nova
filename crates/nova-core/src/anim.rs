use crate::easing::Easing;

/// Linear interpolation. `t` is eased progress and may leave 0..=1 when the
/// easing overshoots.
pub trait Lerp: Clone {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(a: &f32, b: &f32, t: f32) -> f32 {
        a + (b - a) * t
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Key<T> {
    /// Frame in the layer's local time (0 = the layer's `start_frame`).
    pub frame: f64,
    pub value: T,
    /// Easing of the segment from this key to the next one.
    pub easing: Easing,
}

/// Keyframes sorted by strictly increasing frame; never empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Track<T> {
    keys: Vec<Key<T>>,
}

impl<T: Lerp> Track<T> {
    /// Returns `None` unless there is at least one key and frames strictly increase.
    pub fn new(keys: Vec<Key<T>>) -> Option<Track<T>> {
        if keys.is_empty() || keys.iter().any(|k| !k.frame.is_finite()) {
            return None;
        }
        if keys.windows(2).any(|w| w[1].frame <= w[0].frame) {
            return None;
        }
        Some(Track { keys })
    }

    pub fn keys(&self) -> &[Key<T>] {
        &self.keys
    }

    /// Value at `local_frame`. Before the first key the first value holds,
    /// after the last key the last value holds.
    pub fn eval(&self, local_frame: f64, fps: f64) -> T {
        let keys = &self.keys;
        let first = &keys[0];
        if local_frame <= first.frame {
            return first.value.clone();
        }
        let last = &keys[keys.len() - 1];
        if local_frame >= last.frame {
            return last.value.clone();
        }
        // first key whose frame is > local_frame; the segment starts one before it
        let hi = keys.partition_point(|k| k.frame <= local_frame);
        let (a, b) = (&keys[hi - 1], &keys[hi]);
        let span = b.frame - a.frame;
        let u = (local_frame - a.frame) / span;
        let t = a.easing.apply(u, span / fps);
        T::lerp(&a.value, &b.value, t as f32)
    }
}

/// A property that is either constant for the whole render or keyframed.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop<T> {
    Const(T),
    Anim(Track<T>),
}

impl<T: Lerp> Prop<T> {
    pub fn eval(&self, local_frame: f64, fps: f64) -> T {
        match self {
            Prop::Const(v) => v.clone(),
            Prop::Anim(track) => track.eval(local_frame, fps),
        }
    }

    pub fn is_const(&self) -> bool {
        matches!(self, Prop::Const(_))
    }

    /// Local frames between which the value can change: first to last key.
    /// `None` for constants and single-key tracks, which never change.
    pub fn key_span(&self) -> Option<(f64, f64)> {
        match self {
            Prop::Anim(track) if track.keys().len() > 1 => {
                let keys = track.keys();
                Some((keys[0].frame, keys[keys.len() - 1].frame))
            }
            _ => None,
        }
    }

    /// The value at local frame 0 when constant, otherwise `None`.
    pub fn as_const(&self) -> Option<&T> {
        match self {
            Prop::Const(v) => Some(v),
            Prop::Anim(_) => None,
        }
    }
}

impl Prop<f32> {
    /// Upper bound of `|value|` over all time. Overshooting easings are padded,
    /// since a spring briefly exceeds its target.
    pub fn max_abs(&self) -> f32 {
        match self {
            Prop::Const(v) => v.abs(),
            Prop::Anim(track) => {
                let peak = track
                    .keys()
                    .iter()
                    .map(|k| k.value.abs())
                    .fold(0.0, f32::max);
                if track.keys().iter().any(|k| k.easing.overshoots()) {
                    peak * 1.35
                } else {
                    peak
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(keys: &[(f64, f32, Easing)]) -> Track<f32> {
        Track::new(
            keys.iter()
                .map(|&(frame, value, easing)| Key {
                    frame,
                    value,
                    easing,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn rejects_unsorted_or_empty() {
        assert!(Track::<f32>::new(vec![]).is_none());
        let dup = vec![
            Key {
                frame: 5.0,
                value: 0.0,
                easing: Easing::Linear,
            },
            Key {
                frame: 5.0,
                value: 1.0,
                easing: Easing::Linear,
            },
        ];
        assert!(Track::new(dup).is_none());
    }

    #[test]
    fn holds_outside_the_keyed_range() {
        let t = track(&[(10.0, 100.0, Easing::Linear), (20.0, 200.0, Easing::Linear)]);
        assert_eq!(t.eval(-5.0, 30.0), 100.0);
        assert_eq!(t.eval(10.0, 30.0), 100.0);
        assert_eq!(t.eval(20.0, 30.0), 200.0);
        assert_eq!(t.eval(999.0, 30.0), 200.0);
    }

    #[test]
    fn interpolates_and_uses_the_starting_keys_easing() {
        let t = track(&[
            (0.0, 340.0, Easing::OutCubic),
            (18.0, 250.0, Easing::Linear),
            (28.0, 150.0, Easing::Linear),
        ]);
        // out-cubic at u=0.5 is 0.875 -> 340 - 90*0.875
        assert!((t.eval(9.0, 30.0) - 261.25).abs() < 1e-3);
        // second segment is linear
        assert!((t.eval(23.0, 30.0) - 200.0).abs() < 1e-3);
    }

    #[test]
    fn step_easing_jumps_at_the_next_key() {
        let t = track(&[(0.0, 1.0, Easing::Step), (10.0, 5.0, Easing::Linear)]);
        assert_eq!(t.eval(9.0, 30.0), 1.0);
        assert_eq!(t.eval(10.0, 30.0), 5.0);
    }

    #[test]
    fn random_access_equals_sequential() {
        let t = track(&[
            (0.0, 0.0, Easing::DEFAULT_SPRING),
            (45.0, 1.0, Easing::InOutCubic),
            (90.0, 0.25, Easing::Linear),
        ]);
        let forward: Vec<f32> = (0..120).map(|f| t.eval(f as f64, 30.0)).collect();
        let mut order: Vec<usize> = (0..120).collect();
        order.reverse();
        for f in order {
            assert_eq!(t.eval(f as f64, 30.0), forward[f]);
        }
    }

    #[test]
    fn key_span_covers_first_to_last_key() {
        let p = Prop::Anim(track(&[
            (4.0, 0.0, Easing::Linear),
            (10.0, 1.0, Easing::Linear),
            (30.0, 0.0, Easing::Linear),
        ]));
        assert_eq!(p.key_span(), Some((4.0, 30.0)));
        assert_eq!(Prop::Const(1.0_f32).key_span(), None);
        assert_eq!(
            Prop::Anim(track(&[(7.0, 2.0, Easing::Linear)])).key_span(),
            None,
            "one key never changes"
        );
    }

    #[test]
    fn max_abs_pads_overshoot() {
        let plain = Prop::Anim(track(&[
            (0.0, 1.0, Easing::Linear),
            (10.0, -3.0, Easing::Linear),
        ]));
        assert_eq!(plain.max_abs(), 3.0);
        let springy = Prop::Anim(track(&[
            (0.0, 0.0, Easing::DEFAULT_SPRING),
            (10.0, 2.0, Easing::Linear),
        ]));
        assert!(springy.max_abs() > 2.0);
        assert_eq!(Prop::Const(-4.0_f32).max_abs(), 4.0);
    }
}
