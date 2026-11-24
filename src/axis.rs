use std::collections::HashMap;

use image::{GrayImage, RgbaImage};
use imageproc::contrast::{ThresholdType, otsu_level, threshold};
use imageproc::region_labelling::{Connectivity, connected_components};

use crate::edit::clear_border_components;

pub const FIXED_FALLBACK_LEVEL: u8 = 225;
pub const COVERAGE_FULL_LIMIT: f64 = 0.99;
pub const COVERAGE_TINY_LIMIT: f64 = 0.10;
pub const CORNER_WINDOW: i64 = 10;
pub const CORNER_MIN_COVERAGE: f32 = 0.95;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl AxisRect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn is_valid(&self, w: u32, h: u32) -> bool {
        self.width() > 1.0
            && self.height() > 1.0
            && self.left >= 0.0
            && self.top >= 0.0
            && self.right <= w as f32
            && self.bottom <= h as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    Linear,
    Log { base: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRange {
    pub min: f64,
    pub max: f64,
    pub scale: Scale,
}

impl Default for AxisRange {
    fn default() -> Self {
        Self {
            min: 0.0,
            max: 1.0,
            scale: Scale::Linear,
        }
    }
}

impl AxisRange {
    #[cfg(test)]
    pub fn linear(min: f64, max: f64) -> Self {
        Self {
            min,
            max,
            scale: Scale::Linear,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.min.is_finite() || !self.max.is_finite() {
            return Err("values must be finite".to_string());
        }
        if self.min >= self.max {
            return Err("min must be less than max".to_string());
        }
        if let Scale::Log { base } = self.scale {
            if !base.is_finite() || base <= 0.0 || base == 1.0 {
                return Err("log base must be positive and not 1".to_string());
            }
            if self.min <= 0.0 || self.max <= 0.0 {
                return Err("log axis requires positive limits".to_string());
            }
        }
        Ok(())
    }

    pub fn t_to_data(&self, t: f64) -> f64 {
        match self.scale {
            Scale::Linear => self.min + t * (self.max - self.min),
            Scale::Log { base } => {
                let lmin = log_base(base, self.min);
                let lmax = log_base(base, self.max);
                base.powf(lmin + t * (lmax - lmin))
            }
        }
    }
}

pub fn log_base(base: f64, n: f64) -> f64 {
    n.ln() / base.ln()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Calibration {
    pub x: AxisRange,
    pub y: AxisRange,
}

impl Calibration {
    pub fn validate(&self) -> Result<(), String> {
        self.x.validate().map_err(|e| format!("x axis: {e}"))?;
        self.y.validate().map_err(|e| format!("y axis: {e}"))?;
        Ok(())
    }

    pub fn pixel_to_data(&self, r: &AxisRect, p: (f32, f32)) -> (f64, f64) {
        let w = r.width();
        let h = r.height();
        let tx = if w > 0.0 {
            ((p.0 - r.left) / w).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let ty = if h > 0.0 {
            ((r.bottom - p.1) / h).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (self.x.t_to_data(tx as f64), self.y.t_to_data(ty as f64))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

struct Detection {
    rect: AxisRect,
    coverage: f64,
}

pub fn detect_axis_rect(img: &RgbaImage) -> Option<AxisRect> {
    let gray = image::imageops::grayscale(img);
    let level = otsu_level(&gray);
    let first = detect_at_level(&gray, level)?;
    if first.coverage < COVERAGE_TINY_LIMIT && level < FIXED_FALLBACK_LEVEL {
        return detect_at_level(&gray, FIXED_FALLBACK_LEVEL).map(|d| d.rect);
    }
    Some(first.rect)
}

fn detect_at_level(gray: &GrayImage, level: u8) -> Option<Detection> {
    let bin = threshold(gray, level, ThresholdType::BinaryInverted);
    let (w, h) = bin.dimensions();
    let bbox = largest_component_bbox(&bin)?;
    let coverage = bbox_coverage(bbox, w, h);
    if coverage > COVERAGE_FULL_LIMIT {
        let mut cleared = bin.clone();
        clear_border_components(&mut cleared);
        let bbox2 = largest_component_bbox(&cleared)?;
        return Some(Detection {
            rect: refine_rect(&cleared, bbox2),
            coverage: bbox_coverage(bbox2, w, h),
        });
    }
    Some(Detection {
        rect: refine_rect(&bin, bbox),
        coverage,
    })
}

fn bbox_coverage(bbox: (u32, u32, u32, u32), w: u32, h: u32) -> f64 {
    let (minx, miny, maxx, maxy) = bbox;
    let bw = (maxx - minx + 1) as f64;
    let bh = (maxy - miny + 1) as f64;
    bw * bh / (w as f64 * h as f64)
}

fn largest_component_bbox(bin: &GrayImage) -> Option<(u32, u32, u32, u32)> {
    let labels = connected_components(bin, Connectivity::Eight, image::Luma([0u8]));
    let mut stats: HashMap<u32, (u64, u32, u32, u32, u32)> = HashMap::new();
    for (x, y, label) in labels.enumerate_pixels() {
        let l = label.0[0];
        if l == 0 {
            continue;
        }
        let e = stats.entry(l).or_insert((0, x, y, x, y));
        e.0 += 1;
        e.1 = e.1.min(x);
        e.2 = e.2.min(y);
        e.3 = e.3.max(x);
        e.4 = e.4.max(y);
    }
    stats
        .into_iter()
        .max_by_key(|(_, s)| s.0)
        .map(|(_, s)| (s.1, s.2, s.3, s.4))
}

fn refine_rect(bin: &GrayImage, bbox: (u32, u32, u32, u32)) -> AxisRect {
    let (minx, miny, maxx, maxy) = bbox;
    let rw = (maxx - minx + 1) as i64;
    let rh = (maxy - miny + 1) as i64;
    let l = (rw.min(rh) / 2).max(1);
    let tl = refine_corner(bin, bbox, Corner::TopLeft, l);
    let tr = refine_corner(bin, bbox, Corner::TopRight, l);
    let bl = refine_corner(bin, bbox, Corner::BottomLeft, l);
    let br = refine_corner(bin, bbox, Corner::BottomRight, l);

    let mut left_vals = Vec::new();
    let mut top_vals = Vec::new();
    let mut right_vals = Vec::new();
    let mut bottom_vals = Vec::new();
    if let Some((x, y)) = tl {
        left_vals.push(x);
        top_vals.push(y);
    }
    if let Some((x, y)) = tr {
        right_vals.push(x);
        top_vals.push(y);
    }
    if let Some((x, y)) = bl {
        left_vals.push(x);
        bottom_vals.push(y);
    }
    if let Some((x, y)) = br {
        right_vals.push(x);
        bottom_vals.push(y);
    }
    AxisRect {
        left: mean_or(left_vals, minx as f32),
        top: mean_or(top_vals, miny as f32),
        right: mean_or(right_vals, maxx as f32),
        bottom: mean_or(bottom_vals, maxy as f32),
    }
}

fn mean_or(vals: Vec<f32>, fallback: f32) -> f32 {
    if vals.is_empty() {
        fallback
    } else {
        vals.iter().sum::<f32>() / vals.len() as f32
    }
}

fn refine_corner(
    bin: &GrayImage,
    bbox: (u32, u32, u32, u32),
    corner: Corner,
    l: i64,
) -> Option<(f32, f32)> {
    let (minx, miny, maxx, maxy) = bbox;
    let (ax, ay, dx, dy) = match corner {
        Corner::TopLeft => (minx as i64, miny as i64, 1, 1),
        Corner::TopRight => (maxx as i64, miny as i64, -1, 1),
        Corner::BottomLeft => (minx as i64, maxy as i64, 1, -1),
        Corner::BottomRight => (maxx as i64, maxy as i64, -1, -1),
    };
    let (w, h) = (bin.width() as i64, bin.height() as i64);
    let x0 = if dx > 0 { ax } else { ax - CORNER_WINDOW + 1 };
    let y0 = if dy > 0 { ay } else { ay - CORNER_WINDOW + 1 };
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut n = 0u32;
    for oy in 0..CORNER_WINDOW {
        let cy = y0 + oy;
        if cy < 0 || cy >= h {
            continue;
        }
        for ox in 0..CORNER_WINDOW {
            let cx = x0 + ox;
            if cx < 0 || cx >= w {
                continue;
            }
            if run_coverage_h(bin, cy, cx, dx, l) >= CORNER_MIN_COVERAGE
                && run_coverage_v(bin, cx, cy, dy, l) >= CORNER_MIN_COVERAGE
            {
                sx += cx as f32;
                sy += cy as f32;
                n += 1;
            }
        }
    }
    if n == 0 {
        None
    } else {
        Some((sx / n as f32, sy / n as f32))
    }
}

fn run_coverage_h(bin: &GrayImage, y: i64, x_start: i64, dir: i64, l: i64) -> f32 {
    let w = bin.width() as i64;
    let x0 = if dir > 0 { x_start } else { x_start - l + 1 };
    let x1 = x0 + l;
    let xa = x0.max(0);
    let xb = x1.min(w);
    if xb <= xa {
        return 0.0;
    }
    let mut dark = 0u32;
    for x in xa..xb {
        if bin.get_pixel(x as u32, y as u32).0[0] > 0 {
            dark += 1;
        }
    }
    dark as f32 / l as f32
}

fn run_coverage_v(bin: &GrayImage, x: i64, y_start: i64, dir: i64, l: i64) -> f32 {
    let h = bin.height() as i64;
    let y0 = if dir > 0 { y_start } else { y_start - l + 1 };
    let y1 = y0 + l;
    let ya = y0.max(0);
    let yb = y1.min(h);
    if yb <= ya {
        return 0.0;
    }
    let mut dark = 0u32;
    for y in ya..yb {
        if bin.get_pixel(x as u32, y as u32).0[0] > 0 {
            dark += 1;
        }
    }
    dark as f32 / l as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use imageproc::drawing::{draw_filled_rect_mut, draw_line_segment_mut};

    const BG: Rgba<u8> = Rgba([255, 255, 255, 255]);
    const INK: Rgba<u8> = Rgba([0, 0, 0, 255]);

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, BG)
    }

    fn frame(img: &mut RgbaImage, left: i32, top: i32, right: i32, bottom: i32, thickness: i32) {
        draw_line_segment_mut(
            img,
            (left as f32, top as f32),
            (right as f32, top as f32),
            INK,
        );
        draw_line_segment_mut(
            img,
            (left as f32, bottom as f32),
            (right as f32, bottom as f32),
            INK,
        );
        draw_line_segment_mut(
            img,
            (left as f32, top as f32),
            (left as f32, bottom as f32),
            INK,
        );
        draw_line_segment_mut(
            img,
            (right as f32, top as f32),
            (right as f32, bottom as f32),
            INK,
        );
        let _ = thickness;
    }

    #[test]
    fn detects_simple_frame_edges() {
        let mut img = blank(200, 150);
        frame(&mut img, 10, 10, 190, 140, 2);
        let r = detect_axis_rect(&img).expect("frame detected");
        assert!((r.left - 10.0).abs() <= 1.0, "left {}", r.left);
        assert!((r.top - 10.0).abs() <= 1.0, "top {}", r.top);
        assert!((r.right - 190.0).abs() <= 1.0, "right {}", r.right);
        assert!((r.bottom - 140.0).abs() <= 1.0, "bottom {}", r.bottom);
    }

    #[test]
    fn detects_frame_with_data_inside() {
        let mut img = blank(300, 200);
        frame(&mut img, 20, 20, 280, 180, 2);
        draw_filled_rect_mut(
            &mut img,
            imageproc::rect::Rect::at(150, 100).of_size(3, 3),
            INK,
        );
        let r = detect_axis_rect(&img).expect("frame detected");
        assert!((r.left - 20.0).abs() <= 1.0);
        assert!((r.top - 20.0).abs() <= 1.0);
        assert!((r.right - 280.0).abs() <= 1.0);
        assert!((r.bottom - 180.0).abs() <= 1.0);
    }

    #[test]
    fn clear_border_fallback_for_touching_frame() {
        let mut img = blank(100, 80);
        frame(&mut img, 0, 0, 99, 79, 2);
        draw_line_segment_mut(&mut img, (30.0, 20.0), (70.0, 60.0), INK);
        let r = detect_axis_rect(&img).expect("inner structure detected");
        assert!(r.left >= 25.0 && r.left <= 35.0, "left {}", r.left);
        assert!(r.top >= 15.0 && r.top <= 25.0, "top {}", r.top);
        assert!(r.right >= 65.0 && r.right <= 75.0, "right {}", r.right);
        assert!(r.bottom >= 55.0 && r.bottom <= 65.0, "bottom {}", r.bottom);
    }

    #[test]
    fn fixed_level_fallback_for_tiny_frame() {
        let mut img = blank(200, 200);
        for p in img.pixels_mut() {
            *p = Rgba([230, 230, 230, 255]);
        }
        frame(&mut img, 90, 90, 110, 110, 2);
        let r = detect_axis_rect(&img).expect("tiny frame detected");
        assert!((r.left - 90.0).abs() <= 1.0);
        assert!((r.top - 90.0).abs() <= 1.0);
        assert!((r.right - 110.0).abs() <= 1.0);
        assert!((r.bottom - 110.0).abs() <= 1.0);
    }

    #[test]
    fn no_axes_in_blank_image() {
        let img = blank(100, 100);
        assert!(detect_axis_rect(&img).is_none());
    }

    #[test]
    fn rect_validation() {
        let r = AxisRect {
            left: 10.0,
            top: 5.0,
            right: 100.0,
            bottom: 90.0,
        };
        assert!(r.is_valid(200, 100));
        let bad = AxisRect {
            left: 10.0,
            top: 5.0,
            right: 9.0,
            bottom: 90.0,
        };
        assert!(!bad.is_valid(200, 100));
        let overflow = AxisRect {
            left: 10.0,
            top: 5.0,
            right: 250.0,
            bottom: 90.0,
        };
        assert!(!overflow.is_valid(200, 100));
    }

    #[test]
    fn linear_transform_and_readout() {
        let rect = AxisRect {
            left: 10.0,
            top: 20.0,
            right: 110.0,
            bottom: 120.0,
        };
        let cal = Calibration {
            x: AxisRange::linear(0.0, 100.0),
            y: AxisRange::linear(0.0, 50.0),
        };
        assert_eq!(cal.pixel_to_data(&rect, (10.0, 120.0)), (0.0, 0.0));
        assert_eq!(cal.pixel_to_data(&rect, (110.0, 20.0)), (100.0, 50.0));
        assert_eq!(cal.pixel_to_data(&rect, (60.0, 70.0)), (50.0, 25.0));
    }

    #[test]
    fn log_transform_base10_and_base2() {
        let range = AxisRange {
            min: 1.0,
            max: 100.0,
            scale: Scale::Log { base: 10.0 },
        };
        assert!((range.t_to_data(0.0) - 1.0).abs() < 1e-9);
        assert!((range.t_to_data(0.5) - 10.0).abs() < 1e-6);
        assert!((range.t_to_data(1.0) - 100.0).abs() < 1e-6);
        let r2 = AxisRange {
            min: 1.0,
            max: 8.0,
            scale: Scale::Log { base: 2.0 },
        };
        assert!((r2.t_to_data(0.5) - 2.0 * 2.0f64.sqrt()).abs() < 1e-9);
        assert!((r2.t_to_data(1.0) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn validation_rejects_bad_ranges() {
        assert!(AxisRange::linear(0.0, 1.0).validate().is_ok());
        assert!(AxisRange::linear(1.0, 0.0).validate().is_err());
        assert!(AxisRange::linear(0.0, 0.0).validate().is_err());
        assert!(
            AxisRange {
                min: -1.0,
                max: 10.0,
                scale: Scale::Log { base: 10.0 }
            }
            .validate()
            .is_err()
        );
        assert!(
            AxisRange {
                min: 1.0,
                max: 10.0,
                scale: Scale::Log { base: 1.0 }
            }
            .validate()
            .is_err()
        );
        assert!(
            AxisRange {
                min: 0.1,
                max: 10.0,
                scale: Scale::Log { base: 2.0 }
            }
            .validate()
            .is_ok()
        );
    }
}
