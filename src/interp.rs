use std::collections::BTreeMap;

use crate::capture::Px;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DedupMode {
    Lowest,
    Highest,
    #[default]
    Middle,
}

impl DedupMode {
    pub fn name(self) -> &'static str {
        match self {
            DedupMode::Lowest => "Lowest",
            DedupMode::Highest => "Highest",
            DedupMode::Middle => "Middle",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InterpMode {
    #[default]
    MeanSquared,
    Cubic,
    Spline,
}

impl InterpMode {
    pub fn name(self) -> &'static str {
        match self {
            InterpMode::MeanSquared => "Mean squared",
            InterpMode::Cubic => "Cubic",
            InterpMode::Spline => "Spline",
        }
    }
}

pub fn dedup(pts: &[Px], mode: DedupMode) -> Vec<Px> {
    let mut cols: BTreeMap<i64, Vec<f32>> = BTreeMap::new();
    for &(x, y) in pts {
        cols.entry(x.round() as i64).or_default().push(y);
    }
    let mut out = Vec::with_capacity(cols.len());
    for (x, mut ys) in cols {
        ys.sort_by(f32::total_cmp);
        ys.dedup();
        let y = match mode {
            DedupMode::Lowest => ys[0],
            DedupMode::Highest => ys[ys.len() - 1],
            DedupMode::Middle => {
                let n = ys.len();
                if n % 2 == 1 {
                    ys[n / 2]
                } else {
                    (ys[n / 2 - 1] + ys[n / 2]) * 0.5
                }
            }
        };
        out.push((x as f32, y));
    }
    out
}

pub fn resample(pts: &[Px], mode: InterpMode) -> Vec<Px> {
    let nodes = dedup(pts, DedupMode::Middle);
    if nodes.len() < 2 {
        return nodes;
    }
    let x0 = nodes[0].0.round() as i64;
    let x1 = nodes[nodes.len() - 1].0.round() as i64;
    let xs: Vec<f64> = nodes.iter().map(|p| p.0 as f64).collect();
    let ys: Vec<f64> = nodes.iter().map(|p| p.1 as f64).collect();
    match mode {
        InterpMode::MeanSquared => {
            let (a, b) = line_fit(&xs, &ys);
            (x0..=x1)
                .map(|xi| (xi as f32, (a + b * xi as f64) as f32))
                .collect()
        }
        InterpMode::Cubic => {
            let tangents = hermite_tangents(&xs, &ys);
            (x0..=x1)
                .map(|xi| {
                    let y = eval_hermite(&xs, &ys, &tangents, xi as f64);
                    (xi as f32, y as f32)
                })
                .collect()
        }
        InterpMode::Spline => {
            let m = spline_second_derivs(&xs, &ys);
            (x0..=x1)
                .map(|xi| {
                    let y = eval_spline(&xs, &ys, &m, xi as f64);
                    (xi as f32, y as f32)
                })
                .collect()
        }
    }
}

fn line_fit(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let d = n * sxx - sx * sx;
    if d.abs() < 1e-12 {
        return (sy / n, 0.0);
    }
    let b = (n * sxy - sx * sy) / d;
    let a = (sy - b * sx) / n;
    (a, b)
}

fn hermite_tangents(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let mut t = vec![0.0; n];
    if n < 2 {
        return t;
    }
    t[0] = (ys[1] - ys[0]) / (xs[1] - xs[0]);
    t[n - 1] = (ys[n - 1] - ys[n - 2]) / (xs[n - 1] - xs[n - 2]);
    for i in 1..n - 1 {
        t[i] = (ys[i + 1] - ys[i - 1]) / (xs[i + 1] - xs[i - 1]);
    }
    t
}

fn find_segment(xs: &[f64], x: f64) -> usize {
    let n = xs.len();
    let mut i = 0;
    while i < n - 2 && x > xs[i + 1] {
        i += 1;
    }
    i
}

fn eval_hermite(xs: &[f64], ys: &[f64], tangents: &[f64], x: f64) -> f64 {
    let i = find_segment(xs, x);
    let h = xs[i + 1] - xs[i];
    let u = (x - xs[i]) / h;
    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    h00 * ys[i] + h10 * h * tangents[i] + h01 * ys[i + 1] + h11 * h * tangents[i + 1]
}

fn spline_second_derivs(xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let n = xs.len();
    let mut m = vec![0.0; n];
    if n < 3 {
        return m;
    }
    let h: Vec<f64> = (0..n - 1).map(|i| xs[i + 1] - xs[i]).collect();
    let mut a = vec![0.0; n];
    let mut b = vec![0.0; n];
    let mut c = vec![0.0; n];
    let mut d = vec![0.0; n];
    for i in 1..n - 1 {
        a[i] = h[i - 1];
        b[i] = 2.0 * (h[i - 1] + h[i]);
        c[i] = h[i];
        d[i] = 6.0 * ((ys[i + 1] - ys[i]) / h[i] - (ys[i] - ys[i - 1]) / h[i - 1]);
    }
    b[0] = 1.0;
    b[n - 1] = 1.0;
    for i in 1..n {
        let w = a[i] / b[i - 1];
        b[i] -= w * c[i - 1];
        d[i] -= w * d[i - 1];
    }
    m[n - 1] = d[n - 1] / b[n - 1];
    for i in (0..n - 1).rev() {
        m[i] = (d[i] - c[i] * m[i + 1]) / b[i];
    }
    m
}

fn eval_spline(xs: &[f64], ys: &[f64], m: &[f64], x: f64) -> f64 {
    let i = find_segment(xs, x);
    let h = xs[i + 1] - xs[i];
    let a = (xs[i + 1] - x) / h;
    let b = (x - xs[i]) / h;
    a * ys[i] + b * ys[i + 1] + ((a * a * a - a) * m[i] + (b * b * b - b) * m[i + 1]) * h * h / 6.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn dedup_modes_pick_expected_y() {
        let pts = vec![(1.0, 10.0), (1.0, 5.0), (1.0, 7.0), (2.0, 3.0)];
        assert_eq!(dedup(&pts, DedupMode::Lowest), vec![(1.0, 5.0), (2.0, 3.0)]);
        assert_eq!(
            dedup(&pts, DedupMode::Highest),
            vec![(1.0, 10.0), (2.0, 3.0)]
        );
        assert_eq!(dedup(&pts, DedupMode::Middle), vec![(1.0, 7.0), (2.0, 3.0)]);
    }

    #[test]
    fn middle_uses_distinct_values() {
        let pts = vec![(1.0, 5.0), (1.0, 5.0), (1.0, 7.0)];
        assert_eq!(dedup(&pts, DedupMode::Middle), vec![(1.0, 6.0)]);
        let pts = vec![(1.0, 4.0), (1.0, 4.0)];
        assert_eq!(dedup(&pts, DedupMode::Middle), vec![(1.0, 4.0)]);
        let pts = vec![(1.0, 1.0), (1.0, 2.0), (1.0, 2.0), (1.0, 9.0), (2.0, 0.0)];
        assert_eq!(dedup(&pts, DedupMode::Middle), vec![(1.0, 2.0), (2.0, 0.0)]);
    }

    #[test]
    fn dedup_bins_fractional_x_and_sorts() {
        let pts = vec![(3.6, 2.0), (0.9, 8.0), (1.4, 6.0), (3.2, 4.0)];
        let out = dedup(&pts, DedupMode::Middle);
        assert_eq!(out, vec![(1.0, 7.0), (3.0, 4.0), (4.0, 2.0)]);
    }

    #[test]
    fn all_modes_exact_on_linear_data() {
        let pts = vec![(0.0, 0.0), (1.0, 2.0), (2.0, 4.0), (3.0, 6.0)];
        for mode in [
            InterpMode::MeanSquared,
            InterpMode::Cubic,
            InterpMode::Spline,
        ] {
            let out = resample(&pts, mode);
            assert_eq!(out.len(), 4);
            for (i, &(x, y)) in out.iter().enumerate() {
                assert_eq!(x, i as f32);
                assert!(close(y, 2.0 * i as f32), "mode {mode:?} at x {x}: {y}");
            }
        }
    }

    #[test]
    fn resample_fills_gaps_between_nodes() {
        let pts = vec![(0.0, 0.0), (10.0, 10.0)];
        let out = resample(&pts, InterpMode::Cubic);
        assert_eq!(out.len(), 11);
        assert!(close(out[5].1, 5.0));
        let out = resample(&pts, InterpMode::Spline);
        assert_eq!(out.len(), 11);
        assert!(close(out[7].1, 7.0));
    }

    #[test]
    fn cubic_and_spline_pass_through_nodes() {
        let pts = vec![(0.0, 0.0), (1.0, 3.0), (2.0, -1.0), (3.0, 2.0), (4.0, 0.0)];
        for mode in [InterpMode::Cubic, InterpMode::Spline] {
            let out = resample(&pts, mode);
            assert_eq!(out.len(), 5);
            for (i, &(x, y)) in out.iter().enumerate() {
                assert_eq!(x, i as f32);
                assert!(
                    close(y, pts[i].1),
                    "mode {mode:?} node {i}: {y} vs {}",
                    pts[i].1
                );
            }
        }
    }

    #[test]
    fn mean_squared_fits_least_squares_line() {
        let pts = vec![(0.0, 0.0), (1.0, 1.1), (2.0, 1.9), (3.0, 3.2)];
        let out = resample(&pts, InterpMode::MeanSquared);
        assert_eq!(out.len(), 4);
        assert!(close(out[0].1, -0.01));
        assert!(close(out[3].1 - out[0].1, 3.12));
    }

    #[test]
    fn resample_degenerate_inputs() {
        assert!(resample(&[], InterpMode::Cubic).is_empty());
        assert_eq!(
            resample(&[(5.0, 7.0)], InterpMode::Spline),
            vec![(5.0, 7.0)]
        );
    }
}
