/// Easing applied to the segment that *starts* at the keyframe carrying it.
///
/// Every variant is a pure function of normalised progress (and, for springs,
/// the segment's real duration), so any frame can be evaluated in isolation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    /// Hold the start value until the next keyframe.
    Step,
    Linear,
    InQuad,
    OutQuad,
    InOutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    /// CSS `cubic-bezier(x1, y1, x2, y2)`; x1 and x2 must be within 0..=1.
    CubicBezier {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    /// Damped harmonic oscillator from 0 to 1, solved in closed form (no
    /// simulation, so no dependence on earlier frames). Defaults match
    /// Remotion's `spring()`: stiffness 100, damping 10, mass 1.
    Spring {
        stiffness: f32,
        damping: f32,
        mass: f32,
    },
}

impl Easing {
    pub const DEFAULT_SPRING: Easing = Easing::Spring {
        stiffness: 100.0,
        damping: 10.0,
        mass: 1.0,
    };

    pub fn from_name(name: &str) -> Option<Easing> {
        Some(match name {
            "step" | "hold" => Easing::Step,
            "linear" => Easing::Linear,
            "ease_in_quad" => Easing::InQuad,
            "ease_out_quad" => Easing::OutQuad,
            "ease_in_out_quad" => Easing::InOutQuad,
            "ease_in_cubic" | "ease_in" => Easing::InCubic,
            "ease_out_cubic" | "ease_out" => Easing::OutCubic,
            "ease_in_out_cubic" | "ease_in_out" => Easing::InOutCubic,
            "spring" => Easing::DEFAULT_SPRING,
            _ => return None,
        })
    }

    pub const NAMES: &'static [&'static str] = &[
        "step",
        "linear",
        "ease_in_quad",
        "ease_out_quad",
        "ease_in_out_quad",
        "ease_in_cubic",
        "ease_out_cubic",
        "ease_in_out_cubic",
        "spring",
    ];

    /// True when the curve can leave the 0..=1 range (used to pad raster
    /// sizes computed from keyframe values).
    pub fn overshoots(&self) -> bool {
        match *self {
            Easing::Spring {
                stiffness,
                damping,
                mass,
            } => damping < 2.0 * (stiffness * mass).sqrt(),
            Easing::CubicBezier { y1, y2, .. } => {
                !(0.0..=1.0).contains(&y1) || !(0.0..=1.0).contains(&y2)
            }
            _ => false,
        }
    }

    /// Map normalised progress `u` (0..=1 across the segment) to eased
    /// progress. `segment_seconds` is the real duration of the segment and
    /// only matters for springs, which run in real time like Remotion's.
    pub fn apply(&self, u: f64, segment_seconds: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match *self {
            Easing::Step => {
                if u >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Easing::Linear => u,
            Easing::InQuad => u * u,
            Easing::OutQuad => 1.0 - (1.0 - u) * (1.0 - u),
            Easing::InOutQuad => {
                if u < 0.5 {
                    2.0 * u * u
                } else {
                    1.0 - (-2.0 * u + 2.0).powi(2) / 2.0
                }
            }
            Easing::InCubic => u * u * u,
            Easing::OutCubic => 1.0 - (1.0 - u).powi(3),
            Easing::InOutCubic => {
                if u < 0.5 {
                    4.0 * u * u * u
                } else {
                    1.0 - (-2.0 * u + 2.0).powi(3) / 2.0
                }
            }
            Easing::CubicBezier { x1, y1, x2, y2 } => {
                cubic_bezier(x1 as f64, y1 as f64, x2 as f64, y2 as f64, u)
            }
            Easing::Spring {
                stiffness,
                damping,
                mass,
            } => spring(
                stiffness as f64,
                damping as f64,
                mass as f64,
                u * segment_seconds,
            ),
        }
    }
}

/// Position of a unit spring released from 0 towards 1 with zero velocity,
/// at time `t` seconds.
pub fn spring(stiffness: f64, damping: f64, mass: f64, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    let w0 = (stiffness / mass).sqrt();
    let zeta = damping / (2.0 * (stiffness * mass).sqrt());
    if (zeta - 1.0).abs() < 1e-6 {
        // critically damped
        1.0 - (-w0 * t).exp() * (1.0 + w0 * t)
    } else if zeta < 1.0 {
        // under-damped: oscillates around 1 while decaying
        let wd = w0 * (1.0 - zeta * zeta).sqrt();
        1.0 - (-zeta * w0 * t).exp() * ((wd * t).cos() + (zeta * w0 / wd) * (wd * t).sin())
    } else {
        // over-damped: two real decay rates
        let s = (zeta * zeta - 1.0).sqrt();
        let r1 = -w0 * (zeta - s);
        let r2 = -w0 * (zeta + s);
        1.0 - (r2 * (r1 * t).exp() - r1 * (r2 * t).exp()) / (r2 - r1)
    }
}

