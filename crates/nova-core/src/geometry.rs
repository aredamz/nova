/// Axis-aligned rectangle, `left <= right` and `top <= bottom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn from_wh(w: f32, h: f32) -> Rect {
        Rect {
            left: 0.0,
            top: 0.0,
            right: w,
            bottom: h,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub fn inflate(&self, d: f32) -> Rect {
        Rect::new(self.left - d, self.top - d, self.right + d, self.bottom + d)
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect::new(
            self.left.min(o.left),
            self.top.min(o.top),
            self.right.max(o.right),
            self.bottom.max(o.bottom),
        )
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let r = Rect::new(
            self.left.max(o.left),
            self.top.max(o.top),
            self.right.min(o.right),
            self.bottom.min(o.bottom),
        );
        if r.is_empty() {
            Rect::new(0.0, 0.0, 0.0, 0.0)
        } else {
            r
        }
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
}

/// 2D affine transform mapping `(x, y)` to
/// `(a*x + c*y + tx, b*x + d*y + ty)` — the same layout as a CSS `matrix()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub fn translate(tx: f32, ty: f32) -> Affine {
        Affine {
            tx,
            ty,
            ..Affine::IDENTITY
        }
    }

    /// Layer transform.
    ///
    /// `position` is where the layer's anchor point lands in parent space;
    /// the layer is scaled, then rotated clockwise (y points down) around that
    /// anchor: `T(position) · R(rotation) · S(scale) · T(-anchor_px)`.
    pub fn layer(
        position: [f32; 2],
        rotation_deg: f32,
        scale: [f32; 2],
        anchor_px: [f32; 2],
    ) -> Affine {
        let (sin, cos) = if rotation_deg == 0.0 {
            (0.0, 1.0)
        } else {
            rotation_deg.to_radians().sin_cos()
        };
        let (a, b) = (cos * scale[0], sin * scale[0]);
        let (c, d) = (-sin * scale[1], cos * scale[1]);
        Affine {
            a,
            b,
            c,
            d,
            tx: position[0] - (a * anchor_px[0] + c * anchor_px[1]),
            ty: position[1] - (b * anchor_px[0] + d * anchor_px[1]),
        }
    }

    /// `self · rhs`: apply `rhs` first, then `self`.
    pub fn then_inner(&self, rhs: &Affine) -> Affine {
        Affine {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            tx: self.a * rhs.tx + self.c * rhs.ty + self.tx,
            ty: self.b * rhs.tx + self.d * rhs.ty + self.ty,
        }
    }

    pub fn map_point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    /// Bounding box of the transformed rectangle.
    pub fn map_rect(&self, r: &Rect) -> Rect {
        let pts = [
            self.map_point(r.left, r.top),
            self.map_point(r.right, r.top),
            self.map_point(r.right, r.bottom),
            self.map_point(r.left, r.bottom),
        ];
        let mut out = Rect::new(
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for (x, y) in pts {
            out.left = out.left.min(x);
            out.top = out.top.min(y);
            out.right = out.right.max(x);
            out.bottom = out.bottom.max(y);
        }
        out
    }

    /// Largest factor by which this transform stretches a length.
    pub fn max_scale(&self) -> f32 {
        // largest singular value of the 2x2 linear part
        let (a, b, c, d) = (self.a, self.b, self.c, self.d);
        let s1 = a * a + b * b + c * c + d * d;
        let s2 = ((a * a + b * b - c * c - d * d).powi(2) + 4.0 * (a * c + b * d).powi(2)).sqrt();
        ((s1 + s2) / 2.0).max(0.0).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(p: (f32, f32), q: (f32, f32)) -> bool {
        (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3
    }

    #[test]
    fn anchor_point_lands_on_position() {
        // 200x100 box, anchored at its centre, rotated and scaled arbitrarily:
        // the centre must still end up exactly at `position`.
        let m = Affine::layer([640.0, 360.0], 37.0, [1.7, 0.4], [100.0, 50.0]);
        assert!(close(m.map_point(100.0, 50.0), (640.0, 360.0)));
    }

    #[test]
    fn top_left_anchor_is_plain_translation() {
        let m = Affine::layer([180.0, 250.0], 0.0, [1.0, 1.0], [0.0, 0.0]);
        assert!(close(m.map_point(0.0, 0.0), (180.0, 250.0)));
        assert!(close(m.map_point(10.0, 20.0), (190.0, 270.0)));
    }

    #[test]
    fn positive_rotation_is_clockwise_on_screen() {
        // y points down, so +90 degrees sends +x to +y (like CSS rotate()).
        let m = Affine::layer([0.0, 0.0], 90.0, [1.0, 1.0], [0.0, 0.0]);
        assert!(close(m.map_point(1.0, 0.0), (0.0, 1.0)));
    }

    #[test]
    fn scale_happens_before_rotation() {
        let m = Affine::layer([0.0, 0.0], 90.0, [2.0, 1.0], [0.0, 0.0]);
        assert!(close(m.map_point(1.0, 0.0), (0.0, 2.0)));
    }

    #[test]
    fn composition_applies_child_first() {
        let parent = Affine::layer([100.0, 0.0], 0.0, [2.0, 2.0], [0.0, 0.0]);
        let child = Affine::translate(5.0, 5.0);
        let m = parent.then_inner(&child);
        assert!(close(m.map_point(0.0, 0.0), (110.0, 10.0)));
    }

    #[test]
    fn map_rect_bounds_a_rotated_box() {
        let m = Affine::layer([0.0, 0.0], 45.0, [1.0, 1.0], [0.0, 0.0]);
        let r = m.map_rect(&Rect::from_wh(10.0, 10.0));
        let diag = 200.0_f32.sqrt();
        assert!((r.width() - diag).abs() < 1e-3 && (r.height() - diag).abs() < 1e-3);
    }

    #[test]
    fn max_scale_ignores_rotation() {
        let m = Affine::layer([3.0, 4.0], 61.0, [2.5, 0.5], [9.0, 9.0]);
        assert!((m.max_scale() - 2.5).abs() < 1e-3);
    }

    #[test]
    fn rect_union_and_intersection() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 20.0, 8.0);
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 20.0, 10.0));
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 10.0, 8.0));
        assert!(a.intersect(&Rect::new(50.0, 50.0, 60.0, 60.0)).is_empty());
        assert_eq!(Rect::new(0.0, 0.0, 0.0, 0.0).union(&b), b);
    }
}
