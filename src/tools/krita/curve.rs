//! Krita's sensor curve: `KisCubicCurve` (libs/image/kis_cubic_curve.cpp).
//!
//! A natural cubic spline through the curve's points, clamped to 0..=1, and
//! — this is the part that matters for matching Krita — never evaluated
//! directly by a brush. Sensors sample it once into a 256-entry table
//! (`floatTransfer(256)`) and read that table back with linear
//! interpolation, so the brush sees a polyline through 256 spline samples,
//! not the spline.

/// The transfer table size `KisDynamicSensor::parameter` asks for.
pub const TRANSFER_SIZE: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct CubicCurve {
    points: Vec<(f64, f64)>,
}

/// `KisCubicSpline<QPointF, qreal>`.
struct Spline {
    a: Vec<f64>,
    b: Vec<f64>,
    c: Vec<f64>,
    d: Vec<f64>,
    h: Vec<f64>,
    begin: f64,
    end: f64,
    intervals: usize,
}

/// `KisTridiagonalSystem::calculate`, with `c == a` as the spline passes it.
fn tridiagonal(a: &[f64], b: &[f64], c: &[f64], f: &[f64]) -> Vec<f64> {
    let size = b.len();
    let mut x = vec![0.0; size];
    if size == 1 {
        x[0] = f[0] / b[0];
        return x;
    }
    let mut alpha = vec![0.0; size];
    let mut beta = vec![0.0; size];
    alpha[1] = -c[0] / b[0];
    beta[1] = f[0] / b[0];
    for i in 1..size - 1 {
        alpha[i + 1] = -c[i] / (a[i - 1] * alpha[i] + b[i]);
        beta[i + 1] = (f[i] - a[i - 1] * beta[i]) / (a[i - 1] * alpha[i] + b[i]);
    }
    let al = *a.last().unwrap();
    x[size - 1] = (f[size - 1] - al * beta[size - 1]) / (b[size - 1] + al * alpha[size - 1]);
    for i in (0..size - 1).rev() {
        x[i] = alpha[i + 1] * x[i + 1] + beta[i + 1];
    }
    x
}

impl Spline {
    fn new(pts: &[(f64, f64)]) -> Self {
        let intervals = pts.len() - 1;
        let mut h = vec![0.0; intervals];
        let mut a = Vec::with_capacity(pts.len());
        for i in 0..intervals {
            h[i] = pts[i + 1].0 - pts[i].0;
            a.push(pts[i].1);
        }
        a.push(pts[intervals].1);

        let mut tri_b = Vec::new();
        let mut tri_f = Vec::new();
        let mut tri_a = Vec::new();
        for i in 0..intervals.saturating_sub(1) {
            tri_b.push(2.0 * (h[i] + h[i + 1]));
            tri_f.push(6.0 * ((a[i + 2] - a[i + 1]) / h[i + 1] - (a[i + 1] - a[i]) / h[i]));
        }
        if intervals > 2 {
            tri_a.extend_from_slice(&h[1..intervals - 1]);
        }
        let mut c = if intervals > 1 {
            tridiagonal(&tri_a, &tri_b, &tri_a, &tri_f)
        } else {
            Vec::new()
        };
        c.insert(0, 0.0);
        c.push(0.0);

        let mut d = vec![0.0; intervals];
        for i in 0..intervals {
            d[i] = (c[i + 1] - c[i]) / h[i];
        }
        let mut b = vec![0.0; intervals];
        for i in 0..intervals {
            b[i] = -0.5 * (c[i] * h[i]) - (1.0 / 6.0) * (d[i] * h[i] * h[i]) + (a[i + 1] - a[i]) / h[i];
        }
        Self {
            a,
            b,
            c,
            d,
            h,
            begin: pts[0].0,
            end: pts[intervals].0,
            intervals,
        }
    }

    fn find_region(&self, x: f64) -> (usize, f64) {
        let mut x0 = self.begin;
        for i in 0..self.intervals {
            if x >= x0 && x < x0 + self.h[i] {
                return (i, x0);
            }
            x0 += self.h[i];
        }
        // Past the last knot (only ever `x == end`): the last interval.
        (self.intervals - 1, x0 - self.h[self.intervals - 1])
    }