/// CSS cubic-bezier timing function: find the curve parameter whose x equals
/// `x`, then return its y. Newton-Raphson with a bisection fallback.
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let sample_x = |t: f64| ((ax * t + bx) * t + cx) * t;
    let sample_y = |t: f64| ((ay * t + by) * t + cy) * t;
    let sample_dx = |t: f64| (3.0 * ax * t + 2.0 * bx) * t + cx;

    let mut t = x;
    for _ in 0..8 {
        let err = sample_x(t) - x;
        if err.abs() < 1e-7 {
            return sample_y(t);
        }
        let d = sample_dx(t);
        if d.abs() < 1e-7 {
            break;
        }
        t -= err / d;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    t = x;
    for _ in 0..40 {
        let v = sample_x(t);
        if (v - x).abs() < 1e-7 {
            break;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.0;
    }
    sample_y(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[Easing] = &[
        Easing::Linear,
        Easing::InQuad,
        Easing::OutQuad,
        Easing::InOutQuad,
        Easing::InCubic,
        Easing::OutCubic,
        Easing::InOutCubic,
        Easing::CubicBezier {
            x1: 0.25,
            y1: 0.1,
            x2: 0.25,
            y2: 1.0,
        },
    ];

    #[test]
    fn endpoints_are_exact() {
        for e in ALL {
            assert!(e.apply(0.0, 1.0).abs() < 1e-6, "{e:?} at 0");
            assert!((e.apply(1.0, 1.0) - 1.0).abs() < 1e-6, "{e:?} at 1");
        }
    }

    #[test]
    fn monotonic_for_standard_curves() {
        for e in ALL {
            let mut prev = 0.0;
            for i in 0..=100 {
                let v = e.apply(i as f64 / 100.0, 1.0);
                assert!(v >= prev - 1e-9, "{e:?} not monotonic at {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn known_values() {
        assert!((Easing::OutCubic.apply(0.5, 1.0) - 0.875).abs() < 1e-9);
        assert!((Easing::InOutQuad.apply(0.25, 1.0) - 0.125).abs() < 1e-9);
        // CSS `ease` at x=0.5 is ~0.8024
        let ease = Easing::CubicBezier {
            x1: 0.25,
            y1: 0.1,
            x2: 0.25,
            y2: 1.0,
        };
        assert!((ease.apply(0.5, 1.0) - 0.8024).abs() < 1e-3);
        // a bezier that is exactly linear
        let lin = Easing::CubicBezier {
            x1: 1.0 / 3.0,
            y1: 1.0 / 3.0,
            x2: 2.0 / 3.0,
            y2: 2.0 / 3.0,
        };
        for i in 0..=10 {
            let x = i as f64 / 10.0;
            assert!((lin.apply(x, 1.0) - x).abs() < 1e-5);
        }
    }

    #[test]
    fn step_holds_until_the_end() {
        assert_eq!(Easing::Step.apply(0.0, 1.0), 0.0);
        assert_eq!(Easing::Step.apply(0.999, 1.0), 0.0);
        assert_eq!(Easing::Step.apply(1.0, 1.0), 1.0);
    }

    #[test]
    fn spring_matches_numeric_integration() {
        // Integrate m x'' = -k (x - 1) - c x' with a tiny step and compare.
        for &(k, c, m) in &[
            (100.0, 10.0, 1.0),
            (200.0, 28.284_271, 1.0),
            (50.0, 40.0, 2.0),
        ] {
            let (mut x, mut v, dt) = (0.0_f64, 0.0_f64, 1e-5);
            let mut t = 0.0;
            while t < 0.6 {
                let a = (-k * (x - 1.0) - c * v) / m;
                v += a * dt;
                x += v * dt;
                t += dt;
            }
            let closed = spring(k, c, m, t);
            assert!(
                (closed - x).abs() < 2e-3,
                "k={k} c={c} m={m}: {closed} vs {x}"
            );
        }
    }

    #[test]
    fn default_spring_overshoots_then_settles() {
        let s = Easing::DEFAULT_SPRING;
        assert!(s.overshoots());
        let peak = (0..=100)
            .map(|i| s.apply(i as f64 / 100.0, 1.0))
            .fold(0.0, f64::max);
        assert!(peak > 1.05 && peak < 1.25, "peak {peak}");
        assert!((s.apply(1.0, 3.0) - 1.0).abs() < 1e-3);
        assert!(
            !Easing::Spring {
                stiffness: 100.0,
                damping: 30.0,
                mass: 1.0
            }
            .overshoots()
        );
    }
}
