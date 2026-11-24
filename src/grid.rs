use image::RgbaImage;

use crate::axis::AxisRect;
use crate::image_state::{WHITE, is_data_px};

pub const GRIDLINE_COVERAGE: f32 = 0.85;
pub const SHOULDER_COVERAGE: f32 = 0.5;
pub const MIN_SPAN: i64 = 3;

pub fn remove_gridlines(img: &mut RgbaImage, rect: AxisRect) -> (usize, usize) {
    let Some((x0, y0, x1, y1)) = interior(&rect) else {
        return (0, 0);
    };
    let cols = line_columns(img, x0, y0, x1, y1);
    erase_columns(img, &cols, y0, y1);
    let rows = line_rows(img, x0, y0, x1, y1);
    erase_rows(img, &rows, x0, x1);
    (cols.len(), rows.len())
}

fn interior(rect: &AxisRect) -> Option<(i64, i64, i64, i64)> {
    let x0 = (rect.left.ceil() as i64).max(0) + 1;
    let x1 = rect.right.floor() as i64 - 1;
    let y0 = (rect.top.ceil() as i64).max(0) + 1;
    let y1 = rect.bottom.floor() as i64 - 1;
    if x1 < x0 || y1 < y0 || x1 - x0 + 1 < MIN_SPAN || y1 - y0 + 1 < MIN_SPAN {
        return None;
    }
    Some((x0, y0, x1, y1))
}

