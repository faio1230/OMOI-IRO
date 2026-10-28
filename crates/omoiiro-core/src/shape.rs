//! Shapes on the 8 × 8 grid (0.1.3, docs/14 §4-3): one signed distance from
//! a cell centre to a shape, and the alpha it gives. `builtin.shape`,
//! `builtin.path` (its spot) and the ring shapes of `builtin.ripple` use
//! these functions, so a circle, a diamond and a square look the same in
//! every effect. Only the drawing is shared; the file formats are not.
//!
//! Distances are in cells. A negative distance is inside. The metric of a
//! shape is the distance it is round in: Euclidean for a circle, Manhattan
//! for a diamond, Chebyshev for a square, so `metric(m, dx, dy) - size` is
//! the signed distance of that shape of half-size `size`.

/// The distance a shape is round in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// Euclidean (circle).
    Circle,
    /// Manhattan (diamond).
    Diamond,
    /// Chebyshev (square).
    Square,
}

/// Distance of offset (dx, dy) in the metric.
#[inline]
pub fn metric(m: Metric, dx: f64, dy: f64) -> f64 {
    let (ax, ay) = (dx.abs(), dy.abs());
    match m {
        Metric::Circle => (ax * ax + ay * ay).sqrt(),
        Metric::Diamond => ax + ay,
        Metric::Square => ax.max(ay),
    }
}

/// Rotate (dx, dy) by `-angle_deg` (into the shape's own axes).
#[inline]
pub fn unrotate(dx: f64, dy: f64, angle_deg: f64) -> (f64, f64) {
    if angle_deg == 0.0 {
        return (dx, dy);
    }
    let (s, c) = (-angle_deg.to_radians()).sin_cos();
    (dx * c - dy * s, dx * s + dy * c)
}

/// Signed distance to a rectangle of half sizes (hx, hy) centred at the origin.
#[inline]
pub fn sd_rect(dx: f64, dy: f64, hx: f64, hy: f64) -> f64 {
    let (qx, qy) = (dx.abs() - hx, dy.abs() - hy);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0)
}

/// Distance from (px, py) to the segment (ax, ay)–(bx, by).
#[inline]
pub fn segment_distance(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (vx, vy) = (bx - ax, by - ay);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 { (((px - ax) * vx + (py - ay) * vy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (ax + vx * t - px, ay + vy * t - py);
    (cx * cx + cy * cy).sqrt()
}

/// Alpha of a filled shape at signed distance `sd`: 1 inside; with `edge`
/// cells of soft edge it falls linearly across the boundary (0.5 on it).
#[inline]
pub fn fill_alpha(sd: f64, edge: f64) -> f64 {
    if edge > 0.0 {
        (0.5 - sd / edge).clamp(0.0, 1.0)
    } else if sd <= 0.0 {
        1.0
    } else {
        0.0
    }
}

/// Alpha of an outline `stroke` cells wide centred on the boundary.
/// Without a soft edge the stroke keeps at least the cell size of 1 so a
/// thin line on the grid does not fall between cell centres.
#[inline]
pub fn stroke_alpha(sd: f64, stroke: f64, edge: f64) -> f64 {
    let half = if edge > 0.0 { stroke / 2.0 } else { stroke.max(1.0) / 2.0 };
    fill_alpha(sd.abs() - half, edge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_and_signed_distances() {
        assert_eq!(metric(Metric::Circle, 3.0, 4.0), 5.0);
        assert_eq!(metric(Metric::Diamond, 3.0, -4.0), 7.0);
        assert_eq!(metric(Metric::Square, -3.0, 4.0), 4.0);
        assert_eq!(sd_rect(0.0, 0.0, 2.0, 1.0), -1.0);
        assert_eq!(sd_rect(3.0, 0.0, 2.0, 1.0), 1.0);
        assert_eq!(sd_rect(5.0, 5.0, 2.0, 1.0), 5.0);
        assert_eq!(segment_distance(1.0, 1.0, 0.0, 0.0, 2.0, 0.0), 1.0);
        assert_eq!(segment_distance(5.0, 0.0, 0.0, 0.0, 2.0, 0.0), 3.0);
        let (x, y) = unrotate(0.0, 1.0, 90.0);
        assert!((x - 1.0).abs() < 1e-9 && y.abs() < 1e-9);
    }

    #[test]
    fn fill_and_stroke_alpha() {
        assert_eq!((fill_alpha(-0.2, 0.0), fill_alpha(0.2, 0.0)), (1.0, 0.0));
        assert_eq!(fill_alpha(0.0, 1.0), 0.5);
        assert_eq!(fill_alpha(-1.0, 1.0), 1.0);
        assert_eq!(stroke_alpha(0.3, 1.0, 0.0), 1.0, "on a 1-cell line");
        assert_eq!(stroke_alpha(-2.0, 1.0, 0.0), 0.0, "deep inside is not the outline");
        assert_eq!(stroke_alpha(0.0, 0.5, 0.0), 1.0, "thin strokes keep one cell");
    }
}
