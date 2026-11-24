use image::RgbaImage;
use imageproc::region_labelling::{Connectivity, connected_components};

use crate::edit::data_mask;
use crate::image_state::is_data_px;

pub type Px = (f32, f32);

pub const POINT_UNDO_CAP: usize = 20;

#[derive(Clone, Debug)]
pub struct Dataset {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    pub points: Vec<Px>,
    pub point_undo: Vec<Vec<Px>>,
    pub click_counts: Vec<usize>,
}

pub const DATASET_COLORS: [[u8; 3]; 10] = [
    [0, 113, 188],
    [217, 72, 1],
    [0, 158, 115],
    [104, 33, 122],
    [140, 86, 75],
    [196, 156, 148],
    [227, 119, 194],
    [127, 127, 127],
    [188, 189, 34],
    [23, 190, 207],
];

impl Dataset {
    pub fn new(id: u64, name: impl Into<String>, color: [u8; 3]) -> Self {
        Self {
            id,
            name: name.into(),
            color,
            points: Vec::new(),
            point_undo: Vec::new(),
            click_counts: Vec::new(),
        }
    }

    pub fn snapshot_points(&mut self) {
        self.point_undo.push(self.points.clone());
        if self.point_undo.len() > POINT_UNDO_CAP {
            self.point_undo.remove(0);
        }
    }

    pub fn undo_points(&mut self) -> bool {
        match self.point_undo.pop() {
            Some(prev) => {
                self.points = prev;
                true
            }
            None => false,
        }
    }

    pub fn clear_capture_history(&mut self) {
        self.point_undo.clear();
        self.click_counts.clear();
    }
}

pub fn capture_continuous(img: &RgbaImage) -> Vec<Px> {
    let mut pts: Vec<Px> = Vec::new();
    for (x, y, p) in img.enumerate_pixels() {
        if is_data_px(p) {
            pts.push((x as f32, y as f32));
        }
    }
    pts.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
    });
    pts
}

pub fn capture_discrete(img: &RgbaImage, min_area: u32) -> Vec<Px> {
    let mask = data_mask(img);
    let labels = connected_components(&mask, Connectivity::Eight, image::Luma([0u8]));
    let mut stats: std::collections::HashMap<u32, (u64, f64, f64)> =
        std::collections::HashMap::new();
    for (x, y, l) in labels.enumerate_pixels() {
        let l = l.0[0];
        if l == 0 {
            continue;
        }
        let e = stats.entry(l).or_insert((0, 0.0, 0.0));
        e.0 += 1;
        e.1 += x as f64;
        e.2 += y as f64;
    }
    let mut pts: Vec<Px> = stats
        .into_values()
        .filter(|(count, _, _)| *count >= min_area as u64)
        .map(|(count, sx, sy)| ((sx / count as f64) as f32, (sy / count as f64) as f32))
        .collect();
    pts.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap()
            .then(a.1.partial_cmp(&b.1).unwrap())
    });
    pts
}

pub fn segment_length(a: Px, b: Px) -> f32 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

pub fn interpolate_segment(a: Px, b: Px, step: f32) -> Vec<Px> {
    if step <= 0.0 {
        return Vec::new();
    }
    let d = segment_length(a, b);
    if d <= step {
        return Vec::new();
    }
    let n = ((d / step).ceil() as usize).saturating_sub(1);
    (1..=n)
        .map(|k| {
            let t = k as f32 * step / d;
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        })
        .collect()
}

pub fn nearest_point(points: &[Px], p: Px, max_dist: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, q) in points.iter().enumerate() {
        let d = segment_length(*q, p);
        if d <= max_dist && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_state::WHITE;
    use image::Rgba;

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, WHITE)
    }

    #[test]
    fn continuous_collects_all_data_pixels_sorted() {
        let mut img = blank(10, 10);
        img.put_pixel(3, 2, Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 5, Rgba([0, 0, 0, 255]));
        img.put_pixel(3, 1, Rgba([0, 0, 0, 255]));
        let pts = capture_continuous(&img);
        assert_eq!(pts, vec![(1.0, 5.0), (3.0, 1.0), (3.0, 2.0)]);
    }

    #[test]
    fn discrete_finds_centroids_within_half_pixel() {
        let mut img = blank(50, 50);
        for x in 10..15 {
            for y in 10..14 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        for x in 30..36 {
            for y in 20..26 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let pts = capture_discrete(&img, 4);
        assert_eq!(pts.len(), 2);
        assert!((pts[0].0 - 12.0).abs() <= 0.5 && (pts[0].1 - 11.5).abs() <= 0.5);
        assert!((pts[1].0 - 32.5).abs() <= 0.5 && (pts[1].1 - 22.5).abs() <= 0.5);
    }

    #[test]
    fn discrete_min_area_rejects_specks() {
        let mut img = blank(50, 50);
        for x in 10..15 {
            for y in 10..14 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        img.put_pixel(40, 40, Rgba([0, 0, 0, 255]));
        assert_eq!(capture_discrete(&img, 4).len(), 1);
        assert_eq!(capture_discrete(&img, 1).len(), 2);
    }

    #[test]
    fn interpolation_evenly_spaced_intermediates() {
        let pts = interpolate_segment((0.0, 0.0), (30.0, 0.0), 10.0);
        assert_eq!(pts, vec![(10.0, 0.0), (20.0, 0.0)]);
        let pts = interpolate_segment((0.0, 0.0), (35.0, 0.0), 10.0);
        assert_eq!(pts, vec![(10.0, 0.0), (20.0, 0.0), (30.0, 0.0)]);
        assert!(interpolate_segment((0.0, 0.0), (5.0, 0.0), 10.0).is_empty());
        assert!(interpolate_segment((0.0, 0.0), (10.0, 0.0), 10.0).is_empty());
        assert!(interpolate_segment((0.0, 0.0), (30.0, 0.0), 0.0).is_empty());
    }

    #[test]
    fn interpolation_diagonal() {
        let pts = interpolate_segment((0.0, 0.0), (30.0, 30.0), 15.0);
        assert_eq!(pts.len(), 2);
        assert!((pts[0].0 - 15.0 / 2.0f32.sqrt()).abs() < 1e-3);
        assert!((pts[0].1 - 15.0 / 2.0f32.sqrt()).abs() < 1e-3);
        assert!((pts[1].0 - 30.0 / 2.0f32.sqrt()).abs() < 1e-3);
        assert!((pts[1].1 - 30.0 / 2.0f32.sqrt()).abs() < 1e-3);
    }

    #[test]
    fn nearest_point_picks_closest_within_radius() {
        let points = vec![(10.0, 10.0), (20.0, 10.0)];
        assert_eq!(nearest_point(&points, (11.0, 10.0), 2.0), Some(0));
        assert_eq!(nearest_point(&points, (19.5, 10.0), 2.0), Some(1));
        assert_eq!(nearest_point(&points, (15.0, 10.0), 2.0), None);
    }

    #[test]
    fn dataset_undo_roundtrip_and_cap() {
        let mut ds = Dataset::new(1, "d", [0, 0, 0]);
        for k in 0..30 {
            ds.snapshot_points();
            ds.points.push((k as f32, 0.0));
        }
        let mut undos = 0;
        while ds.undo_points() {
            undos += 1;
        }
        assert_eq!(undos, POINT_UNDO_CAP);
        assert_eq!(ds.points.len(), 10);
        ds.clear_capture_history();
        assert!(!ds.undo_points());
    }
}