fn dark_column_counts(img: &RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<u32> {
    let mut counts = Vec::with_capacity((x1 - x0 + 1) as usize);
    for x in x0..=x1 {
        let mut n = 0u32;
        for y in y0..=y1 {
            if is_data_px(img.get_pixel(x as u32, y as u32)) {
                n += 1;
            }
        }
        counts.push(n);
    }
    counts
}

fn dark_row_counts(img: &RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<u32> {
    let mut counts = Vec::with_capacity((y1 - y0 + 1) as usize);
    for y in y0..=y1 {
        let mut n = 0u32;
        for x in x0..=x1 {
            if is_data_px(img.get_pixel(x as u32, y as u32)) {
                n += 1;
            }
        }
        counts.push(n);
    }
    counts
}

fn spike_indices(counts: &[u32], span: f32) -> Vec<usize> {
    let mut marks = vec![false; counts.len()];
    for (i, &n) in counts.iter().enumerate() {
        marks[i] = n as f32 >= GRIDLINE_COVERAGE * span;
    }
    for i in 1..counts.len() {
        if !marks[i] && counts[i] as f32 >= SHOULDER_COVERAGE * span && marks[i - 1] {
            marks[i] = true;
        }
    }
    for i in (1..counts.len()).rev() {
        if !marks[i] && counts[i] as f32 >= SHOULDER_COVERAGE * span && marks[i + 1] {
            marks[i] = true;
        }
    }
    marks
        .iter()
        .enumerate()
        .filter_map(|(i, m)| m.then_some(i))
        .collect()
}

fn line_columns(img: &RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<i64> {
    let span = (y1 - y0 + 1) as f32;
    spike_indices(&dark_column_counts(img, x0, y0, x1, y1), span)
        .into_iter()
        .map(|i| x0 + i as i64)
        .collect()
}

fn line_rows(img: &RgbaImage, x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<i64> {
    let span = (x1 - x0 + 1) as f32;
    spike_indices(&dark_row_counts(img, x0, y0, x1, y1), span)
        .into_iter()
        .map(|i| y0 + i as i64)
        .collect()
}

fn erase_columns(img: &mut RgbaImage, cols: &[i64], y0: i64, y1: i64) {
    for &x in cols {
        for y in y0..=y1 {
            img.put_pixel(x as u32, y as u32, WHITE);
        }
    }
}

fn erase_rows(img: &mut RgbaImage, rows: &[i64], x0: i64, x1: i64) {
    for &y in rows {
        for x in x0..=x1 {
            img.put_pixel(x as u32, y as u32, WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    const INK: Rgba<u8> = Rgba([0, 0, 0, 255]);

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, WHITE)
    }

    fn rect() -> AxisRect {
        AxisRect {
            left: 10.0,
            top: 10.0,
            right: 90.0,
            bottom: 70.0,
        }
    }

    fn fill_v(img: &mut RgbaImage, x: i64, y0: i64, y1: i64) {
        for y in y0..=y1 {
            img.put_pixel(x as u32, y as u32, INK);
        }
    }

    fn fill_h(img: &mut RgbaImage, y: i64, x0: i64, x1: i64) {
        for x in x0..=x1 {
            img.put_pixel(x as u32, y as u32, INK);
        }
    }

    #[test]
    fn removes_grid_cross_inside_rect_only() {
        let mut img = blank(100, 80);
        img.put_pixel(2, 2, INK);
        fill_v(&mut img, 10, 10, 70);
        fill_v(&mut img, 90, 10, 70);
        fill_h(&mut img, 10, 10, 90);
        fill_h(&mut img, 70, 10, 90);
        fill_v(&mut img, 50, 11, 69);
        fill_h(&mut img, 40, 11, 89);
        let (nv, nh) = remove_gridlines(&mut img, rect());
        assert_eq!((nv, nh), (1, 1));
        assert_eq!(img.get_pixel(50, 30), &WHITE);
        assert_eq!(img.get_pixel(30, 40), &WHITE);
        assert!(is_data_px(img.get_pixel(2, 2)));
        assert!(is_data_px(img.get_pixel(10, 40)));
        assert!(is_data_px(img.get_pixel(50, 10)));
        assert!(is_data_px(img.get_pixel(90, 40)));
    }

    #[test]
    fn steep_partial_column_survives() {
        let mut img = blank(100, 80);
        for y in 15..=60 {
            img.put_pixel(50, y, INK);
        }
        let (nv, nh) = remove_gridlines(&mut img, rect());
        assert_eq!((nv, nh), (0, 0));
        assert!(is_data_px(img.get_pixel(50, 30)));
    }

    #[test]
    fn antialiased_shoulder_of_thick_line_removed() {
        let mut img = blank(100, 80);
        fill_v(&mut img, 50, 11, 69);
        for y in (11..=69).step_by(2) {
            img.put_pixel(51, y, Rgba([120, 120, 120, 255]));
        }
        for y in (11..=69).step_by(3) {
            img.put_pixel(52, y, Rgba([120, 120, 120, 255]));
        }
        let (nv, nh) = remove_gridlines(&mut img, rect());
        assert_eq!((nv, nh), (2, 0));
        assert_eq!(img.get_pixel(50, 30), &WHITE);
        assert_eq!(img.get_pixel(51, 30), &WHITE);
        assert!(is_data_px(img.get_pixel(52, 32)));
    }

    #[test]
    fn thick_line_all_columns_removed() {
        let mut img = blank(100, 80);
        fill_v(&mut img, 50, 11, 69);
        fill_v(&mut img, 51, 11, 69);
        fill_v(&mut img, 52, 11, 69);
        let (nv, _) = remove_gridlines(&mut img, rect());
        assert_eq!(nv, 3);
    }

    #[test]
    fn interior_too_small_is_noop() {
        let mut img = blank(8, 8);
        fill_v(&mut img, 4, 1, 6);
        let r = AxisRect {
            left: 1.0,
            top: 1.0,
            right: 4.0,
            bottom: 4.0,
        };
        let (nv, nh) = remove_gridlines(&mut img, r);
        assert_eq!((nv, nh), (0, 0));
        assert!(is_data_px(img.get_pixel(4, 3)));
    }

    #[test]
    fn curve_survives_gridline_removal() {
        let mut img = blank(100, 80);
        fill_v(&mut img, 50, 11, 69);
        fill_h(&mut img, 40, 11, 89);
        for x in 20..=80 {
            let y = 65 - (x - 20) / 4;
            img.put_pixel(x, y, INK);
        }
        let (nv, nh) = remove_gridlines(&mut img, rect());
        assert_eq!((nv, nh), (1, 1));
        assert!(is_data_px(img.get_pixel(20, 65)));
        assert!(is_data_px(img.get_pixel(80, 50)));
        assert_eq!(img.get_pixel(50, 65), &WHITE);
        assert_eq!(img.get_pixel(78, 40), &WHITE);
    }
}