    fn value(&self, x: f64) -> f64 {
        let (i, x0) = self.find_region(x);
        let t = x - x0;
        self.a[i] + self.b[i] * t + 0.5 * self.c[i] * t * t + (1.0 / 6.0) * self.d[i] * t * t * t
    }
}

impl CubicCurve {
    /// The identity, Krita's `DEFAULT_CURVE_STRING`.
    #[cfg(test)]
    pub fn identity() -> Self {
        Self {
            points: vec![(0.0, 0.0), (1.0, 1.0)],
        }
    }

    /// Parse Krita's `"x,y;x,y;"` form. Pairs without a comma are skipped;
    /// the points are sorted by x, as `setPoints` does.
    pub fn parse(s: &str) -> Option<Self> {
        let mut points = Vec::new();
        for pair in s.split(';') {
            if !pair.contains(',') {
                continue;
            }
            let mut it = pair.split(',');
            let x = it.next()?.trim().parse::<f64>().ok()?;
            let y = it.next()?.trim().parse::<f64>().ok()?;
            points.push((x, y));
        }
        if points.len() < 2 {
            return None;
        }
        points.sort_by(|p, q| p.0.total_cmp(&q.0));
        Some(Self { points })
    }

    /// `KisCubicCurve::isIdentity`. A sensor with an identity curve skips
    /// the transfer table entirely and reports its raw value.
    pub fn is_identity(&self) -> bool {
        let n = self.points.len();
        if self.points[0] != (0.0, 0.0) || self.points[n - 1] != (1.0, 1.0) {
            return false;
        }
        self.points[1..n - 1]
            .iter()
            .all(|&(x, y)| super::qt::fuzzy_compare(x, y))
    }

    /// `KisCubicCurve::floatTransfer(size)`.
    pub fn float_transfer(&self, size: usize) -> Vec<f64> {
        let spline = Spline::new(&self.points);
        let end = 1.0 / (size - 1) as f64;
        (0..size)
            .map(|i| {
                let x = (i as f64 * end).clamp(spline.begin, spline.end);
                let y = spline.value(x).clamp(0.0, 1.0);
                y.clamp(0.0, 1.0)
            })
            .collect()
    }
}

/// `KisCubicCurve::interpolateLinear`.
pub fn interpolate_linear(normalized: f64, transfer: &[f64]) -> f64 {
    let max_value = (transfer.len() - 1) as f64;
    let bilinear_x = (max_value * normalized).clamp(0.0, max_value);
    let floored = bilinear_x.floor();
    let ceiled = bilinear_x.ceil();
    let t = bilinear_x - floored;
    const EPS: f64 = 1e-6;
    let v = if t < EPS {
        transfer[floored as usize]
    } else if t > 1.0 - EPS {
        transfer[ceiled as usize]
    } else {
        let a = transfer[floored as usize];
        let b = transfer[ceiled as usize];
        a + t * (b - a)
    };
    // KisAlgebra2D::copysign
    if normalized >= 0.0 {
        v.abs()
    } else {
        -v.abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_recognised() {
        assert!(CubicCurve::parse("0,0;1,1;").unwrap().is_identity());
        assert!(!CubicCurve::parse("0,1;1,0.246231;").unwrap().is_identity());
    }

    /// Two points make a straight line: the spline has no curvature to add.
    #[test]
    fn two_point_curve_is_linear() {
        let c = CubicCurve::parse("0,1;1,0.246231;").unwrap();
        let t = c.float_transfer(TRANSFER_SIZE);
        assert!((t[0] - 1.0).abs() < 1e-12);
        assert!((t[255] - 0.246231).abs() < 1e-12);
        let mid = interpolate_linear(0.5, &t);
        assert!((mid - (1.0 + 0.246231) / 2.0).abs() < 1e-9, "{mid}");
    }

    /// Pencil-5's opacity curve passes through its middle knot.
    #[test]
    fn spline_passes_through_its_knots() {
        let c = CubicCurve::parse("0,0;0.144578,0.0481932;1,1;").unwrap();
        let t = c.float_transfer(TRANSFER_SIZE);
        let v = interpolate_linear(0.144578, &t);
        assert!((v - 0.0481932).abs() < 2e-3, "{v}");
        assert_eq!(interpolate_linear(0.0, &t), 0.0);
        assert!((interpolate_linear(1.0, &t) - 1.0).abs() < 1e-12);
    }
}
