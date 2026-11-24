use image::{GrayImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_filled_rect_mut, draw_line_segment_mut};
use imageproc::region_labelling::{Connectivity, connected_components};

use crate::image_state::{WHITE, is_data_px};

pub const UNDO_CAP: usize = 20;
pub const ERASER_RADIUS: i32 = 4;
pub const PEN_RADIUS: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Pen,
    Eraser,
    RectErase,
    PolygonErase,
}

#[derive(Clone, Debug, Default)]
pub struct EditState {
    pub tool: Tool,
    pub polygon: Vec<(f32, f32)>,
    undo: Vec<RgbaImage>,
    redo: Vec<RgbaImage>,
}

impl EditState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.polygon.clear();
    }

    pub fn snapshot(&mut self, current: &RgbaImage) {
        self.undo.push(current.clone());
        if self.undo.len() > UNDO_CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, current: &mut RgbaImage) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(current.clone());
                if self.redo.len() > UNDO_CAP {
                    self.redo.remove(0);
                }
                *current = prev;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, current: &mut RgbaImage) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(current.clone());
                if self.undo.len() > UNDO_CAP {
                    self.undo.remove(0);
                }
                *current = next;
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn cancel_polygon(&mut self) {
        self.polygon.clear();
    }

    pub fn can_close_polygon(&self) -> bool {
        self.polygon.len() >= 3
    }
}

pub fn pen_stroke(img: &mut RgbaImage, from: (f32, f32), to: (f32, f32)) {
    let black = Rgba([0, 0, 0, 255]);
    if (from.0 - to.0).abs() < 0.5 && (from.1 - to.1).abs() < 0.5 {
        draw_filled_circle_mut(img, (from.0 as i32, from.1 as i32), PEN_RADIUS, black);
    } else {
        draw_line_segment_mut(img, from, to, black);
        draw_filled_circle_mut(img, (to.0 as i32, to.1 as i32), PEN_RADIUS, black);
    }
}

pub fn eraser_stamp(img: &mut RgbaImage, at: (f32, f32)) {
    let (w, h) = img.dimensions();
    let cx = at.0 as i32;
    let cy = at.1 as i32;
    let r = ERASER_RADIUS;
    for y in (cy - r)..=(cy + r) {
        for x in (cx - r)..=(cx + r) {
            let dx = (x - cx) * (x - cx);
            let dy = (y - cy) * (y - cy);
            if dx + dy > r * r || x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                continue;
            }
            img.put_pixel(x as u32, y as u32, WHITE);
        }
    }
}

