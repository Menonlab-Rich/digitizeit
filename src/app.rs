use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::axis::{self, AxisRect, Calibration, Scale};
use crate::capture::{self, DATASET_COLORS, Dataset, Px};
use crate::edit::{EditState, Tool};
use crate::export::{self, Format};
use crate::filter::{self, FilterParams};
use crate::grid;
use crate::image_state::ImageState;
use crate::interp::{self, DedupMode, InterpMode};
use crate::loader;

type RgbaImage = image::RgbaImage;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Load,
    Axes,
    Range,
    Filter,
    Edit,
    Data,
    Export,
}

const ALL_STEPS: [Step; 7] = [
    Step::Load,
    Step::Axes,
    Step::Range,
    Step::Filter,
    Step::Edit,
    Step::Data,
    Step::Export,
];

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Load => "Load",
            Step::Axes => "Axes",
            Step::Range => "Range",
            Step::Filter => "Filter",
            Step::Edit => "Edit",
            Step::Data => "Data",
            Step::Export => "Export",
        }
    }

    pub fn index(self) -> usize {
        ALL_STEPS.iter().position(|s| *s == self).unwrap()
    }

    pub fn hint(self) -> &'static str {
        match self {
            Step::Load => "Open, paste, drop or download a plot image",
            Step::Axes => "Auto-detect the axis rectangle or drag its edge handles",
            Step::Range => "Set axis limits; log scales need positive limits",
            Step::Filter => "Pick a line color, mask the curve area, then Apply",
            Step::Edit => "Erase noise; undo/redo with Ctrl+Z / Ctrl+Y",
            Step::Data => "Create datasets and capture points",
            Step::Export => "Write CSV or TXT files",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DataMode {
    Continuous,
    Discrete,
    Manual,
    Adjust,
}

impl DataMode {
    pub fn name(self) -> &'static str {
        match self {
            DataMode::Continuous => "Continuous",
            DataMode::Discrete => "Discrete",
            DataMode::Manual => "Manual",
            DataMode::Adjust => "Adjust",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    fn index(self) -> usize {
        match self {
            Edge::Top => 0,
            Edge::Bottom => 1,
            Edge::Left => 2,
            Edge::Right => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum EditDrag {
    Stroke { last: Px },
    Rect { start: Px },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum FilterTool {
    #[default]
    Pick,
    Mask,
}

#[derive(Clone, Copy, Debug)]
struct View {
    zoom: f32,
    pan: Vec2,
    needs_fit: bool,
}

pub struct App {
    step: Step,
    image: Option<ImageState>,
    texture: Option<egui::TextureHandle>,
    texture_version: u64,
    texture_dirty: bool,
    view: View,
    axis_rect: Option<AxisRect>,
    calib: Calibration,
    filter: FilterParams,
    filter_tool: FilterTool,
    mask_draft: Vec<Px>,
    filter_preview: Option<RgbaImage>,
    edit: EditState,
    edit_drag: Option<EditDrag>,
    export_format: Format,
    export_combined: bool,
    datasets: Vec<Dataset>,
    selected_dataset: usize,
    next_dataset_id: u64,
    data_mode: DataMode,
    min_blob_area: u32,
    manual_step: f32,
    one_to_one: bool,
    dedup_mode: DedupMode,
    interpolate: bool,
    interp_mode: InterpMode,
    hover_px: Option<Px>,
    status: String,
    error: Option<String>,
    url_rx: Option<Receiver<Result<RgbaImage, String>>>,
    url_pending: bool,
    url_input: String,
}

const MIN_ZOOM: f32 = 0.02;
const MAX_ZOOM: f32 = 128.0;
const ADJUST_GRAB_RADIUS: f32 = 10.0;

impl App {
    pub fn new() -> Self {
        Self {
            step: Step::Load,
            image: None,
            texture: None,
            texture_version: 0,
            texture_dirty: false,
            view: View {
                zoom: 1.0,
                pan: Vec2::ZERO,
                needs_fit: true,
            },
            axis_rect: None,
            calib: Calibration::default(),
            filter: FilterParams::default(),
            filter_tool: FilterTool::default(),
            mask_draft: Vec::new(),
            filter_preview: None,
            edit: EditState::new(),
            edit_drag: None,
            export_format: Format::Csv,
            export_combined: true,
            datasets: Vec::new(),
            selected_dataset: 0,
            next_dataset_id: 1,
            data_mode: DataMode::Continuous,
            min_blob_area: 4,
            manual_step: 10.0,
            one_to_one: true,
            dedup_mode: DedupMode::default(),
            interpolate: false,
            interp_mode: InterpMode::default(),
            hover_px: None,
            status: "Welcome to digitizeit".to_string(),
            error: None,
            url_rx: None,
            url_pending: false,
            url_input: String::new(),
        }
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.error = None;
    }

    fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        self.status = "Error".to_string();
    }

    fn set_image(&mut self, img: RgbaImage, msg: &str) {
        self.image = Some(ImageState::new(img));
        self.texture = None;
        self.texture_version = 0;
        self.texture_dirty = false;
        self.view.needs_fit = true;
        self.axis_rect = None;
        self.filter = FilterParams::default();
        self.filter_tool = FilterTool::default();
        self.mask_draft.clear();
        self.filter_preview = None;
        self.edit.reset();
        self.edit_drag = None;
        self.set_status(msg.to_string());
        self.step = Step::Axes;
    }

    fn step_unlocked_reason(&self, step: Step) -> Result<(), &'static str> {
        match step {
            Step::Load => Ok(()),
            Step::Axes => {
                if self.image.is_some() {
                    Ok(())
                } else {
                    Err("Load an image first")
                }
            }
            Step::Range | Step::Filter | Step::Edit | Step::Data => {
                if self.image.is_none() {
                    return Err("Load an image first");
                }
                if self.axis_rect.is_none() {
                    return Err("Detect or draw the axis rectangle in the Axes step");
                }
                Ok(())
            }
            Step::Export => {
                if self.image.is_none() {
                    return Err("Load an image first");
                }
                if self.axis_rect.is_none() {
                    return Err("Detect or draw the axis rectangle in the Axes step");
                }
                if !self.datasets.iter().any(|d| !d.points.is_empty()) {
                    return Err("Capture at least one point in the Data step");
                }
                Ok(())
            }
        }
    }

    fn filter_crop(&self) -> Option<AxisRect> {
        let r = self.axis_rect?;
        let (w, h) = self
            .image
            .as_ref()
            .map(|i| i.dimensions())
            .unwrap_or((0, 0));
        r.is_valid(w, h).then_some(r)
    }

    fn commit_live_filter(&mut self) {
        let Some(picked) = self.filter.picked else {
            self.filter_preview = None;
            return;
        };
        let tol = self.filter.tolerance;
        let crop = self.filter_crop();
        let mask = self.filter.mask.clone();
        let Some(img) = self.image.as_mut() else {
            self.filter_preview = None;
            return;
        };
        let filtered = filter::apply(&img.original, picked, tol, crop, mask.as_deref());
        img.apply_filter(filtered);
        self.edit.reset();
        self.filter_preview = None;
    }

    fn go_to_step(&mut self, step: Step) {
        if self.step == Step::Filter && step != Step::Filter && self.filter_preview.is_some() {
            self.commit_live_filter();
            self.set_status("Filter preview committed");
        }
        self.step = step;
    }

    fn recompute_preview(&mut self) {
        let params = self.filter.clone();
        if let (Some(img), Some(picked)) = (self.image.as_ref(), params.picked) {
            self.filter_preview = Some(filter::apply(
                &img.original,
                picked,
                params.tolerance,
                self.filter_crop(),
                params.mask.as_deref(),
            ));
            self.texture_dirty = true;
        }
    }

    fn close_mask(&mut self) {
        if self.mask_draft.len() < 3 {
            return;
        }
        self.filter.mask = Some(std::mem::take(&mut self.mask_draft));
        self.recompute_preview();
        self.set_status("Mask set — filtering limited to the masked area");
    }

    fn clear_mask(&mut self) {
        self.mask_draft.clear();
        if self.filter.mask.take().is_some() {
            self.recompute_preview();
            self.set_status("Mask cleared");
        } else {
            self.status = "No mask".to_string();
        }
    }

    fn remove_gridlines_op(&mut self) {
        let Some(rect) = self.filter_crop() else {
            self.set_error("Set a valid axis rectangle first");
            return;
        };
        let Some(img) = self.image.as_mut() else {
            self.set_error("Load an image first");
            return;
        };
        self.edit.snapshot(&img.edited);
        let (nv, nh) = grid::remove_gridlines(&mut img.edited, rect);
        img.bump();
        self.set_status(format!("Removed {nv} vertical, {nh} horizontal gridlines"));
    }

    fn close_polygon(&mut self) {
        if let Some(img) = &mut self.image {
            let poly = std::mem::take(&mut self.edit.polygon);
            self.edit.snapshot(&img.edited);
            crate::edit::erase_polygon(&mut img.edited, &poly);
            img.bump();
            self.set_status("Polygon erased");
        }
    }

    fn do_edit_undo(&mut self) {
        if let Some(img) = &mut self.image {
            if self.edit.undo(&mut img.edited) {
                img.bump();
                self.set_status("Undo");
            } else {
                self.status = "Nothing to undo".to_string();
            }
        }
    }

    fn do_edit_redo(&mut self) {
        if let Some(img) = &mut self.image {
            if self.edit.redo(&mut img.edited) {
                img.bump();
                self.set_status("Redo");
            } else {
                self.status = "Nothing to redo".to_string();
            }
        }
    }

    fn clear_border_op(&mut self) {
        if let Some(img) = &mut self.image {
            self.edit.snapshot(&img.edited);
            crate::edit::clear_border(&mut img.edited);
            img.bump();
            self.set_status("Border-connected ink cleared");
        }
    }

    fn manual_click(&mut self, px: Px) {
        if self.datasets.is_empty() {
            self.set_error("Create a dataset first");
            return;
        }
        let step = self.manual_step;
        let idx = self.selected_dataset.min(self.datasets.len() - 1);
        let ds = &mut self.datasets[idx];
        let last = ds.points.last().copied();
        let inter = last
            .map(|l| capture::interpolate_segment(l, px, step))
            .unwrap_or_default();
        let added = inter.len() + 1;
        ds.points.extend_from_slice(&inter);
        ds.points.push(px);
        ds.click_counts.push(added);
        self.status = format!("Point added ({} total)", ds.points.len());
    }

    fn adjust_click(&mut self, px: Px) {
        if self.datasets.is_empty() {
            self.set_error("Create a dataset first");
            return;
        }
        let radius = ADJUST_GRAB_RADIUS / self.view.zoom;
        let idx = self.selected_dataset.min(self.datasets.len() - 1);
        let ds = &mut self.datasets[idx];
        ds.snapshot_points();
        if let Some(i) = capture::nearest_point(&ds.points, px, radius) {
            ds.points.remove(i);
            self.status = format!("Point deleted ({} left)", ds.points.len());
        } else {
            ds.points.push(px);
            self.status = format!("Point inserted ({} total)", ds.points.len());
        }
    }

    fn show_one_to_one_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut one = self.one_to_one;
            ui.checkbox(&mut one, "1:1 (one y per x)");
            self.one_to_one = one;
        });
        if self.one_to_one {
            ui.horizontal(|ui| {
                ui.label("duplicate x:");
                egui::ComboBox::from_id_salt("dedup_mode")
                    .selected_text(self.dedup_mode.name())
                    .show_ui(ui, |ui| {
                        for m in [DedupMode::Lowest, DedupMode::Highest, DedupMode::Middle] {
                            ui.selectable_value(&mut self.dedup_mode, m, m.name());
                        }
                    });
            });
        }
    }

    fn capture_selected(&mut self, mode: DataMode) {
        if self.filter_preview.is_some() {
            self.commit_live_filter();
        }
        let Some(img) = self.image.as_ref() else {
            self.set_error("Load an image first");
            return;
        };
        if self.datasets.is_empty() {
            self.set_error("Create a dataset first");
            return;
        }
        let mut pts = match mode {
            DataMode::Continuous => capture::capture_continuous(&img.edited),
            DataMode::Discrete => capture::capture_discrete(&img.edited, self.min_blob_area),
            _ => return,
        };
        let mut note = String::new();
        if self.one_to_one || (self.interpolate && mode == DataMode::Continuous) {
            pts = interp::dedup(&pts, self.dedup_mode);
            note.push_str(" (1:1");
            if self.interpolate && mode == DataMode::Continuous {
                pts = interp::resample(&pts, self.interp_mode);
                note.push_str(&format!(", {} interpolated", self.interp_mode.name()));
            }
            note.push(')');
        }
        let idx = self.selected_dataset.min(self.datasets.len() - 1);
        let count;
        {
            let ds = &mut self.datasets[idx];
            ds.snapshot_points();
            ds.points = pts;
            ds.clear_capture_history();
            count = ds.points.len();
        }
        self.set_status(format!("Captured {count} points{note}"));
    }

    fn undo_last_click(&mut self) {
        if self.datasets.is_empty() {
            self.status = "Nothing to undo".to_string();
            return;
        }
        let idx = self.selected_dataset.min(self.datasets.len() - 1);
        let ds = &mut self.datasets[idx];
        let n = ds.click_counts.pop().unwrap_or(0);
        if n > 0 {
            let new_len = ds.points.len().saturating_sub(n);
            ds.points.truncate(new_len);
            self.status = "Removed last point".to_string();
        } else {
            self.status = "Nothing to undo".to_string();
        }
    }

    fn undo_dataset_points(&mut self) {
        if self.datasets.is_empty() {
            self.status = "Nothing to undo".to_string();
            return;
        }
        let idx = self.selected_dataset.min(self.datasets.len() - 1);
        let ds = &mut self.datasets[idx];
        if ds.undo_points() {
            self.set_status("Undo");
        } else {
            self.status = "Nothing to undo".to_string();
        }
    }

    fn export_outputs(&self) -> Option<Vec<(String, String)>> {
        let rect = self.axis_rect?;
        let mut out = Vec::new();
        if self.export_combined {
            out.push((
                "combined".to_string(),
                export::combined_table(self.export_format, &self.calib, &rect, &self.datasets),
            ));
        } else {
            for ds in &self.datasets {
                let name = export::sanitize_name(&ds.name);
                let name = if name.is_empty() {
                    format!("dataset-{}", ds.id)
                } else {
                    name
                };
                out.push((
                    name,
                    export::dataset_table(
                        self.export_format,
                        &ds.name,
                        &self.calib,
                        &rect,
                        &ds.points,
                    ),
                ));
            }
        }
        Some(out)
    }

    fn do_export(&mut self) {
        if let Err(e) = self.calib.validate() {
            self.set_error(e);
            return;
        }
        let Some(outputs) = self.export_outputs() else {
            self.set_error("Nothing to export");
            return;
        };
        let ext = self.export_format.extension();
        let dialog = rfd::FileDialog::new()
            .add_filter(format!("{} file", ext.to_uppercase()), &[ext])
            .set_file_name(format!("digitizeit.{ext}"));
        let Some(path) = dialog.save_file() else {
            return;
        };
        let mut written: Vec<String> = Vec::new();
        if self.export_combined {
            let Some((_, content)) = outputs.first() else {
                self.set_error("Nothing to export");
                return;
            };
            match std::fs::write(&path, content) {
                Ok(()) => written.push(path.display().to_string()),
                Err(e) => {
                    self.set_error(format!("Write failed: {e}"));
                    return;
                }
            }
        } else {
            let (stem, dir) = {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "digitizeit".to_string());
                let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                (stem, dir)
            };
            for (i, (_, content)) in outputs.iter().enumerate() {
                let name = &outputs[i].0;
                let fname = format!("{}_{i:02}_{name}.{ext}", stem);
                let p = dir.join(fname);
                match std::fs::write(&p, content) {
                    Ok(()) => written.push(p.display().to_string()),
                    Err(e) => {
                        self.set_error(format!("Write failed: {e}"));
                        return;
                    }
                }
            }
        }
        self.status = format!("Saved: {}", written.join(", "));
    }

    fn fit_view(&mut self, rect: Rect, img_w: f32, img_h: f32) {
        let pad = 24.0;
        let zoom = ((rect.width() - pad) / img_w)
            .min((rect.height() - pad) / img_h)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        self.view.zoom = zoom;
        self.view.pan = Vec2::new(
            (rect.width() - img_w * zoom) * 0.5,
            (rect.height() - img_h * zoom) * 0.5,
        );
    }

    fn clamp_view(&mut self, rect: Rect, img_w: f32, img_h: f32) {
        let size = Vec2::new(img_w, img_h) * self.view.zoom;
        let m = 60.0;
        let x0 = -(size.x - m);
        let x1 = rect.width() - m;
        let y0 = -(size.y - m);
        let y1 = rect.height() - m;
        if x0 <= x1 {
            self.view.pan.x = self.view.pan.x.clamp(x0, x1);
        }
        if y0 <= y1 {
            self.view.pan.y = self.view.pan.y.clamp(y0, y1);
        }
    }

    fn zoom_at(&mut self, rect: Rect, cursor: Pos2, factor: f32) {
        let local = cursor - rect.min;
        let img_pt = (local - self.view.pan) / self.view.zoom;
        let new_zoom = (self.view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.view.pan = local - img_pt * new_zoom;
        self.view.zoom = new_zoom;
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        let disp: Option<&RgbaImage> = match (&self.image, &self.filter_preview) {
            (Some(img), Some(prev)) if self.step == Step::Filter => Some(prev),
            (Some(img), _) => Some(&img.edited),
            (None, _) => None,
        };
        let Some(disp) = disp else {
            return;
        };
        let version = self.image.as_ref().map(|i| i.version).unwrap_or(0);
        let needs = self.texture.is_none() || self.texture_version != version || self.texture_dirty;
        if !needs {
            return;
        }
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [disp.width() as usize, disp.height() as usize],
            disp.as_raw(),
        );
        let opts = egui::TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        };
        let existing = self.texture.take();
        let handle = match existing {
            Some(mut t) => {
                t.set(color, opts);
                t
            }
            None => ctx.load_texture("digitizeit-image", color, opts),
        };
        self.texture = Some(handle);
        self.texture_version = version;
        self.texture_dirty = false;
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size().max(Vec2::splat(60.0));
        let (rect, mut response) = ui.allocate_exact_size(available, Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(rect);
        let dims = self
            .image
            .as_ref()
            .map(|i| i.dimensions())
            .unwrap_or((800, 600));
        let img_w = dims.0 as f32;
        let img_h = dims.1 as f32;

        if self.view.needs_fit {
            self.fit_view(rect, img_w, img_h);
            self.view.needs_fit = false;
        }

        if response.hovered() {
            let zoom_delta = ui.input(|i| i.zoom_delta());
            let scroll = ui.input(|i| i.smooth_scroll_delta());
            let factor = if zoom_delta != 1.0 {
                zoom_delta
            } else if scroll.y != 0.0 {
                (-scroll.y * 0.002).exp()
            } else {
                1.0
            };
            if factor != 1.0
                && let Some(cursor) = response.hover_pos()
            {
                self.zoom_at(rect, cursor, factor);
            }
        }

        self.clamp_view(rect, img_w, img_h);
        let mut topleft = rect.min + self.view.pan;
        let zoom = self.view.zoom;

        let s2p = |sp: Pos2| -> Px { ((sp.x - topleft.x) / zoom, (sp.y - topleft.y) / zoom) };
        let p2s = |p: Px| -> Pos2 { Pos2::new(topleft.x + p.0 * zoom, topleft.y + p.1 * zoom) };

        let mut pan_blocked = false;

        match self.step {
            Step::Axes => {
                if let Some(r) = self.axis_rect.as_mut() {
                    let mids: [(Edge, Px); 4] = [
                        (Edge::Top, ((r.left + r.right) * 0.5, r.top)),
                        (Edge::Bottom, ((r.left + r.right) * 0.5, r.bottom)),
                        (Edge::Left, (r.left, (r.top + r.bottom) * 0.5)),
                        (Edge::Right, (r.right, (r.top + r.bottom) * 0.5)),
                    ];
                    for (edge, mid) in mids {
                        let sp = p2s(mid);
                        let handle_id = egui::Id::unique(("axis_handle", edge.index()));
                        let hresp = ui.interact(
                            Rect::from_center_size(sp, Vec2::splat(20.0)),
                            handle_id,
                            Sense::drag(),
                        );
                        if hresp.hovered() {
                            painter.circle_filled(sp, 7.0, Color32::from_rgb(0, 200, 255));
                            pan_blocked = true;
                        }
                        painter.circle_filled(sp, 5.0, Color32::from_rgb(0, 170, 230));
                        painter.circle_filled(sp, 2.0, Color32::WHITE);
                        if hresp.dragged() {
                            let d = hresp.drag_delta();
                            match edge {
                                Edge::Top => {
                                    r.top = (r.top + d.y / zoom).clamp(0.0, r.bottom - 1.0);
                                }
                                Edge::Bottom => {
                                    r.bottom = (r.bottom + d.y / zoom).clamp(r.top + 1.0, img_h);
                                }
                                Edge::Left => {
                                    r.left = (r.left + d.x / zoom).clamp(0.0, r.right - 1.0);
                                }
                                Edge::Right => {
                                    r.right = (r.right + d.x / zoom).clamp(r.left + 1.0, img_w);
                                }
                            }
                            pan_blocked = true;
                        }
                    }
                }
            }
            Step::Edit => {
                let drag_tool =
                    matches!(self.edit.tool, Tool::Pen | Tool::Eraser | Tool::RectErase);
                if drag_tool {
                    pan_blocked = true;
                }
                if let Some(img) = self.image.as_mut() {
                    match self.edit.tool {
                        Tool::Pen => {
                            if response.drag_started_by(egui::PointerButton::Primary)
                                && let Some(p) = response.interact_pointer_pos()
                            {
                                self.edit.snapshot(&img.edited);
                                let p = s2p(p);
                                crate::edit::pen_stroke(&mut img.edited, p, p);
                                img.bump();
                                self.edit_drag = Some(EditDrag::Stroke { last: p });
                            }
                            if let Some(EditDrag::Stroke { last }) = self.edit_drag
                                && response.dragged_by(egui::PointerButton::Primary)
                                && let Some(sp) = response.interact_pointer_pos()
                            {
                                let cur = s2p(sp);
                                if (cur.0 - last.0).abs() > 0.01 || (cur.1 - last.1).abs() > 0.01 {
                                    crate::edit::pen_stroke(&mut img.edited, last, cur);
                                    img.bump();
                                    self.edit_drag = Some(EditDrag::Stroke { last: cur });
                                }
                            }
                            if response.drag_stopped_by(egui::PointerButton::Primary) {
                                self.edit_drag = None;
                            }
                        }
                        Tool::Eraser => {
                            if response.drag_started_by(egui::PointerButton::Primary)
                                && let Some(sp) = response.interact_pointer_pos()
                            {
                                self.edit.snapshot(&img.edited);
                                let p = s2p(sp);
                                crate::edit::eraser_stamp(&mut img.edited, p);
                                img.bump();
                                self.edit_drag = Some(EditDrag::Stroke { last: p });
                            }
                            if self.edit_drag.is_some()
                                && response.dragged_by(egui::PointerButton::Primary)
                                && let Some(sp) = response.interact_pointer_pos()
                            {
                                let cur = s2p(sp);
                                crate::edit::eraser_stamp(&mut img.edited, cur);
                                img.bump();
                            }
                            if response.drag_stopped_by(egui::PointerButton::Primary) {
                                self.edit_drag = None;
                            }
                        }
                        Tool::RectErase => {
                            if response.drag_started_by(egui::PointerButton::Primary)
                                && let Some(sp) = response.interact_pointer_pos()
                            {
                                self.edit_drag = Some(EditDrag::Rect { start: s2p(sp) });
                            }
                            if response.drag_stopped_by(egui::PointerButton::Primary) {
                                if let Some(EditDrag::Rect { start }) = self.edit_drag
                                    && let Some(sp) = response.interact_pointer_pos()
                                {
                                    self.edit.snapshot(&img.edited);
                                    crate::edit::erase_rect(&mut img.edited, start, s2p(sp));
                                    img.bump();
                                    self.set_status("Rectangle erased");
                                }
                                self.edit_drag = None;
                            }
                        }
                        Tool::PolygonErase => {
                            if response.clicked()
                                && let Some(sp) = response.interact_pointer_pos()
                            {
                                self.edit.polygon.push(s2p(sp));
                            }
                        }
                    }
                }
            }
            Step::Filter => {
                response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
                if response.clicked()
                    && let Some(sp) = response.interact_pointer_pos()
                {
                    let p = s2p(sp);
                    let x = (p.0.floor() as i32).clamp(0, dims.0 as i32 - 1) as u32;
                    let y = (p.1.floor() as i32).clamp(0, dims.1 as i32 - 1) as u32;
                    match self.filter_tool {
                        FilterTool::Pick => {
                            if let Some(img) = self.image.as_ref() {
                                let c = img.original.get_pixel(x, y).0;
                                self.filter.picked = Some([c[0], c[1], c[2]]);
                                self.recompute_preview();
                                self.status = format!(
                                    "Picked rgb({},{},{}) — set tolerance and Apply",
                                    c[0], c[1], c[2]
                                );
                            }
                        }
                        FilterTool::Mask => {
                            self.mask_draft.push(p);
                            self.status = format!(
                                "Mask vertex {} — Close mask when done",
                                self.mask_draft.len()
                            );
                        }
                    }
                }
            }
            Step::Data => match self.data_mode {
                DataMode::Manual | DataMode::Adjust => {
                    response = response.on_hover_cursor(egui::CursorIcon::Crosshair);
                    if response.clicked()
                        && let Some(sp) = response.interact_pointer_pos()
                    {
                        let px = s2p(sp);
                        match self.data_mode {
                            DataMode::Manual => self.manual_click(px),
                            DataMode::Adjust => self.adjust_click(px),
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }

        let mut pan = self.view.pan;
        let drag_delta = if response.dragged_by(egui::PointerButton::Middle)
            || (!pan_blocked && response.dragged_by(egui::PointerButton::Primary))
        {
            response.drag_delta()
        } else {
            Vec2::ZERO
        };
        pan += drag_delta;
        self.view.pan = pan;
        self.clamp_view(rect, img_w, img_h);
        topleft = rect.min + self.view.pan;
        let zoom = self.view.zoom;

        let p2s_final =
            |p: Px| -> Pos2 { Pos2::new(topleft.x + p.0 * zoom, topleft.y + p.1 * zoom) };
        let s2p_final = |sp: Pos2| -> Px { ((sp.x - topleft.x) / zoom, (sp.y - topleft.y) / zoom) };

        self.update_texture(ui.ctx());
        let tex_id = self.texture.as_ref().map(|t| t.id());
        let img_rect = Rect::from_min_size(topleft, Vec2::new(img_w, img_h) * zoom);
        painter.rect_filled(img_rect.expand(2.0), 0.0, Color32::from_rgb(60, 60, 64));
        if let Some(tex_id) = tex_id {
            painter.image(
                tex_id,
                img_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        painter.rect_stroke(
            img_rect,
            0.0,
            Stroke::new(1.0, Color32::from_rgb(90, 90, 96)),
            StrokeKind::Outside,
        );

        if response.hovered() {
            if let Some(sp) = response.hover_pos() {
                painter.line_segment(
                    [Pos2::new(sp.x, rect.top()), Pos2::new(sp.x, rect.bottom())],
                    Stroke::new(0.5, Color32::from_rgb(120, 120, 130)),
                );
                painter.line_segment(
                    [Pos2::new(rect.left(), sp.y), Pos2::new(rect.right(), sp.y)],
                    Stroke::new(0.5, Color32::from_rgb(120, 120, 130)),
                );
                self.hover_px = Some(s2p_final(sp));
            }
        } else {
            self.hover_px = None;
        }

        match self.step {
            Step::Axes => {
                if let Some(r) = self.axis_rect {
                    let a = p2s_final((r.left, r.top));
                    let b = p2s_final((r.right, r.bottom));
                    let color = Color32::from_rgb(0, 200, 255);
                    painter.line_segment(
                        [Pos2::new(a.x, a.y), Pos2::new(b.x, a.y)],
                        Stroke::new(2.0, color),
                    );
                    painter.line_segment(
                        [Pos2::new(a.x, b.y), Pos2::new(b.x, b.y)],
                        Stroke::new(2.0, color),
                    );
                    painter.line_segment(
                        [Pos2::new(a.x, a.y), Pos2::new(a.x, b.y)],
                        Stroke::new(2.0, color),
                    );
                    painter.line_segment(
                        [Pos2::new(b.x, a.y), Pos2::new(b.x, b.y)],
                        Stroke::new(2.0, color),
                    );
                    let mids: [Px; 4] = [
                        ((r.left + r.right) * 0.5, r.top),
                        ((r.left + r.right) * 0.5, r.bottom),
                        (r.left, (r.top + r.bottom) * 0.5),
                        (r.right, (r.top + r.bottom) * 0.5),
                    ];
                    for mid in mids {
                        let sp = p2s_final(mid);
                        painter.circle_filled(sp, 5.0, color);
                        painter.circle_filled(sp, 2.0, Color32::WHITE);
                    }
                }
            }
            Step::Filter => {
                if let Some(m) = self.filter.mask.as_deref()
                    && m.len() >= 2
                {
                    let pts: Vec<Pos2> = m.iter().map(|p| p2s_final(*p)).collect();
                    for (i, sp) in pts.iter().enumerate() {
                        if i > 0 {
                            painter.line_segment(
                                [pts[i - 1], *sp],
                                Stroke::new(1.5, Color32::from_rgb(220, 90, 255)),
                            );
                        }
                    }
                    painter.line_segment(
                        [pts[pts.len() - 1], pts[0]],
                        Stroke::new(1.0, Color32::from_rgb(220, 90, 255)),
                    );
                    for sp in &pts {
                        painter.circle_filled(*sp, 3.0, Color32::from_rgb(220, 90, 255));
                    }
                }
                if !self.mask_draft.is_empty() {
                    let pts: Vec<Pos2> = self.mask_draft.iter().map(|p| p2s_final(*p)).collect();
                    for (i, sp) in pts.iter().enumerate() {
                        painter.circle_filled(*sp, 4.0, Color32::from_rgb(255, 200, 0));
                        if i > 0 {
                            painter.line_segment(
                                [pts[i - 1], *sp],
                                Stroke::new(1.5, Color32::from_rgb(255, 200, 0)),
                            );
                        }
                    }
                    if let Some(hover) = response.hover_pos()
                        && let Some(last) = pts.last()
                    {
                        painter.line_segment(
                            [*last, hover],
                            Stroke::new(1.0, Color32::from_rgb(255, 200, 120)),
                        );
                    }
                }
            }
            Step::Edit => {
                if !self.edit.polygon.is_empty() {
                    let pts: Vec<Pos2> = self.edit.polygon.iter().map(|p| p2s_final(*p)).collect();
                    for (i, sp) in pts.iter().enumerate() {
                        painter.circle_filled(*sp, 4.0, Color32::from_rgb(255, 200, 0));
                        if i > 0 {
                            painter.line_segment(
                                [pts[i - 1], *sp],
                                Stroke::new(1.5, Color32::from_rgb(255, 200, 0)),
                            );
                        }
                    }
                    if let Some(hover) = response.hover_pos()
                        && let Some(last) = pts.last()
                    {
                        painter.line_segment(
                            [*last, hover],
                            Stroke::new(1.0, Color32::from_rgb(255, 200, 120)),
                        );
                    }
                }
                if let Some(EditDrag::Rect { start }) = self.edit_drag
                    && let Some(sp) = response.interact_pointer_pos()
                {
                    let a = p2s_final(start);
                    painter.rect_stroke(
                        Rect::from_two_pos(a, sp),
                        0.0,
                        Stroke::new(1.5, Color32::from_rgb(255, 120, 120)),
                        StrokeKind::Middle,
                    );
                }
            }
            Step::Data => {
                for (i, ds) in self.datasets.iter().enumerate() {
                    let color = Color32::from_rgb(ds.color[0], ds.color[1], ds.color[2]);
                    let selected = i == self.selected_dataset;
                    for p in ds.points.iter().take(50000) {
                        let sp = p2s_final(*p);
                        if !rect.contains(sp) {
                            continue;
                        }
                        if selected {
                            painter.circle_filled(sp, 2.5, color);
                        } else {
                            painter.circle(sp, 2.0, Color32::TRANSPARENT, Stroke::new(1.0, color));
                        }
                    }
                }
            }
            _ => {}
        }

        if self.image.is_none() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "No image loaded — use the Load step",
                egui::FontId::proportional(18.0),
                Color32::from_rgb(140, 140, 150),
            );
        }
    }

    fn show_rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        for step in ALL_STEPS {
            let unlocked = self.step_unlocked_reason(step);
            let mut btn = egui::Button::new(format!("{}. {}", step.index() + 1, step.name()))
                .min_size(Vec2::new(ui.available_width() - 12.0, 30.0));
            if self.step == step {
                btn = btn.selected(true);
            }
            let mut resp = ui.add_enabled(unlocked.is_ok(), btn);
            if let Err(reason) = unlocked {
                resp = resp.on_disabled_hover_text(reason);
            }
            if resp.clicked() {
                self.go_to_step(step);
            }
            ui.add_space(2.0);
        }
        ui.add_space(12.0);
        ui.separator();
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            ui.small(self.step.hint());
        });
    }

    fn show_controls(&mut self, ui: &mut egui::Ui) {
        match self.step {
            Step::Load => self.show_load_panel(ui),
            Step::Axes => self.show_axes_panel(ui),
            Step::Range => self.show_range_panel(ui),
            Step::Filter => self.show_filter_panel(ui),
            Step::Edit => self.show_edit_panel(ui),
            Step::Data => self.show_data_panel(ui),
            Step::Export => self.show_export_panel(ui),
        }
    }

    fn show_load_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Load image");
        ui.add_space(4.0);
        if ui.button("Open File…").clicked() {
            let path = rfd::FileDialog::new()
                .add_filter("Images", loader::IMAGE_EXTENSIONS)
                .pick_file();
            if let Some(path) = path {
                match loader::load_from_path(&path) {
                    Ok(img) => self.set_image(img, "Opened image file"),
                    Err(e) => self.set_error(format!("Open failed: {e}")),
                }
            }
        }
        if ui.button("Paste from Clipboard").clicked() {
            match loader::load_from_clipboard() {
                Ok(img) => self.set_image(img, "Pasted from clipboard"),
                Err(e) => self.set_error(format!("Clipboard failed: {e}")),
            }
        }
        ui.add_space(8.0);
        ui.label("Image URL:");
        ui.add(
            egui::TextEdit::singleline(&mut self.url_input)
                .hint_text("https://…")
                .desired_width(ui.available_width()),
        );
        if ui
            .add_enabled(!self.url_pending, egui::Button::new("Download"))
            .clicked()
            && !self.url_input.trim().is_empty()
        {
            let url = self.url_input.trim().to_string();
            let (tx, rx) = channel();
            loader::spawn_url_download(url, tx);
            self.url_rx = Some(rx);
            self.url_pending = true;
            self.set_status("Downloading…");
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if self.url_pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Downloading…");
            });
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label("You can also drag && drop an image file onto the window.");
        if let Some(img) = self.image.as_ref() {
            ui.add_space(8.0);
            ui.label(format!(
                "Loaded: {} × {} px",
                img.original.width(),
                img.original.height()
            ));
        }
    }

    fn show_axes_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Axes");
        ui.add_space(4.0);
        if ui.button("Auto-detect").clicked()
            && let Some(img) = self.image.as_ref()
        {
            match axis::detect_axis_rect(&img.original) {
                Some(r) => {
                    self.axis_rect = Some(r);
                    self.set_status(format!(
                        "Detected axes: L {:.1} T {:.1} R {:.1} B {:.1}",
                        r.left, r.top, r.right, r.bottom
                    ));
                }
                None => {
                    self.set_error("No axis rectangle found — set the edges manually");
                }
            }
        }
        if self.axis_rect.is_none()
            && ui.button("Manual rectangle (full image)").clicked()
            && let Some(img) = self.image.as_ref()
        {
            let (w, h) = img.dimensions();
            let dx = w as f32 * 0.05;
            let dy = h as f32 * 0.05;
            self.axis_rect = Some(AxisRect {
                left: dx,
                top: dy,
                right: w as f32 - dx,
                bottom: h as f32 - dy,
            });
            self.set_status("Drag the four edge handles inward");
        }
        ui.add_space(8.0);
        if let Some(r) = self.axis_rect {
            let (w, h) = self
                .image
                .as_ref()
                .map(|i| i.dimensions())
                .unwrap_or((0, 0));
            if r.is_valid(w, h) {
                ui.colored_label(Color32::from_rgb(80, 200, 120), "Axis rectangle is valid");
            } else {
                ui.colored_label(
                    Color32::RED,
                    "Invalid rectangle (crossing edges or outside image)",
                );
            }
            ui.monospace(format!(
                "left {:.1}  top {:.1}\nright {:.1}  bottom {:.1}\nwidth {:.1}  height {:.1}",
                r.left,
                r.top,
                r.right,
                r.bottom,
                r.width(),
                r.height()
            ));
        } else {
            ui.label("No axis rectangle yet. Auto-detect or start a manual one.");
        }
        ui.add_space(8.0);
        ui.label("Handles: circles on each edge. Drag to move.");
    }

    fn show_range_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Range");
        ui.add_space(4.0);
        let mut c = self.calib;
        ui.label("x axis (left edge → right edge)");
        ui.horizontal(|ui| {
            ui.label("min");
            ui.add(egui::DragValue::new(&mut c.x.min).speed(1.0));
            ui.label("max");
            ui.add(egui::DragValue::new(&mut c.x.max).speed(1.0));
        });
        ui.horizontal(|ui| {
            if ui
                .selectable_label(matches!(c.x.scale, Scale::Linear), "Linear")
                .clicked()
            {
                c.x.scale = Scale::Linear;
            }
            if ui
                .selectable_label(matches!(c.x.scale, Scale::Log { .. }), "Log")
                .clicked()
            {
                c.x.scale = Scale::Log { base: 10.0 };
            }
            if let Scale::Log { base } = &mut c.x.scale {
                ui.add(
                    egui::DragValue::new(base)
                        .speed(0.1)
                        .range(0.01..=10000.0)
                        .prefix("b="),
                );
            }
        });
        ui.add_space(8.0);
        ui.label("y axis (bottom edge → top edge)");
        ui.horizontal(|ui| {
            ui.label("min");
            ui.add(egui::DragValue::new(&mut c.y.min).speed(1.0));
            ui.label("max");
            ui.add(egui::DragValue::new(&mut c.y.max).speed(1.0));
        });
        ui.horizontal(|ui| {
            if ui
                .selectable_label(matches!(c.y.scale, Scale::Linear), "Linear")
                .clicked()
            {
                c.y.scale = Scale::Linear;
            }
            if ui
                .selectable_label(matches!(c.y.scale, Scale::Log { .. }), "Log")
                .clicked()
            {
                c.y.scale = Scale::Log { base: 10.0 };
            }
            if let Scale::Log { base } = &mut c.y.scale {
                ui.add(
                    egui::DragValue::new(base)
                        .speed(0.1)
                        .range(0.01..=10000.0)
                        .prefix("b="),
                );
            }
        });
        self.calib = c;
        ui.add_space(8.0);
        match self.calib.validate() {
            Ok(()) => {
                ui.colored_label(Color32::from_rgb(80, 200, 120), "Calibration is valid");
            }
            Err(e) => {
                ui.colored_label(Color32::RED, e);
            }
        }
        ui.add_space(8.0);
        ui.small("y-min sits on the bottom edge, y-max on the top edge (image y grows downward).");
    }

    fn show_filter_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Filter");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.filter_tool == FilterTool::Pick, "Pick color")
                .clicked()
            {
                self.filter_tool = FilterTool::Pick;
            }
            if ui
                .selectable_label(self.filter_tool == FilterTool::Mask, "Mask")
                .clicked()
            {
                self.filter_tool = FilterTool::Mask;
            }
        });
        ui.small(match self.filter_tool {
            FilterTool::Pick => "Click a pixel on the image to pick the line color.",
            FilterTool::Mask => {
                "Click vertices around the curve area; Close mask limits filtering to it."
            }
        });
        let mut picked = self.filter.picked;
        ui.horizontal(|ui| {
            ui.label("Color:");
            match &mut picked {
                Some(rgb) => {
                    let mut c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        *rgb = [c.r(), c.g(), c.b()];
                        self.filter.picked = Some(*rgb);
                        self.recompute_preview();
                    }
                }
                None => {
                    ui.label("none picked yet");
                }
            }
        });
        let mut tol = self.filter.tolerance as f32;
        let resp = ui
            .add(egui::Slider::new(&mut tol, 0.0..=filter::MAX_TOLERANCE as f32).text("tolerance"));
        if resp.changed() {
            self.filter.tolerance = tol as u8;
            self.recompute_preview();
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can_apply = self.filter.picked.is_some();
            if ui
                .add_enabled(can_apply, egui::Button::new("Apply"))
                .clicked()
            {
                self.commit_live_filter();
                self.set_status("Filter applied (cropped to axes)");
            }
            if ui.button("Reset").clicked() {
                self.filter = FilterParams::default();
                self.filter_preview = None;
                if let Some(img) = self.image.as_mut() {
                    img.reset_filter();
                    self.edit.reset();
                }
                self.set_status("Filter reset");
            }
        });
        ui.add_space(8.0);
        ui.label("Mask");
        if let Some(m) = self.filter.mask.as_ref() {
            ui.colored_label(
                Color32::from_rgb(220, 90, 255),
                format!("Mask active — {} vertices; only that area is kept", m.len()),
            );
        } else {
            ui.small("No mask — filtering applies to the whole axis rectangle.");
        }
        if !self.mask_draft.is_empty() {
            ui.label(format!("draft vertices: {}", self.mask_draft.len()));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.mask_draft.len() >= 3, egui::Button::new("Close mask"))
                .clicked()
            {
                self.close_mask();
            }
            if ui
                .add_enabled(
                    self.filter.mask.is_some() || !self.mask_draft.is_empty(),
                    egui::Button::new("Clear mask"),
                )
                .clicked()
            {
                self.clear_mask();
            }
        });
        ui.small("Enter closes the mask, Escape cancels the draft.");
        if self.filter_preview.is_some() {
            ui.colored_label(
                Color32::from_rgb(200, 200, 90),
                "Preview is live — Apply to commit (leaving this step commits it)",
            );
        }
    }

    fn show_edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Edit");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for tool in [Tool::Pen, Tool::Eraser, Tool::RectErase, Tool::PolygonErase] {
                let name = match tool {
                    Tool::Pen => "Pen",
                    Tool::Eraser => "Eraser",
                    Tool::RectErase => "Rect",
                    Tool::PolygonErase => "Polygon",
                };
                if ui.selectable_label(self.edit.tool == tool, name).clicked() {
                    self.edit.tool = tool;
                }
            }
        });
        ui.small(match self.edit.tool {
            Tool::Pen => "Drag to draw black ink.",
            Tool::Eraser => "Drag to erase pixels to white.",
            Tool::RectErase => "Drag a rectangle to erase it.",
            Tool::PolygonErase => "Click vertices; Enter or the button closes and erases.",
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.edit.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.do_edit_undo();
            }
            if ui
                .add_enabled(self.edit.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                self.do_edit_redo();
            }
        });
        ui.small("Ctrl+Z / Ctrl+Y");
        if self.edit.tool == Tool::PolygonErase && !self.edit.polygon.is_empty() {
            ui.add_space(4.0);
            ui.label(format!("vertices: {}", self.edit.polygon.len()));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.edit.can_close_polygon(),
                        egui::Button::new("Close && Erase"),
                    )
                    .clicked()
                {
                    self.close_polygon();
                }
                if ui.button("Cancel").clicked() {
                    self.edit.cancel_polygon();
                }
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Clear Border").clicked() {
                self.clear_border_op();
            }
            if ui.button("Remove Gridlines").clicked() {
                self.remove_gridlines_op();
            }
        });
        ui.small("Clear Border removes ink connected to the image border.");
        ui.small(
            "Remove Gridlines erases perfectly straight vertical/horizontal lines spanning the axis area.",
        );
    }

    fn show_data_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Data");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for mode in [
                DataMode::Continuous,
                DataMode::Discrete,
                DataMode::Manual,
                DataMode::Adjust,
            ] {
                if ui
                    .selectable_label(self.data_mode == mode, mode.name())
                    .clicked()
                {
                    self.data_mode = mode;
                }
            }
        });
        ui.add_space(6.0);
        match self.data_mode {
            DataMode::Continuous => {
                ui.small("All non-white pixels of the current image become points of the selected dataset.");
                self.show_one_to_one_controls(ui);
                ui.horizontal(|ui| {
                    let mut it = self.interpolate;
                    ui.checkbox(&mut it, "Interpolate");
                    self.interpolate = it;
                    if self.interpolate {
                        egui::ComboBox::from_id_salt("interp_mode")
                            .selected_text(self.interp_mode.name())
                            .show_ui(ui, |ui| {
                                for m in [
                                    InterpMode::MeanSquared,
                                    InterpMode::Cubic,
                                    InterpMode::Spline,
                                ] {
                                    ui.selectable_value(&mut self.interp_mode, m, m.name());
                                }
                            });
                    }
                });
                ui.small("Interpolate rebuilds the curve as a true function: Mean squared = least-squares line, Cubic = piecewise Hermite, Spline = natural cubic spline.");
                if ui
                    .add_enabled(self.image.is_some(), egui::Button::new("Capture"))
                    .clicked()
                {
                    self.capture_selected(DataMode::Continuous);
                }
            }
            DataMode::Discrete => {
                ui.small("Blob centroids of the current image (min area rejects specks).");
                let mut area = self.min_blob_area as f32;
                let resp = ui.add(egui::Slider::new(&mut area, 1.0..=100.0).text("min blob area"));
                if resp.changed() {
                    self.min_blob_area = area as u32;
                }
                self.show_one_to_one_controls(ui);
                if ui
                    .add_enabled(self.image.is_some(), egui::Button::new("Capture"))
                    .clicked()
                {
                    self.capture_selected(DataMode::Discrete);
                }
            }
            DataMode::Manual => {
                ui.small("Click the image to append points to the selected dataset; points are interpolated along segments.");
                let mut s = self.manual_step;
                let resp = ui.add(egui::Slider::new(&mut s, 1.0..=50.0).text("interp step (px)"));
                if resp.changed() {
                    self.manual_step = s;
                }
                if ui.button("Undo Last").clicked() {
                    self.undo_last_click();
                }
                ui.small("Ctrl+Z also removes the last click.");
            }
            DataMode::Adjust => {
                ui.small("Click near a point to delete it; click elsewhere to insert one.");
                if ui.button("Undo").clicked() {
                    self.undo_dataset_points();
                }
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        let count = self.datasets.len();
        if ui.button("+ Add dataset").clicked() {
            let id = self.next_dataset_id;
            self.next_dataset_id += 1;
            let color = DATASET_COLORS[count % DATASET_COLORS.len()];
            self.datasets
                .push(Dataset::new(id, format!("dataset {id}"), color));
            self.selected_dataset = self.datasets.len() - 1;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for i in 0..self.datasets.len() {
                let color = self.datasets[i].color;
                let name = self.datasets[i].name.clone();
                let npoints = self.datasets[i].points.len();
                ui.horizontal(|ui| {
                    let mut c = Color32::from_rgb(color[0], color[1], color[2]);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        self.datasets[i].color = [c.r(), c.g(), c.b()];
                    }
                    if ui
                        .selectable_label(self.selected_dataset == i, format!("{name} ({npoints})"))
                        .clicked()
                    {
                        self.selected_dataset = i;
                    }
                    if ui.small_button("✕").clicked() {
                        self.datasets.remove(i);
                        if self.selected_dataset >= self.datasets.len() {
                            self.selected_dataset = self.datasets.len().saturating_sub(1);
                        }
                    }
                });
                if self.selected_dataset == i && !self.datasets.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label("name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.datasets[i].name)
                                .desired_width(160.0),
                        );
                    });
                }
                ui.add_space(2.0);
            }
        });
    }

    fn show_export_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Export");
        ui.add_space(4.0);
        let mut fmt = self.export_format;
        let mut combined = self.export_combined;
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.export_format == Format::Csv, "CSV")
                .clicked()
            {
                fmt = Format::Csv;
            }
            if ui
                .selectable_label(self.export_format == Format::Txt, "TXT (tab-separated)")
                .clicked()
            {
                fmt = Format::Txt;
            }
        });
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.export_combined, "Single combined file")
                .clicked()
            {
                combined = true;
            }
            if ui
                .selectable_label(!self.export_combined, "One file per dataset")
                .clicked()
            {
                combined = false;
            }
        });
        self.export_format = fmt;
        self.export_combined = combined;
        ui.add_space(8.0);
        if ui.button("Save…").clicked() {
            self.do_export();
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label("Preview (first rows):");
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                if let Some((_, content)) = self.export_outputs().as_deref().and_then(|o| o.first())
                {
                    let preview: String = content.lines().take(30).collect::<Vec<_>>().join("\n");
                    ui.add(
                        egui::TextEdit::multiline(&mut preview.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(ui.available_width())
                            .interactive(false),
                    );
                }
            });
        match self.calib.validate() {
            Ok(()) => {}
            Err(e) => {
                ui.colored_label(Color32::RED, e);
            }
        }
    }

    fn show_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.small(self.step.hint());
            ui.separator();
            let readout = self.readout_text();
            ui.monospace(readout);
            if let Some(e) = self.error.clone() {
                ui.separator();
                ui.colored_label(Color32::RED, e);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Fit").clicked() {
                    self.view.needs_fit = true;
                }
                ui.label(format!("{:.0}%", self.view.zoom * 100.0));
            });
        });
    }

    fn readout_text(&self) -> String {
        match (self.hover_px, self.image.as_ref()) {
            (Some((x, y)), Some(_)) => {
                let s = format!("px {x:.0},{y:.0}");
                if let Some(r) = self.axis_rect {
                    let (dx, dy) = self.calib.pixel_to_data(&r, (x, y));
                    format!("{s}  →  data {dx:.4}, {dy:.4}")
                } else {
                    s
                }
            }
            _ => "px —".to_string(),
        }
    }

    fn pump_url(&mut self, ctx: &egui::Context) {
        if !self.url_pending {
            return;
        }
        let rx = self.url_rx.take();
        match rx {
            Some(rx) => match rx.try_recv() {
                Ok(Ok(img)) => {
                    self.set_image(img, "Downloaded image");
                    self.url_pending = false;
                }
                Ok(Err(msg)) => {
                    self.set_error(format!("Download failed: {msg}"));
                    self.url_pending = false;
                }
                Err(TryRecvError::Empty) => {
                    self.url_rx = Some(rx);
                    ctx.request_repaint_after(Duration::from_millis(120));
                }
                Err(TryRecvError::Disconnected) => {
                    self.set_error("Download failed: connection dropped");
                    self.url_pending = false;
                }
            },
            None => {
                self.url_pending = false;
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let paths: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = paths.first() {
            match loader::dropped_file_path(path) {
                Ok(img) => self.set_image(img, "Loaded dropped file"),
                Err(e) => self.set_error(format!("Drop failed: {e}")),
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let (esc, enter, undo, redo) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Enter),
                i.modifiers.ctrl && i.key_pressed(egui::Key::Z) && !i.modifiers.shift,
                i.modifiers.ctrl
                    && (i.key_pressed(egui::Key::Y)
                        || (i.key_pressed(egui::Key::Z) && i.modifiers.shift)),
            )
        });
        if esc {
            if self.step == Step::Filter && !self.mask_draft.is_empty() {
                self.mask_draft.clear();
                self.status = "Mask draft cancelled".to_string();
            }
            if !self.edit.polygon.is_empty() {
                self.edit.cancel_polygon();
                self.status = "Polygon cancelled".to_string();
            }
            self.edit_drag = None;
        }
        if enter
            && self.step == Step::Edit
            && self.edit.tool == Tool::PolygonErase
            && self.edit.can_close_polygon()
        {
            self.close_polygon();
        }
        if enter
            && self.step == Step::Filter
            && self.filter_tool == FilterTool::Mask
            && self.mask_draft.len() >= 3
        {
            self.close_mask();
        }
        if undo {
            match self.step {
                Step::Edit => self.do_edit_undo(),
                Step::Data => match self.data_mode {
                    DataMode::Manual => self.undo_last_click(),
                    _ => self.undo_dataset_points(),
                },
                _ => {}
            }
        }
        if redo && self.step == Step::Edit {
            self.do_edit_redo();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_url(ui.ctx());
        self.handle_dropped_files(ui.ctx());
        self.handle_shortcuts(ui.ctx());

        egui::Panel::left("rail")
            .exact_size(150.0)
            .show(ui, |ui| self.show_rail(ui));
        egui::Panel::right("controls")
            .exact_size(330.0)
            .show(ui, |ui| self.show_controls(ui));
        egui::Panel::bottom("status")
            .exact_size(34.0)
            .show(ui, |ui| self.show_status(ui));
        egui::CentralPanel::default().show(ui, |ui| self.show_canvas(ui));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Dataset;
    use crate::image_state::WHITE;
    use image::Rgba;

    fn setup_app() -> App {
        let mut app = App::new();
        let mut img = RgbaImage::from_pixel(50, 40, WHITE);
        img.put_pixel(5, 5, Rgba([255, 0, 0, 255]));
        img.put_pixel(30, 20, Rgba([0, 200, 0, 255]));
        img.put_pixel(31, 20, Rgba([0, 200, 0, 255]));
        app.image = Some(ImageState::new(img));
        app.axis_rect = Some(AxisRect {
            left: 10.0,
            top: 10.0,
            right: 45.0,
            bottom: 35.0,
        });
        app.filter.picked = Some([0, 200, 0]);
        app.datasets.push(Dataset::new(1, "d", [0, 0, 0]));
        app
    }

    #[test]
    fn leaving_filter_commits_preview_so_capture_matches_viewfinder() {
        let mut app = setup_app();
        app.recompute_preview();
        assert!(app.filter_preview.is_some());
        app.step = Step::Filter;
        app.go_to_step(Step::Data);
        assert!(app.filter_preview.is_none());
        app.capture_selected(DataMode::Continuous);
        let pts = &app.datasets[0].points;
        assert_eq!(pts.len(), 2, "only the green curve pixels, got {pts:?}");
        assert!(
            pts.iter()
                .all(|&(x, y)| (29.0..=32.0).contains(&x) && (18.0..=22.0).contains(&y))
        );
    }

    #[test]
    fn capture_commits_live_preview_first() {
        let mut app = setup_app();
        app.recompute_preview();
        app.capture_selected(DataMode::Continuous);
        assert!(app.filter_preview.is_none());
        let pts = &app.datasets[0].points;
        assert_eq!(pts.len(), 2, "only the green curve pixels, got {pts:?}");
    }

    #[test]
    fn capture_without_filter_uses_edited_image() {
        let mut app = setup_app();
        app.capture_selected(DataMode::Continuous);
        let pts = &app.datasets[0].points;
        assert_eq!(pts.len(), 3, "unfiltered edited keeps all ink, got {pts:?}");
    }

    #[test]
    fn clamp_view_skips_clamp_on_tiny_viewport() {
        let mut app = App::new();
        app.image = Some(ImageState::new(RgbaImage::from_pixel(800, 44, WHITE)));
        app.view.zoom = 1.0;
        app.view.pan = Vec2::new(16.0, 16.0);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 68.0));
        app.clamp_view(rect, 800.0, 44.0);
        assert_eq!(app.view.pan, Vec2::new(16.0, 16.0));
    }

    #[test]
    fn clamp_view_constrains_pan_on_normal_viewport() {
        let mut app = App::new();
        app.image = Some(ImageState::new(RgbaImage::from_pixel(100, 100, WHITE)));
        app.view.zoom = 1.0;
        app.view.pan = Vec2::new(1000.0, 1000.0);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        app.clamp_view(rect, 100.0, 100.0);
        assert_eq!(app.view.pan, Vec2::new(740.0, 540.0));
    }

    #[test]
    fn continuous_capture_one_to_one_and_interpolation() {
        let mut app = App::new();
        let mut img = RgbaImage::from_pixel(50, 40, WHITE);
        for x in 12i32..=22 {
            let y0 = 2 * x - 12;
            img.put_pixel(x as u32, y0 as u32, Rgba([0, 200, 0, 255]));
            img.put_pixel(x as u32, (y0 + 1) as u32, Rgba([0, 200, 0, 255]));
        }
        app.image = Some(ImageState::new(img));
        app.axis_rect = Some(AxisRect {
            left: 10.0,
            top: 10.0,
            right: 45.0,
            bottom: 35.0,
        });
        app.filter.picked = Some([0, 200, 0]);
        app.datasets.push(Dataset::new(1, "d", [0, 0, 0]));
        app.capture_selected(DataMode::Continuous);
        let pts = &app.datasets[0].points;
        assert_eq!(pts.len(), 11, "one y per x, got {pts:?}");
        assert!(
            pts.iter()
                .all(|&(x, y)| (y - (2.0 * x - 11.5)).abs() <= 0.01)
        );
        assert!(pts.windows(2).all(|w| w[0].0 < w[1].0));
        app.interpolate = true;
        app.interp_mode = InterpMode::MeanSquared;
        app.capture_selected(DataMode::Continuous);
        let pts = &app.datasets[0].points;
        assert_eq!(pts.len(), 11, "interpolated stays dense, got {pts:?}");
        assert!(
            pts.iter()
                .all(|&(x, y)| (y - (2.0 * x - 11.5)).abs() <= 0.05)
        );
    }
}
