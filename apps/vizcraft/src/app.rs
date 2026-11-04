//! The application: project state, refresh, undo/redo, file handling and the frame layout
//! (title bar, ribbon, view rail, panes, editors).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

use egui::{Color32, Ui};
use vizcraft_core::build::{self, Built};
use vizcraft_core::doc::*;
use vizcraft_core::scalar::Scalar;
use vizcraft_core::state::Runtime;
use vizcraft_core::vquery::VisualData;

use crate::theme::{self, Tokens};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum View {
    Report,
    Table,
    Model,
    Dax,
    Tmdl,
    Sheets,
    Flows,
    Story,
    Sql,
    /// Data Formulator-style AI chart building (data threads).
    Formulator,
    /// AI hub: providers, local models, OpenJEV, personal brain, privacy.
    Ai,
}

impl View {
    pub const ALL: [(View, &'static str, &'static str); 11] = [
        (View::Report, "report", "Report view"),
        (View::Table, "table-view", "Table view"),
        (View::Model, "model", "Model view"),
        (View::Dax, "dax", "DAX query view"),
        (View::Tmdl, "tmdl", "TMDL view"),
        (View::Sheets, "sheet", "Data sheets (Excel-like editing)"),
        (View::Flows, "flow", "Flows (Fabric / KNIME-style pipelines, notebooks, ML)"),
        (View::Story, "story", "Stories (Tableau-style story points)"),
        (View::Sql, "sql", "SQL (DuckDB analytics over the model, files and your warehouse; Python)"),
        (View::Formulator, "formulator", "Formulator (AI data threads, after Microsoft Data Formulator)"),
        (View::Ai, "ai", "AI hub (models, keys, local models, OpenJEV, personal brain)"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    File,
    Home,
    Insert,
    Modeling,
    View,
    Data,
    Format,
    Arrange,
    Publish,
    Help,
}

pub struct Toast {
    pub text: String,
    pub until: f64,
    pub error: bool,
}

/// Panes on the right side of the report view.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Panes {
    pub filters: bool,
    pub visualizations: bool,
    pub data: bool,
    pub bookmarks: bool,
    pub selection: bool,
    pub sync_slicers: bool,
    pub performance: bool,
    pub format_shape: bool,
}

impl Default for Panes {
    fn default() -> Self {
        Panes { filters: false, visualizations: true, data: true, bookmarks: false, selection: false, sync_slicers: false, performance: false, format_shape: true }
    }
}

pub struct VizApp {
    pub project: Project,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub built: Built,
    pub rt: Runtime,
    pub view: View,
    pub ribbon_tab: RibbonTab,
    /// The Power Query Editor window is open.
    pub pq_open: bool,
    /// The page shown in the report view.
    pub page: u64,
    /// Selected elements on the page.
    pub selected: Vec<u64>,
    undo: Vec<(String, String)>,
    redo: Vec<(String, String)>,
    pub toasts: Vec<Toast>,
    /// Re-run queries (Refresh) at the end of the frame.
    pub needs_refresh: bool,
    /// Rebuild the model from loaded data (model edits, sheet edits, parameters).
    pub needs_rebuild: bool,
    pub dark: bool,
    pub panes: Panes,
    /// Visual result cache (key → data, per model generation).
    pub cache: HashMap<String, Rc<VisualData>>,
    pub generation: u64,
    pub textures: HashMap<String, egui::TextureHandle>,
    /// Per-feature UI state.
    pub report: crate::report::ReportUi,
    pub pq: crate::pq::PqState,
    pub model_ui: crate::model::ModelUi,
    pub sheet_ui: crate::sheet::SheetUi,
    pub flows_ui: crate::flows::FlowsUi,
    pub publish_ui: crate::publish::PublishUi,
    pub sql_ui: crate::sqlview::SqlUi,
    pub ribbon: crate::ribbon::RibbonUi,
    /// AI: copilot, Formulator, AI hub, personal brain.
    pub ai: crate::ai::AiState,
    pub last_refresh_ms: f64,
    pub screenshot: Option<(String, u32)>,
    /// Frame at which `--screenshot` captures.
    pub screenshot_frame: u32,
    /// Synthetic input for automated UI checks (`VIZCRAFT_EVENTS="20:click:500,400;40:key:Delete"`).
    pub synthetic: Vec<(u32, String)>,
    pub frame: u32,
}

impl VizApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: &[String]) -> VizApp {
        let dark = cc.storage.and_then(|s| s.get_string("dark")).is_some_and(|v| v == "true");
        theme::apply(&cc.egui_ctx, dark);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut app = VizApp {
            project: Project::new(),
            path: None,
            dirty: false,
            built: Built::default(),
            rt: Runtime::default(),
            view: View::Report,
            ribbon_tab: RibbonTab::Home,
            pq_open: false,
            page: 0,
            selected: vec![],
            undo: vec![],
            redo: vec![],
            toasts: vec![],
            needs_refresh: true,
            needs_rebuild: false,
            dark,
            panes: Panes::default(),
            cache: HashMap::new(),
            generation: 0,
            textures: HashMap::new(),
            report: Default::default(),
            pq: Default::default(),
            model_ui: Default::default(),
            sheet_ui: Default::default(),
            flows_ui: Default::default(),
            publish_ui: Default::default(),
            sql_ui: Default::default(),
            ribbon: Default::default(),
            ai: Default::default(),
            last_refresh_ms: 0.0,
            screenshot: None,
            screenshot_frame: 15,
            synthetic: std::env::var("VIZCRAFT_EVENTS")
                .unwrap_or_default()
                .split(';')
                .filter_map(|e| {
                    let (f, rest) = e.split_once(':')?;
                    Some((f.trim().parse().ok()?, rest.to_string()))
                })
                .collect(),
            frame: 0,
        };
        let mut i = 0;
        while i < args.len() {
            match args.get(i).map(String::as_str) {
                Some("--sample") => app.load_project(vizcraft_core::sample::sales(), None),
                Some("--view") => {
                    i += 1;
                    let v = args.get(i).cloned().unwrap_or_default();
                    if let Some((view, _, _)) = View::ALL.iter().find(|(_, k, _)| *k == v || format!("{:?}", View::ALL.iter().find(|(_, kk, _)| *kk == v).map(|x| x.0)).contains(&v)) {
                        app.view = *view;
                    }
                    if v == "pq" {
                        app.pq_open = true;
                    }
                }
                Some("--page") => {
                    i += 1;
                    let n: usize = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
                    if let Some(p) = app.project.report.pages.get(n) {
                        app.page = p.id;
                    }
                }
                Some("--screenshot") => {
                    i += 1;
                    app.screenshot = args.get(i).map(|p| (p.clone(), 0));
                }
                Some("--screenshot-frame") => {
                    i += 1;
                    app.screenshot_frame = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(15);
                }
                Some("--dark") => {
                    app.dark = true;
                    theme::apply(&cc.egui_ctx, true);
                }
                Some(path) if !path.starts_with("--") => app.open_path(PathBuf::from(path)),
                _ => {}
            }
            i += 1;
        }
        if app.page == 0 {
            app.page = app.project.report.pages.first().map(|p| p.id).unwrap_or(0);
        }
        app
    }

    pub fn tokens(&self) -> Tokens {
        if self.dark { theme::DARK } else { theme::LIGHT }
    }

    // ---------------------------------------------------------------- project lifecycle

    pub fn load_project(&mut self, p: Project, path: Option<PathBuf>) {
        self.project = p;
        self.path = path;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.rt = Runtime::default();
        self.page = self.project.report.pages.first().map(|p| p.id).unwrap_or(0);
        self.needs_refresh = true;
        self.cache.clear();
    }

    pub fn open_path(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|s| Project::from_json(&s).map_err(|e| e.to_string())) {
            Ok(p) => {
                self.load_project(p, Some(path.clone()));
                self.toast(format!("Opened {}", path.display()));
            }
            Err(e) => self.error(format!("Couldn't open {}: {e}", path.display())),
        }
    }

    pub fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("VizCraft project", &["vizc", "json"]).pick_file() {
            self.open_path(path);
        }
    }

    pub fn save(&mut self, save_as: bool) {
        let path = match (&self.path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("VizCraft project", &["vizc"]).set_file_name(format!("{}.vizc", self.project.name)).save_file(),
        };
        let Some(path) = path else { return };
        match std::fs::write(&path, self.project.to_json()) {
            Ok(()) => {
                self.path = Some(path.clone());
                self.dirty = false;
                self.toast(format!("Saved {}", path.display()));
            }
            Err(e) => self.error(format!("Couldn't save: {e}")),
        }
    }

    pub fn new_project(&mut self) {
        self.load_project(Project::new(), None);
    }

    // ---------------------------------------------------------------- undo / redo

    /// Call before changing the project: records an undo step.
    pub fn checkpoint(&mut self, label: &str) {
        self.undo.push((label.to_string(), self.project.to_json()));
        if self.undo.len() > 60 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some((label, json)) = self.undo.pop()
            && let Ok(p) = Project::from_json(&json)
        {
            self.redo.push((label.clone(), self.project.to_json()));
            self.project = p;
            self.after_history();
            self.toast(format!("Undo: {label}"));
        }
    }

    pub fn redo(&mut self) {
        if let Some((label, json)) = self.redo.pop()
            && let Ok(p) = Project::from_json(&json)
        {
            self.undo.push((label.clone(), self.project.to_json()));
            self.project = p;
            self.after_history();
            self.toast(format!("Redo: {label}"));
        }
    }

    fn after_history(&mut self) {
        self.needs_rebuild = true;
        self.selected.retain(|id| self.project.element(*id).is_some());
        if self.project.page(self.page).is_none() {
            self.page = self.project.report.pages.first().map(|p| p.id).unwrap_or(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }

    // ---------------------------------------------------------------- data

    pub fn params(&self) -> BTreeMap<String, Scalar> {
        self.project.model.parameters.iter().map(|p| (p.name.clone(), p.current.clone())).collect()
    }

    /// Re-runs every query and rebuilds the model.
    pub fn refresh(&mut self) {
        let t0 = std::time::Instant::now();
        build::sync_model_tables(&mut self.project);
        let host = vizcraft_connectors::DesktopHost::default();
        let params = self.params();
        // `VizCraft.Sql(...)` in queries runs on the SQL view's embedded engine.
        let sql_host = crate::sqlview::SqlHost::new(&host, &mut self.sql_ui, &self.project, &self.built, self.path.as_deref());
        self.built = build::build(&self.project, &sql_host, &params);
        self.after_build();
        self.last_refresh_ms = t0.elapsed().as_secs_f64() * 1000.0;
    }

    /// Rebuilds the model from loaded data.
    pub fn rebuild(&mut self) {
        build::sync_model_tables(&mut self.project);
        // New tables (a new query or sheet) need loading.
        let missing = self.project.model.tables.iter().any(|t| matches!(t.source, TableSource::Query(_) | TableSource::Flow(_)) && !self.built.tables.contains_key(&t.name));
        if missing {
            self.refresh();
            return;
        }
        self.built = build::rebuild(&self.project, &self.built, &self.params());
        self.after_build();
    }

    fn after_build(&mut self) {
        self.generation += 1;
        self.cache.clear();
        self.report.on_rebuild();
        for (obj, e) in self.built.errors.clone().iter().take(3) {
            self.toasts.push(Toast { text: format!("{obj}: {e}"), until: now() + 6.0, error: true });
        }
    }

    pub fn mark_model_dirty(&mut self) {
        self.needs_rebuild = true;
    }
    pub fn mark_data_dirty(&mut self) {
        self.needs_refresh = true;
    }

    // ---------------------------------------------------------------- messages

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), until: now() + 3.0, error: false });
    }
    pub fn error(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), until: now() + 7.0, error: true });
    }

    pub fn current_page(&self) -> Option<&Page> {
        self.project.page(self.page)
    }
    pub fn current_page_mut(&mut self) -> Option<&mut Page> {
        let id = self.page;
        self.project.page_mut(id)
    }

    pub fn go_to_page(&mut self, id: u64) {
        if id != self.page {
            self.rt.back_stack.push(self.page);
            self.page = id;
            self.selected.clear();
            self.rt.selection = None;
        }
        self.view = View::Report;
    }

    /// A texture from base64 image data (cached by key).
    pub fn texture(&mut self, ctx: &egui::Context, key: &str, b64: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(key) {
            return Some(t.clone());
        }
        let bytes = crate::report::b64_decode(b64)?;
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let t = ctx.load_texture(key, ci, egui::TextureOptions::LINEAR);
        self.textures.insert(key.to_string(), t.clone());
        Some(t)
    }

    // ---------------------------------------------------------------- frame

    fn keys(&mut self, ctx: &egui::Context) {
        // Only text fields keep Ctrl+Z for themselves (grids and canvases may hold focus).
        let typing = ctx.text_edit_focused();
        let (undo, redo, save, open, newp, refresh) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z)),
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y))
                    || i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z)),
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S)),
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O)),
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N)),
                i.key_pressed(egui::Key::F5),
            )
        });
        if !typing {
            if undo {
                self.undo();
            }
            if redo {
                self.redo();
            }
        }
        if save {
            self.save(false);
        }
        if open {
            self.open_dialog();
        }
        if newp {
            self.new_project();
        }
        // In the SQL view F5 runs the script instead.
        if refresh && !(self.view == View::Sql && !self.pq_open) {
            self.needs_refresh = true;
        }
    }

    fn rail(&mut self, ui: &mut Ui, t: &Tokens) {
        egui::Panel::left("rail").exact_size(52.0).resizable(false).frame(egui::Frame::new().fill(t.rail)).show(ui, |ui| {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                for (v, icon, tip) in View::ALL {
                    let active = self.view == v && !self.pq_open;
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, 40.0), egui::Sense::click());
                    if active {
                        ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(28));
                        ui.painter().rect_filled(egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height())), 1.0, t.accent);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(14));
                    }
                    crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0)), icon, if active { Color32::WHITE } else { t.rail_text });
                    if resp.on_hover_text(tip).clicked() {
                        self.view = v;
                        self.pq_open = false;
                    }
                }
                ui.add_space(16.0);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, 40.0), egui::Sense::click());
                if resp.hovered() || self.pq_open {
                    ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(if self.pq_open { 28 } else { 14 }));
                }
                crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0)), "transform", if self.pq_open { Color32::WHITE } else { t.rail_text });
                if resp.on_hover_text("Power Query Editor (Transform data)").clicked() {
                    self.pq_open = !self.pq_open;
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui, t: &Tokens) {
        egui::Panel::bottom("status").exact_size(26.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(10, 4)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let tables = self.built.model.tables.len();
                let rows: usize = self.built.model.tables.iter().map(|t| t.row_count()).sum();
                ui.label(egui::RichText::new(format!("{} · {} tables · {} rows · {} measures", self.project.name, tables, rows, self.project.model.measures.len())).size(11.0).color(t.muted));
                if !self.built.errors.is_empty() {
                    let r = ui.label(egui::RichText::new(format!("⚠ {} issues", self.built.errors.len())).size(11.0).color(t.danger));
                    r.on_hover_text(self.built.errors.iter().map(|(a, b)| format!("{a}: {b}")).collect::<Vec<_>>().join("\n"));
                }
                ui.label(egui::RichText::new(format!("refreshed in {:.0} ms", self.last_refresh_ms)).size(11.0).color(t.muted));
                if let Some(role) = &self.rt.role {
                    ui.label(egui::RichText::new(format!("👁 Viewing as {role}")).size(11.0).color(t.accent));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.view == View::Report {
                        if ui.add(egui::Slider::new(&mut self.report.zoom, 0.25..=3.0).show_value(false)).on_hover_text("Zoom").changed() {
                            self.report.fit = false;
                        }
                        ui.label(egui::RichText::new(format!("{:.0}%", self.report.zoom * 100.0)).size(11.0));
                        if ui.small_button("Fit").clicked() {
                            self.report.fit = true;
                        }
                    }
                    if self.dirty {
                        ui.label(egui::RichText::new("● Unsaved changes").size(11.0).color(t.muted));
                    }
                });
            });
        });
    }

    fn show_toasts(&mut self, ctx: &egui::Context, t: &Tokens) {
        let now = now();
        self.toasts.retain(|x| x.until > now);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("toasts")).anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0)).show(ctx, |ui| {
            for x in self.toasts.iter().rev().take(4) {
                egui::Frame::new().fill(if x.error { Color32::from_rgb(0x7F, 0x1D, 0x1D) } else { Color32::from_rgb(0x1F, 0x29, 0x37) }).corner_radius(6.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(egui::RichText::new(&x.text).color(Color32::WHITE));
                });
                ui.add_space(4.0);
            }
            let _ = t;
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| self.project.name.clone());
        format!("{}{} — VizCraft", name, if self.dirty { " •" } else { "" })
    }
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl eframe::App for VizApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.needs_refresh {
            self.needs_refresh = false;
            self.needs_rebuild = false;
            self.refresh();
        } else if self.needs_rebuild {
            self.needs_rebuild = false;
            self.rebuild();
        }
        let t = self.tokens();
        self.keys(&ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        crate::ribbon::show(self, ui, &t);
        self.status_bar(ui, &t);
        self.rail(ui, &t);
        if self.ai.copilot_open && !self.pq_open && matches!(self.view, View::Report | View::Table | View::Model | View::Dax) {
            crate::ai::copilot::pane(self, ui);
        }
        if self.pq_open {
            crate::pq::show(self, ui);
        } else {
            match self.view {
                View::Report => crate::report::show(self, ui),
                View::Table => crate::model::table_view(self, ui),
                View::Model => crate::model::model_view(self, ui),
                View::Dax => crate::model::dax_view(self, ui),
                View::Tmdl => crate::model::tmdl_view(self, ui),
                View::Sheets => crate::sheet::show(self, ui),
                View::Flows => crate::flows::show(self, ui),
                View::Story => crate::report::story::show(self, ui),
                View::Sql => crate::sqlview::show(self, ui),
                View::Formulator => crate::ai::formulator::show(self, ui),
                View::Ai => crate::ai::hub::show(self, ui),
            }
        }
        crate::pq::dialogs(self, &ctx);
        crate::model::dialogs(self, &ctx);
        crate::sheet::dialogs(self, &ctx);
        crate::flows::dialogs(self, &ctx);
        crate::publish::dialogs(self, &ctx);
        crate::report::dialogs(self, &ctx);
        crate::sqlview::dialogs(self, &ctx);
        crate::ai::dialogs(self, &ctx);
        self.show_toasts(&ctx, &t);
        self.handle_screenshot(&ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.frame += 1;
        let now: Vec<String> = self.synthetic.iter().filter(|(f, _)| *f == self.frame).map(|(_, e)| e.clone()).collect();
        for e in now {
            let mut parts = e.splitn(2, ':');
            let kind = parts.next().unwrap_or("");
            let arg = parts.next().unwrap_or("");
            let pos = || {
                let (x, y) = arg.split_once(',')?;
                Some(egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
            };
            match kind {
                "move" => {
                    if let Some(p) = pos() {
                        raw.events.push(egui::Event::PointerMoved(p));
                    }
                }
                "click" | "rclick" | "down" | "up" => {
                    if let Some(p) = pos() {
                        let button = if kind == "rclick" { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                        raw.events.push(egui::Event::PointerMoved(p));
                        if kind != "up" {
                            raw.events.push(egui::Event::PointerButton { pos: p, button, pressed: true, modifiers: egui::Modifiers::NONE });
                        }
                        if kind != "down" {
                            raw.events.push(egui::Event::PointerButton { pos: p, button, pressed: false, modifiers: egui::Modifiers::NONE });
                        }
                    }
                }
                "key" => {
                    let key = egui::Key::from_name(arg);
                    if let Some(k) = key {
                        raw.events.push(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
                    }
                }
                "text" => raw.events.push(egui::Event::Text(arg.to_string())),
                _ => {}
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("dark", self.dark.to_string());
    }
}

impl VizApp {
    /// `--screenshot path`: saves a PNG of the window after a few frames, then quits.
    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = self.screenshot.clone() else { return };
        if frames == self.screenshot_frame {
            let p2 = path.clone();
            let ctx2 = ctx.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let [w, h] = img.size;
                let mut buf = Vec::with_capacity(w * h * 4);
                for p in &img.pixels {
                    buf.extend_from_slice(&p.to_array());
                }
                if let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
                    let _ = rgba.save(&p2);
                }
                ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
            })));
        }
        if frames > 200 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.screenshot = Some((path, frames + 1));
        ctx.request_repaint();
    }
}