pub fn erase_rect(img: &mut RgbaImage, a: (f32, f32), b: (f32, f32)) {
    let (w, h) = img.dimensions();
    let x0 = a.0.min(b.0).floor().max(0.0) as i32;
    let y0 = a.1.min(b.1).floor().max(0.0) as i32;
    let x1 = (a.0.max(b.0).ceil() as i32).min(w as i32);
    let y1 = (a.1.max(b.1).ceil() as i32).min(h as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    draw_filled_rect_mut(
        img,
        imageproc::rect::Rect::at(x0, y0).of_size((x1 - x0) as u32, (y1 - y0) as u32),
        WHITE,
    );
}

pub fn fill_polygon<I, P>(img: &mut I, poly: &[(f32, f32)], color: P)
where
    I: image::GenericImage<Pixel = P>,
    P: image::Pixel<Subpixel = u8>,
{
    if poly.len() < 3 {
        return;
    }
    let (w, h) = img.dimensions();
    let ymin = (poly.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32).max(0);
    let ymax = (poly.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32).min(h as i32 - 1);
    let n = poly.len();
    for y in ymin..=ymax {
        let sy = y as f32 + 0.5;
        let mut crossings: Vec<f32> = Vec::new();
        for i in 0..n {
            let (x1, y1) = poly[i];
            let (x2, y2) = poly[(i + 1) % n];
            let (ya, yb) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
            if (y1 <= sy && y2 > sy) || (y2 <= sy && y1 > sy) {
                if ya == yb {
                    continue;
                }
                let t = (sy - ya) / (yb - ya);
                crossings.push(x1 + t * (x2 - x1));
            }
        }
        crossings.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in crossings.chunks(2) {
            if pair.len() < 2 {
                continue;
            }
            let xa = (pair[0].ceil() as i32).max(0);
            let xb = (pair[1].ceil() as i32).min(w as i32);
            for x in xa..xb {
                img.put_pixel(x as u32, y as u32, color);
            }
        }
    }
}

pub fn erase_polygon(img: &mut RgbaImage, poly: &[(f32, f32)]) {
    fill_polygon(img, poly, WHITE);
}

pub fn data_mask(img: &RgbaImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut mask = GrayImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels() {
        if is_data_px(p) {
            mask.put_pixel(x, y, image::Luma([255]));
        }
    }
    mask
}

pub fn clear_border_components(mask: &mut GrayImage) {
    let labels = connected_components(mask, Connectivity::Eight, image::Luma([0u8]));
    let (w, h) = mask.dimensions();
    let mut border_labels: Vec<u32> = Vec::new();
    for x in 0..w {
        let l = labels.get_pixel(x, 0).0[0];
        if l > 0 {
            border_labels.push(l);
        }
        let l = labels.get_pixel(x, h - 1).0[0];
        if l > 0 {
            border_labels.push(l);
        }
    }
    for y in 0..h {
        let l = labels.get_pixel(0, y).0[0];
        if l > 0 {
            border_labels.push(l);
        }
        let l = labels.get_pixel(w - 1, y).0[0];
        if l > 0 {
            border_labels.push(l);
        }
    }
    if border_labels.is_empty() {
        return;
    }
    for (x, y, l) in labels.enumerate_pixels() {
        if border_labels.contains(&l.0[0]) {
            mask.put_pixel(x, y, image::Luma([0]));
        }
    }
}

pub fn clear_border(img: &mut RgbaImage) {
    let mut mask = data_mask(img);
    clear_border_components(&mut mask);
    for (x, y, p) in mask.enumerate_pixels() {
        if p.0[0] == 0 && is_data_px(img.get_pixel(x, y)) {
            img.put_pixel(x, y, WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_state::ImageState;

    fn blank(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, WHITE)
    }

    fn count(img: &RgbaImage, f: impl Fn(&Rgba<u8>) -> bool) -> usize {
        img.pixels().filter(|p| f(p)).count()
    }

    #[test]
    fn pen_draws_and_undo_restores() {
        let st = ImageState::new(blank(20, 20));
        let mut edit = EditState::new();
        let mut img = st.edited.clone();
        edit.snapshot(&img);
        pen_stroke(&mut img, (5.0, 5.0), (10.0, 5.0));
        assert!(count(&img, |p| p == &Rgba([0, 0, 0, 255])) > 0);
        assert!(edit.undo(&mut img));
        assert_eq!(count(&img, |p| p == &Rgba([0, 0, 0, 255])), 0);
        assert!(edit.redo(&mut img));
        assert!(count(&img, |p| p == &Rgba([0, 0, 0, 255])) > 0);
        assert!(edit.undo(&mut img));
        assert_eq!(count(&img, |p| p == &Rgba([0, 0, 0, 255])), 0);
        assert!(!edit.undo(&mut img));
    }

    #[test]
    fn pen_click_draws_dot() {
        let mut img = blank(10, 10);
        pen_stroke(&mut img, (5.0, 5.0), (5.0, 5.0));
        assert_eq!(img.get_pixel(5, 5), &Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn eraser_blanks_ink_to_white() {
        let mut img = blank(20, 20);
        img.put_pixel(10, 10, Rgba([255, 0, 0, 255]));
        img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
        eraser_stamp(&mut img, (10.0, 10.0));
        assert_eq!(img.get_pixel(10, 10), &WHITE);
        assert!(is_data_px(img.get_pixel(5, 5)));
    }

    #[test]
    fn erase_rect_clears_region() {
        let mut img = blank(20, 20);
        img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
        erase_rect(&mut img, (3.0, 3.0), (8.0, 8.0));
        assert_eq!(img.get_pixel(5, 5), &WHITE);
        assert!(count(&img, |p| p == &Rgba([0, 0, 0, 255])) == 0);
    }

    #[test]
    fn polygon_fill_covers_inside() {
        let mut img = blank(20, 20);
        erase_polygon(
            &mut img,
            &[(5.0, 5.0), (14.0, 5.0), (14.0, 14.0), (5.0, 14.0)],
        );
        assert_eq!(img.get_pixel(9, 9), &WHITE);
        assert_eq!(img.get_pixel(4, 9), &WHITE);
        assert_eq!(img.get_pixel(15, 9), &WHITE);
    }

    #[test]
    fn polygon_fill_concave() {
        let mut img = blank(20, 20);
        let poly = vec![
            (4.0, 4.0),
            (15.0, 4.0),
            (15.0, 15.0),
            (10.0, 15.0),
            (10.0, 8.0),
            (4.0, 8.0),
        ];
        erase_polygon(&mut img, &poly);
        assert_eq!(img.get_pixel(6, 5), &WHITE);
        assert_eq!(img.get_pixel(12, 12), &WHITE);
        assert_eq!(img.get_pixel(6, 12), &WHITE);
    }

    #[test]
    fn undo_stack_is_capped() {
        let mut edit = EditState::new();
        let mut img = blank(5, 5);
        for _ in 0..30 {
            edit.snapshot(&img);
            pen_stroke(&mut img, (2.0, 2.0), (2.0, 2.0));
        }
        let mut undos = 0;
        while edit.undo(&mut img) {
            undos += 1;
        }
        assert_eq!(undos, UNDO_CAP);
    }

    #[test]
    fn snapshot_clears_redo() {
        let mut edit = EditState::new();
        let mut img = blank(5, 5);
        edit.snapshot(&img);
        pen_stroke(&mut img, (2.0, 2.0), (2.0, 2.0));
        assert!(edit.undo(&mut img));
        assert!(edit.can_redo());
        edit.snapshot(&img);
        assert!(!edit.can_redo());
    }

    #[test]
    fn clear_border_removes_border_touching_ink() {
        let mut img = blank(50, 40);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        img.put_pixel(49, 39, Rgba([0, 0, 255, 255]));
        img.put_pixel(20, 20, Rgba([0, 0, 0, 255]));
        clear_border(&mut img);
        assert_eq!(img.get_pixel(0, 0), &WHITE);
        assert_eq!(img.get_pixel(49, 39), &WHITE);
        assert_eq!(img.get_pixel(20, 20), &Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn clear_border_keeps_touching_blob_entirely_removed() {
        let mut img = blank(30, 30);
        for x in 0..5 {
            img.put_pixel(x, 0, Rgba([0, 0, 0, 255]));
            img.put_pixel(x, 3, Rgba([0, 0, 0, 255]));
        }
        for y in 0..4 {
            img.put_pixel(0, y, Rgba([0, 0, 0, 255]));
            img.put_pixel(4, y, Rgba([0, 0, 0, 255]));
        }
        clear_border(&mut img);
        assert_eq!(count(&img, is_data_px), 0);
    }
}
