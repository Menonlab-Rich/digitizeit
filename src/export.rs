use crate::axis::{AxisRect, Calibration};
use crate::capture::{Dataset, Px};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    Txt,
}

impl Format {
    pub fn separator(self) -> &'static str {
        match self {
            Format::Csv => ",",
            Format::Txt => "\t",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Txt => "txt",
        }
    }
}

pub fn format_value(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let mut s = format!("{v:.6}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" {
        return "0".to_string();
    }
    s
}

pub fn sanitize_name(name: &str) -> String {
    name.replace([',', '\t', '\n', '\r'], " ")
        .trim()
        .to_string()
}

pub fn dataset_rows(calib: &Calibration, rect: &AxisRect, points: &[Px]) -> Vec<(f64, f64)> {
    points
        .iter()
        .map(|p| calib.pixel_to_data(rect, *p))
        .collect()
}

pub fn dataset_table(
    format: Format,
    name: &str,
    calib: &Calibration,
    rect: &AxisRect,
    points: &[Px],
) -> String {
    let sep = format.separator();
    let rows = dataset_rows(calib, rect, points);
    let mut out = format!("x{sep}{}\n", sanitize_name(name));
    for (x, y) in rows {
        out.push_str(&format_value(x));
        out.push_str(sep);
        out.push_str(&format_value(y));
        out.push('\n');
    }
    out
}

pub fn combined_table(
    format: Format,
    calib: &Calibration,
    rect: &AxisRect,
    datasets: &[Dataset],
) -> String {
    let sep = format.separator();
    let mut per_ds: Vec<Vec<Option<f64>>> = Vec::new();
    for _ in datasets {
        per_ds.push(Vec::new());
    }
    let mut keys: Vec<String> = Vec::new();
    let mut index: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (di, ds) in datasets.iter().enumerate() {
        for p in &ds.points {
            let (x, y) = calib.pixel_to_data(rect, *p);
            let key = format_value(x);
            let slot = match index.get(&key) {
                Some(i) => *i,
                None => {
                    let i = keys.len();
                    keys.push(key.clone());
                    for col in per_ds.iter_mut() {
                        col.push(None);
                    }
                    index.insert(key, i);
                    i
                }
            };
            per_ds[di][slot] = Some(y);
        }
    }
    let mut out = String::new();
    out.push('x');
    for ds in datasets {
        out.push_str(sep);
        out.push_str(&sanitize_name(&ds.name));
    }
    out.push('\n');
    for (i, key) in keys.iter().enumerate() {
        out.push_str(key);
        for col in &per_ds {
            out.push_str(sep);
            match col[i] {
                Some(y) => out.push_str(&format_value(y)),
                None => out.push_str(""),
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::axis::{AxisRange, AxisRect, Scale};

    fn calib() -> Calibration {
        Calibration {
            x: AxisRange::linear(0.0, 100.0),
            y: AxisRange::linear(0.0, 10.0),
        }
    }

    fn rect() -> AxisRect {
        AxisRect {
            left: 10.0,
            top: 20.0,
            right: 110.0,
            bottom: 120.0,
        }
    }

    #[test]
    fn value_formatting_trims_zeros() {
        assert_eq!(format_value(1.0), "1");
        assert_eq!(format_value(0.5), "0.5");
        assert_eq!(format_value(-2.25), "-2.25");
        assert_eq!(format_value(-0.0), "0");
        assert_eq!(format_value(-0.0000001), "0");
        assert_eq!(format_value(123.456789), "123.456789");
        assert_eq!(format_value(0.0), "0");
    }

    #[test]
    fn csv_roundtrip() {
        let points = vec![(10.0, 120.0), (60.0, 70.0), (110.0, 20.0)];
        let text = dataset_table(Format::Csv, "line a", &calib(), &rect(), &points);
        let mut lines = text.lines();
        assert_eq!(lines.next().unwrap(), "x,line a");
        let rows: Vec<Vec<f64>> = lines
            .map(|l| l.split(',').map(|v| v.parse().unwrap()).collect())
            .collect();
        assert_eq!(
            rows,
            vec![vec![0.0, 0.0], vec![50.0, 5.0], vec![100.0, 10.0]]
        );
    }

    #[test]
    fn txt_uses_tabs() {
        let points = vec![(10.0, 120.0)];
        let text = dataset_table(Format::Txt, "d", &calib(), &rect(), &points);
        assert_eq!(text, "x\td\n0\t0\n");
    }

    #[test]
    fn combined_aligns_by_x() {
        let mut a = Dataset::new(1, "a", [255, 0, 0]);
        a.points = vec![(10.0, 120.0), (60.0, 70.0)];
        let mut b = Dataset::new(2, "b", [0, 0, 255]);
        b.points = vec![(60.0, 60.0), (110.0, 20.0)];
        let text = combined_table(Format::Csv, &calib(), &rect(), &[a, b]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "x,a,b");
        assert_eq!(lines[1], "0,0,");
        assert_eq!(lines[2], "50,5,6");
        assert_eq!(lines[3], "100,,10");
    }

    #[test]
    fn log_export_writes_data_values() {
        let calib = Calibration {
            x: AxisRange {
                min: 1.0,
                max: 100.0,
                scale: Scale::Log { base: 10.0 },
            },
            y: AxisRange::linear(0.0, 1.0),
        };
        let points = vec![(10.0, 120.0), (60.0, 70.0), (110.0, 20.0)];
        let text = dataset_table(Format::Csv, "d", &calib, &rect(), &points);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], "1,0");
        assert_eq!(lines[2], "10,0.5");
        assert_eq!(lines[3], "100,1");
    }

    #[test]
    fn sanitize_replaces_separators() {
        assert_eq!(sanitize_name("a,b\tc"), "a b c");
    }
}
