#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod axis;
mod capture;
mod edit;
mod export;
mod filter;
mod grid;
mod image_state;
mod interp;
mod loader;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "digitizeit",
        options,
        Box::new(|_cc| Ok(Box::new(app::App::new()))),
    )
}

#[cfg(test)]
mod e2e {
    use crate::axis::{AxisRange, Calibration, detect_axis_rect};
    use crate::capture::{self, Dataset};
    use crate::export::{self, Format};
    use crate::filter;
    use crate::image_state::WHITE;
    use image::Rgba;
    use image::RgbaImage;
    use imageproc::drawing::draw_line_segment_mut;

    const INK: Rgba<u8> = Rgba([0, 0, 0, 255]);
    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);

    fn synthetic_plot() -> RgbaImage {
        let mut img = RgbaImage::from_pixel(300, 200, Rgba([255, 255, 255, 255]));
        draw_line_segment_mut(&mut img, (20.0, 20.0), (280.0, 20.0), INK);
        draw_line_segment_mut(&mut img, (20.0, 180.0), (280.0, 180.0), INK);
        draw_line_segment_mut(&mut img, (20.0, 20.0), (20.0, 180.0), INK);
        draw_line_segment_mut(&mut img, (280.0, 20.0), (280.0, 180.0), INK);
        draw_line_segment_mut(&mut img, (25.0, 150.0), (275.0, 50.0), RED);
        img
    }

    #[test]
    fn detect_filter_capture_export_linear() {
        let img = synthetic_plot();
        let rect = detect_axis_rect(&img).expect("frame detected");
        assert!((rect.left - 20.0).abs() <= 1.0);
        assert!((rect.top - 20.0).abs() <= 1.0);
        assert!((rect.right - 280.0).abs() <= 1.0);
        assert!((rect.bottom - 180.0).abs() <= 1.0);

        let filtered = filter::apply(&img, [255, 0, 0], 30, Some(rect), None);
        let pts = capture::capture_continuous(&filtered);
        assert!(pts.len() > 100, "expected a dense line, got {}", pts.len());
        assert!(
            pts.first().unwrap().0 < pts.last().unwrap().0,
            "sorted by x"
        );

        let calib = Calibration {
            x: AxisRange::linear(0.0, 10.0),
            y: AxisRange::linear(0.0, 1.0),
        };
        let mut ds = Dataset::new(1, "line", [255, 0, 0]);
        ds.points = pts;
        let rows = export::dataset_rows(&calib, &rect, &ds.points);
        let (first_x, first_y) = rows[0];
        let (last_x, last_y) = rows[rows.len() - 1];
        assert!((first_x - 0.19).abs() < 0.1, "first_x {first_x}");
        assert!((first_y - 0.1875).abs() < 0.05, "first_y {first_y}");
        assert!((last_x - 9.81).abs() < 0.1, "last_x {last_x}");
        assert!((last_y - 0.8125).abs() < 0.05, "last_y {last_y}");

        let csv = export::dataset_table(Format::Csv, "line", &calib, &rect, &ds.points);
        let parsed: Vec<Vec<f64>> = csv
            .lines()
            .skip(1)
            .map(|l| l.split(',').map(|v| v.parse::<f64>().unwrap()).collect())
            .collect();
        assert_eq!(parsed.len(), rows.len());
    }

    #[test]
    fn detect_filter_capture_discrete_and_log_export() {
        let mut img = synthetic_plot();
        for x in 100..106 {
            for y in 100..106 {
                img.put_pixel(x, y, RED);
            }
        }
        let rect = detect_axis_rect(&img).unwrap();
        let filtered = filter::apply(&img, [255, 0, 0], 30, Some(rect), None);
        let pts = capture::capture_discrete(&filtered, 4);
        assert!(
            pts.iter()
                .any(|(x, y)| (x - 102.5).abs() <= 0.5 && (y - 102.5).abs() <= 0.5)
        );

        let calib = Calibration {
            x: AxisRange {
                min: 1.0,
                max: 100.0,
                scale: crate::axis::Scale::Log { base: 10.0 },
            },
            y: AxisRange::linear(0.0, 1.0),
        };
        let center_px = (102.5, 102.5);
        let (dx, _) = calib.pixel_to_data(&rect, center_px);
        let t = ((102.5 - rect.left) / rect.width()) as f64;
        assert!((dx - 10f64.powf(t * 2.0)).abs() < 1e-6);
        let csv = export::dataset_table(Format::Csv, "blobs", &calib, &rect, &[center_px]);
        assert!(csv.lines().nth(1).unwrap().starts_with("4.3"));
    }

    #[test]
    fn crop_filter_gridlines_capture_pipeline() {
        let mut img = synthetic_plot();
        draw_line_segment_mut(&mut img, (60.0, 160.0), (240.0, 40.0), INK);
        for y in 21..180 {
            img.put_pixel(150, y, INK);
        }
        for x in 21..280 {
            img.put_pixel(x, 110, INK);
        }
        img.put_pixel(5, 5, INK);
        let rect = detect_axis_rect(&img).unwrap();
        let mut filtered = filter::apply(&img, [0, 0, 0], 10, Some(rect), None);
        assert_eq!(filtered.get_pixel(5, 5), &WHITE);
        assert_eq!(filtered.get_pixel(100, 120), &WHITE);
        assert!(crate::image_state::is_data_px(filtered.get_pixel(100, 133)));
        let (nv, nh) = crate::grid::remove_gridlines(&mut filtered, rect);
        assert_eq!((nv, nh), (1, 1));
        assert_eq!(filtered.get_pixel(150, 60), &WHITE);
        assert_eq!(filtered.get_pixel(200, 110), &WHITE);
        assert!(crate::image_state::is_data_px(filtered.get_pixel(100, 133)));
        let pts = capture::capture_continuous(&filtered);
        let interior_pts: Vec<(f32, f32)> = pts
            .iter()
            .copied()
            .filter(|&(x, y)| x > 20.0 && x < 280.0 && y > 20.0 && y < 180.0)
            .collect();
        assert!(!interior_pts.iter().any(|&(x, _)| x == 150.0));
        assert!(!interior_pts.iter().any(|&(_, y)| y == 110.0));
        assert!(pts.iter().any(|&(x, y)| x == 100.0 && y == 133.0));
    }

    #[test]
    fn eraser_keeps_erased_content_out_of_capture() {
        let mut img = synthetic_plot();
        for x in 150..155 {
            for y in 60..65 {
                img.put_pixel(x, y, RED);
            }
        }
        let rect = detect_axis_rect(&img).unwrap();
        let mut edited = filter::apply(&img, [255, 0, 0], 10, Some(rect), None);
        crate::edit::eraser_stamp(&mut edited, (152.0, 62.0));
        let pts = capture::capture_continuous(&edited);
        assert!(
            !pts.iter()
                .any(|&(x, y)| (140.0..165.0).contains(&x) && (50.0..75.0).contains(&y))
        );
        assert!(pts.iter().any(|&(x, _)| x < 100.0));
    }
}
