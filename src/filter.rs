use image::{GrayImage, Luma, Rgba, RgbaImage};

use crate::axis::AxisRect;
use crate::edit::fill_polygon;
use crate::image_state::WHITE;

#[derive(Clone, Debug, Default)]
pub struct FilterParams {
    pub picked: Option<[u8; 3]>,
    pub tolerance: u8,
    pub mask: Option<Vec<(f32, f32)>>,
}

pub const MAX_TOLERANCE: u8 = 128;

pub fn channel_bounds(picked: [u8; 3], tol: u8) -> ([u8; 3], [u8; 3]) {
    (
        picked.map(|c| c.saturating_sub(tol)),
        picked.map(|c| c.saturating_add(tol)),
    )
}

pub fn matches(p: &Rgba<u8>, min: [u8; 3], max: [u8; 3]) -> bool {
    (0..3).all(|i| p.0[i] >= min[i] && p.0[i] <= max[i])
}

fn inside_crop(crop: Option<AxisRect>, x: u32, y: u32) -> bool {
    crop.is_none_or(|r| {
        let (fx, fy) = (x as f32, y as f32);
        fx >= r.left && fx <= r.right && fy >= r.top && fy <= r.bottom
    })
}

fn polygon_mask(poly: &[(f32, f32)], w: u32, h: u32) -> Option<GrayImage> {
    if poly.len() < 3 {
        return None;
    }
    let mut mask = GrayImage::from_pixel(w, h, Luma([0]));
    fill_polygon(&mut mask, poly, Luma([255]));
    Some(mask)
}

pub fn apply(
    img: &RgbaImage,
    picked: [u8; 3],
    tol: u8,
    crop: Option<AxisRect>,
    mask: Option<&[(f32, f32)]>,
) -> RgbaImage {
    let (min, max) = channel_bounds(picked, tol);
    let mask_bin = mask.and_then(|p| polygon_mask(p, img.width(), img.height()));
    let mut out = img.clone();
    for (x, y, p) in out.enumerate_pixels_mut() {
        let keep =
            inside_crop(crop, x, y) && mask_bin.as_ref().is_none_or(|m| m.get_pixel(x, y).0[0] > 0);
        if !keep || !matches(p, min, max) {
            *p = WHITE;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_state::is_data_px;

    #[test]
    fn channel_bounds_saturate() {
        let (min, max) = channel_bounds([10, 100, 250], 30);
        assert_eq!(min, [0, 70, 220]);
        assert_eq!(max, [40, 130, 255]);
    }

    #[test]
    fn matches_is_per_channel_range() {
        assert!(matches(
            &Rgba([100, 100, 100, 255]),
            [90, 90, 90],
            [110, 110, 110]
        ));
        assert!(!matches(
            &Rgba([100, 100, 100, 255]),
            [105, 90, 90],
            [110, 110, 110]
        ));
        assert!(!matches(
            &Rgba([100, 100, 100, 255]),
            [90, 90, 90],
            [110, 110, 99]
        ));
    }

    #[test]
    fn apply_zeroes_nonmatching_to_white() {
        let mut img = RgbaImage::from_pixel(4, 4, WHITE);
        img.put_pixel(0, 0, Rgba([200, 0, 0, 255]));
        img.put_pixel(1, 1, Rgba([0, 0, 200, 255]));
        let out = apply(&img, [200, 0, 0], 10, None, None);
        assert_eq!(out.get_pixel(0, 0), &Rgba([200, 0, 0, 255]));
        assert_eq!(out.get_pixel(1, 1), &WHITE);
        assert_eq!(out.get_pixel(3, 3), &WHITE);
    }

    #[test]
    fn tolerance_wide_keeps_antialiased_neighbors() {
        let mut img = RgbaImage::from_pixel(2, 2, WHITE);
        img.put_pixel(0, 0, Rgba([190, 10, 10, 255]));
        let out = apply(&img, [200, 0, 0], 30, None, None);
        assert_eq!(out.get_pixel(0, 0), &Rgba([190, 10, 10, 255]));
    }

    #[test]
    fn apply_keeps_only_matching() {
        let mut img = RgbaImage::from_pixel(4, 4, WHITE);
        img.put_pixel(0, 0, Rgba([200, 0, 0, 255]));
        img.put_pixel(1, 1, Rgba([0, 0, 200, 255]));
        let out = apply(&img, [200, 0, 0], 10, None, None);
        assert_eq!(out.get_pixel(0, 0), &Rgba([200, 0, 0, 255]));
        assert_eq!(out.get_pixel(1, 1), &WHITE);
        let kept = out.pixels().filter(|p| is_data_px(p)).count();
        assert_eq!(kept, 1);
    }

    #[test]
    fn crop_zeroes_everything_outside_axis_rect() {
        let mut img = RgbaImage::from_pixel(10, 10, WHITE);
        img.put_pixel(1, 1, Rgba([200, 0, 0, 255]));
        img.put_pixel(8, 8, Rgba([200, 0, 0, 255]));
        img.put_pixel(5, 5, Rgba([200, 0, 0, 255]));
        let rect = AxisRect {
            left: 4.0,
            top: 4.0,
            right: 9.0,
            bottom: 9.0,
        };
        let out = apply(&img, [200, 0, 0], 10, Some(rect), None);
        assert_eq!(out.get_pixel(1, 1), &WHITE);
        assert_eq!(out.get_pixel(8, 8), &Rgba([200, 0, 0, 255]));
        assert_eq!(out.get_pixel(5, 5), &Rgba([200, 0, 0, 255]));
    }

    #[test]
    fn mask_limits_filtering_to_polygon() {
        let mut img = RgbaImage::from_pixel(10, 10, WHITE);
        img.put_pixel(2, 2, Rgba([200, 0, 0, 255]));
        img.put_pixel(7, 7, Rgba([200, 0, 0, 255]));
        let poly = [(5.0, 5.0), (10.0, 5.0), (10.0, 10.0), (5.0, 10.0)];
        let out = apply(&img, [200, 0, 0], 10, None, Some(&poly));
        assert_eq!(out.get_pixel(2, 2), &WHITE);
        assert_eq!(out.get_pixel(7, 7), &Rgba([200, 0, 0, 255]));
    }

    #[test]
    fn crop_and_mask_combine() {
        let mut img = RgbaImage::from_pixel(12, 12, WHITE);
        img.put_pixel(2, 8, Rgba([200, 0, 0, 255]));
        img.put_pixel(6, 2, Rgba([200, 0, 0, 255]));
        img.put_pixel(6, 8, Rgba([200, 0, 0, 255]));
        let rect = AxisRect {
            left: 4.0,
            top: 0.0,
            right: 11.0,
            bottom: 11.0,
        };
        let poly = [(4.0, 4.0), (11.0, 4.0), (11.0, 11.0), (4.0, 11.0)];
        let out = apply(&img, [200, 0, 0], 10, Some(rect), Some(&poly));
        assert_eq!(out.get_pixel(2, 8), &WHITE);
        assert_eq!(out.get_pixel(6, 2), &WHITE);
        assert_eq!(out.get_pixel(6, 8), &Rgba([200, 0, 0, 255]));
    }

    #[test]
    fn degenerate_mask_is_ignored() {
        let mut img = RgbaImage::from_pixel(4, 4, WHITE);
        img.put_pixel(1, 1, Rgba([200, 0, 0, 255]));
        let out = apply(&img, [200, 0, 0], 10, None, Some(&[(1.0, 1.0), (3.0, 3.0)]));
        assert_eq!(out.get_pixel(1, 1), &Rgba([200, 0, 0, 255]));
    }
}
