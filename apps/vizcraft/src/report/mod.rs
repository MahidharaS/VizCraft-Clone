//! Report view: pages, the canvas, visuals, design elements and the report panes.

pub mod actions;
pub mod arrange;
pub mod bookmarks;
pub mod canvas;
pub mod dashboard;
pub mod design;
pub mod filters;
pub mod fx_dialog;
pub mod header;
pub mod layers;
pub mod menus;
pub mod pages;
pub mod panes;
pub mod render;
pub mod script;
pub mod shapes;
pub mod story;
pub mod text;
pub mod worksheet;

use std::collections::HashMap;
use std::sync::mpsc;

use egui::{RichText, Ui};
use vizcraft_core::color::Color;
use vizcraft_core::doc::*;
use vizcraft_core::scalar::Scalar;

use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets::{big_button, column, group, small_button};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragKind {
    Move,
    Resize(u8),
    Rotate,
    Marquee,
}

#[derive(Clone, Debug)]
pub struct Drag {
    pub kind: DragKind,
    pub start: (f32, f32),
    pub originals: Vec<(u64, f32, f32, f32, f32, f32)>,
}

pub struct ReportUi {
    pub zoom: f32,
    pub fit: bool,
    pub fit_width: bool,
    pub mobile: bool,
    pub edit_interactions: bool,
    pub reading: bool,
    pub pan: egui::Vec2,
    pub render_cache: HashMap<(u64, u64), (String, render::Rendered)>,
    pub text_cache: HashMap<String, String>,
    pub script_images: HashMap<u64, String>,
    pub script_errors: HashMap<u64, String>,
    pub script_jobs: Vec<(u64, mpsc::Receiver<Result<Vec<u8>, String>>)>,
    pub slicer_search: HashMap<u64, String>,
    pub scroll: HashMap<u64, f32>,
    pub drag: Option<Drag>,
    pub guides: Vec<(bool, f32)>,
    pub editing_text: Option<u64>,
    pub text_buffer: String,
    pub clipboard: Vec<Element>,
    pub viz_tab: usize,
    pub format_search: String,
    pub field_search: String,
    pub fx: Option<fx_dialog::FxDialog>,
    pub chip_menu: Option<(u64, String, usize)>,
    pub dragging_well: Option<(u64, String, usize)>,
    pub more_menu: Option<u64>,
    pub header_consumed: bool,
    pub last_bookmark: Option<u64>,
    pub renaming_page: Option<(u64, String)>,
    pub show_as_table: Option<u64>,
    pub actions_dialog: bool,
    pub story: Option<(u64, usize)>,
    pub story_applied: Option<u64>,
}

impl Default for ReportUi {
    fn default() -> Self {
        ReportUi {
            zoom: 1.0,
            fit: true,
            fit_width: false,
            mobile: false,
            edit_interactions: false,
            reading: false,
            pan: egui::Vec2::ZERO,
            render_cache: HashMap::new(),
            text_cache: HashMap::new(),
            script_images: HashMap::new(),
            script_errors: HashMap::new(),
            script_jobs: Vec::new(),
            slicer_search: HashMap::new(),
            scroll: HashMap::new(),
            drag: None,
            guides: vec![],
            editing_text: None,
            text_buffer: String::new(),
            clipboard: vec![],
            viz_tab: 0,
            format_search: String::new(),
            field_search: String::new(),
            fx: None,
            chip_menu: None,
            dragging_well: None,
            more_menu: None,
            header_consumed: false,
            last_bookmark: None,
            renaming_page: None,
            show_as_table: None,
            actions_dialog: false,
            story: None,
            story_applied: None,
        }
    }
}

impl ReportUi {
    pub fn on_rebuild(&mut self) {
        self.render_cache.clear();
        self.text_cache.clear();
    }
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    script::poll(app);
    keyboard(app, ui);
    pages::tabs(app, ui, &t);
    let kind = app.current_page().map(|p| p.kind).unwrap_or_default();
    // Right panes.
    let pane_frame = egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(8)).stroke(egui::Stroke::new(1.0, t.border));
    if app.panes.data {
        egui::Panel::right("pane-data").default_size(215.0).size_range(170.0..=420.0).frame(pane_frame).show(ui, |ui| panes::data_pane(app, ui, &t));
    } else {
        collapsed_strip(ui, &t, "strip-data", "Data", &mut app.panes.data);
    }
    if app.panes.visualizations {
        egui::Panel::right("pane-viz").default_size(250.0).size_range(210.0..=480.0).frame(pane_frame).show(ui, |ui| panes::visualizations(app, ui, &t));
    } else {
        collapsed_strip(ui, &t, "strip-viz", "Visualizations", &mut app.panes.visualizations);
    }
    if app.panes.filters {
        egui::Panel::right("pane-filters").default_size(210.0).size_range(170.0..=400.0).frame(pane_frame).show(ui, |ui| filters::pane(app, ui, &t));
    } else {
        collapsed_strip(ui, &t, "strip-filters", "Filters", &mut app.panes.filters);
    }
    if app.panes.bookmarks {
        egui::Panel::right("pane-bm").default_size(220.0).frame(pane_frame).show(ui, |ui| bookmarks::pane(app, ui, &t));
    }
    if app.panes.selection {
        egui::Panel::right("pane-sel").default_size(220.0).frame(pane_frame).show(ui, |ui| layers::selection_pane(app, ui, &t));
    }
    if app.panes.sync_slicers {
        egui::Panel::right("pane-sync").default_size(240.0).frame(pane_frame).show(ui, |ui| layers::sync_pane(app, ui, &t));
    }
    if app.panes.performance {
        egui::Panel::right("pane-perf").default_size(260.0).frame(pane_frame).show(ui, |ui| layers::performance_pane(app, ui, &t));
    }
    if app.report.mobile {
        pages::mobile(app, ui, &t);
        return;
    }
    match kind {
        PageKind::Worksheet => {
            egui::Panel::left("marks-card").default_size(190.0).frame(pane_frame).show(ui, |ui| worksheet::marks_card(app, ui, &t));
            worksheet::shelves(app, ui, &t);
        }
        PageKind::Dashboard => {
            egui::Panel::left("dash-pane").default_size(210.0).frame(pane_frame).show(ui, |ui| dashboard::pane(app, ui, &t));
        }
        _ => {}
    }
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas)).show(ui, |ui| {
        canvas::show(app, ui, &t);
        if let Some(id) = app.report.editing_text
            && let Some((_, el)) = app.project.element(id)
        {
            let z = app.report.zoom;
            let r = egui::Rect::from_min_size(ui.max_rect().min + egui::vec2(24.0 + el.x * z, 24.0 + el.y * z), egui::vec2(el.w * z, el.h * z));
            text::inline_editor(app, ui, r);
        }
    });
}

/// A collapsed pane: a thin vertical strip with its name; click to expand.
fn collapsed_strip(ui: &mut Ui, t: &Tokens, id: &str, label: &str, open: &mut bool) {
    egui::Panel::right(id.to_string()).exact_size(26.0).resizable(false).frame(egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        let r = ui.max_rect();
        let resp = ui.interact(r, ui.scope_id().with(id), egui::Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.accent_soft);
        }
        let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), t.text);
        let pos = egui::pos2(r.center().x - galley.size().y / 2.0, r.top() + 14.0);
        let mut shape = egui::epaint::TextShape::new(pos + egui::vec2(galley.size().y, 0.0), galley, t.text);
        shape.angle = std::f32::consts::FRAC_PI_2;
        ui.painter().add(shape);
        if resp.on_hover_text(format!("Show the {label} pane")).clicked() {
            *open = true;
        }
    });
}

fn keyboard(app: &mut VizApp, ui: &Ui) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let (del, esc, copy, paste, cut, dup, group_k, ungroup_k, all, arrows, shift) = ui.input_mut(|i| {
        let cmd = egui::Modifiers::COMMAND;
        (
            i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            i.key_pressed(egui::Key::Escape),
            i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::C)) || i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::V)) || i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
            i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::X)) || i.events.iter().any(|e| matches!(e, egui::Event::Cut)),
            i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::D)),
            i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::G)),
            i.consume_shortcut(&egui::KeyboardShortcut::new(cmd | egui::Modifiers::SHIFT, egui::Key::G)),
            i.consume_shortcut(&egui::KeyboardShortcut::new(cmd, egui::Key::A)),
            (
                i.key_pressed(egui::Key::ArrowLeft) as i32 * -1 + i.key_pressed(egui::Key::ArrowRight) as i32,
                i.key_pressed(egui::Key::ArrowUp) as i32 * -1 + i.key_pressed(egui::Key::ArrowDown) as i32,
            ),
            i.modifiers.shift,
        )
    });
    if del && !app.selected.is_empty() {
        delete_selected(app);
    }
    if esc {
        app.selected.clear();
        app.rt.selection = None;
        app.rt.focus = None;
        app.report.render_cache.clear();
    }
    if copy {
        clipboard(app, ui.ctx(), "copy");
    }
    if cut {
        clipboard(app, ui.ctx(), "cut");
    }
    if paste {
        clipboard(app, ui.ctx(), "paste");
    }
    if dup {
        duplicate(app);
    }
    if ungroup_k {
        arrange::ungroup(app);
    } else if group_k {
        arrange::group(app);
    }
    if all && let Some(p) = app.current_page() {
        app.selected = p.elements.iter().filter(|e| e.parent.is_none()).map(|e| e.id).collect();
    }
    if (arrows.0 != 0 || arrows.1 != 0) && !app.selected.is_empty() {
        let step = if shift { 10.0 } else { 1.0 };
        let sel = app.selected.clone();
        app.checkpoint("Nudge");
        if let Some(pg) = app.current_page_mut() {
            let mut ids = sel.clone();
            ids.extend(pg.elements.iter().filter(|e| e.parent.is_some_and(|p| sel.contains(&p))).map(|e| e.id));
            for e in pg.elements.iter_mut().filter(|e| ids.contains(&e.id)) {
                e.x += arrows.0 as f32 * step;
                e.y += arrows.1 as f32 * step;
            }
        }
    }
}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    fx_dialog::show(app, ctx);
    dashboard::actions_dialog(app, ctx);
    if let Some(id) = app.report.show_as_table {
        let mut open = true;
        let page = app.page;
        let data = app.report.render_cache.get(&(id, page)).and_then(|(_, r)| r.data.clone());
        egui::Window::new("Data behind the visual").open(&mut open).default_size([640.0, 380.0]).show(ctx, |ui| {
            let Some(d) = data else {
                ui.label("No data.");
                return;
            };
            egui::ScrollArea::both().show(ui, |ui| {
                egui::Grid::new("sat").striped(true).show(ui, |ui| {
                    for c in &d.cols {
                        ui.label(RichText::new(&c.label).strong());
                    }
                    ui.end_row();
                    for r in 0..d.rows.len().min(2000) {
                        for c in 0..d.cols.len() {
                            ui.label(d.text(r, c));
                        }
                        ui.end_row();
                    }
                });
            });
        });
        if !open {
            app.report.show_as_table = None;
        }
    }
}

// ============================================================== element operations

fn place(app: &mut VizApp, mut el: Element) {
    let id = app.project.next_id();
    el.id = id;
    let (pw, ph) = app.current_page().map(|p| (p.width, p.height)).unwrap_or((1280.0, 720.0));
    // Cascade new elements so they don't stack exactly.
    let n = app.current_page().map(|p| p.elements.len()).unwrap_or(0) as f32;
    el.x = (pw / 2.0 - el.w / 2.0 + (n % 6.0) * 16.0).max(0.0);
    el.y = (ph / 2.0 - el.h / 2.0 + (n % 6.0) * 16.0).max(0.0);
    app.checkpoint("Insert");
    if let Some(pg) = app.current_page_mut() {
        pg.elements.push(el);
    }
    app.selected = vec![id];
    app.view = crate::app::View::Report;
    app.pq_open = false;
}

pub fn insert_visual(app: &mut VizApp, kind: VisualKind) {
    let (w, h) = match kind {
        VisualKind::Card | VisualKind::Kpi => (240.0, 120.0),
        VisualKind::Slicer => (220.0, 220.0),
        VisualKind::Gauge => (300.0, 220.0),
        VisualKind::Table | VisualKind::Matrix => (520.0, 300.0),
        _ => (460.0, 300.0),
    };
    let mut v = VisualSpec::new(kind);
    if matches!(kind, VisualKind::PythonVisual) {
        v.script = script::PY_TEMPLATE.into();
    }
    if matches!(kind, VisualKind::RVisual) {
        v.script = script::R_TEMPLATE.into();
    }
    let el = Element { name: kind.label().into(), w, h, kind: ElementKind::Visual(v), ..Element::default() };
    place(app, el);
    app.panes.visualizations = true;
    app.report.viz_tab = 0;
}

pub fn insert_text(app: &mut VizApp) {
    let el = Element {
        name: "Text box".into(),
        w: 360.0,
        h: 80.0,
        kind: ElementKind::Text(TextSpec { text: vizcraft_core::doc::RichText { runs: vec![TextRun { text: "Title with a live value: {[".to_string() + &app.project.model.measures.first().map(|m| m.name.clone()).unwrap_or_else(|| "Measure".into()) + "]}", size: 18.0, bold: true, ..TextRun::default() }], align: "left".into(), valign: "top".into(), line_height: 1.2 }, smart_narrative: false }),
        ..Element::default()
    };
    place(app, el);
}

pub fn insert_shape(app: &mut VizApp, kind: ShapeKind) {
    let (w, h) = match kind {
        ShapeKind::Line => (240.0, 4.0),
        ShapeKind::Frame => (480.0, 300.0),
        _ => (200.0, 140.0),
    };
    let mut spec = ShapeSpec { shape: kind, ..ShapeSpec::default() };
    if kind == ShapeKind::Frame {
        spec.fill = Fill::Solid(Color::rgba(255, 255, 255, 255));
        spec.stroke = Stroke { width: 1.0, color: Color::rgb(0xDF, 0xE3, 0xE8), dash: "solid".into() };
    }
    if kind == ShapeKind::Path {
        spec.points = vec![(0.0, 1.0), (0.3, 0.2), (0.6, 0.7), (1.0, 0.0)];
        spec.closed = false;
        spec.stroke = Stroke { width: 3.0, color: Color::rgb(0x2B, 0x6C, 0xE0), dash: "solid".into() };
        spec.fill = Fill::None;
    }
    let mut el = Element { name: format!("{kind:?}"), w, h, kind: ElementKind::Shape(spec), ..Element::default() };
    el.style.padding = 0.0;
    if kind == ShapeKind::Frame {
        el.style.radius = [12.0; 4];
    }
    place(app, el);
}

pub fn insert_button(app: &mut VizApp, kind: &str) {
    let mut b = ButtonSpec { icon: kind.to_string(), ..ButtonSpec::default() };
    let (w, h) = match kind {
        "Page navigator" => {
            b.navigator = Some("pages".into());
            (600.0, 40.0)
        }
        "Bookmark navigator" => {
            b.navigator = Some("bookmarks".into());
            (480.0, 40.0)
        }
        "Back" => {
            b.text = vizcraft_core::doc::RichText::plain("Back");
            b.action = Some(Action::Back);
            (110.0, 36.0)
        }
        "Bookmark" => {
            b.text = vizcraft_core::doc::RichText::plain("Bookmark");
            b.action = app.project.report.bookmarks.first().map(|bm| Action::Bookmark { id: bm.id });
            (140.0, 36.0)
        }
        "Q&A" => {
            b.text = vizcraft_core::doc::RichText::plain("Ask a question");
            b.action = Some(Action::QandA);
            (160.0, 36.0)
        }
        "Reset" => {
            b.text = vizcraft_core::doc::RichText::plain("Clear slicers");
            b.action = Some(Action::ClearAllSlicers);
            (150.0, 36.0)
        }
        "Web" => {
            b.text = vizcraft_core::doc::RichText::plain("Open link");
            b.action = Some(Action::Url { url: "https://".into() });
            (140.0, 36.0)
        }
        _ => {
            b.text = vizcraft_core::doc::RichText::plain(if kind == "Blank" { "Button" } else { kind });
            (140.0, 36.0)
        }
    };
    let mut el = Element { name: format!("{kind} button"), w, h, kind: ElementKind::Button(b), ..Element::default() };
    el.style.padding = 0.0;
    place(app, el);
}

pub fn insert_image(app: &mut VizApp) {
    let mut source = String::new();
    if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg"]).pick_file()
        && let Ok(bytes) = std::fs::read(&path)
    {
        source = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "image".into());
        app.project.report.images.insert(source.clone(), b64_encode(&bytes));
    }
    let el = Element { name: "Image".into(), w: 240.0, h: 160.0, kind: ElementKind::Image(ImageSpec { source, scaling: "fit".into(), action: None }), ..Element::default() };
    place(app, el);
}

pub fn new_page(app: &mut VizApp, kind: PageKind) {
    app.checkpoint("New page");
    let id = app.project.next_id();
    let n = app.project.report.pages.iter().filter(|p| p.kind == kind).count() + 1;
    let name = match kind {
        PageKind::Worksheet => format!("Sheet {n}"),
        PageKind::Dashboard => format!("Dashboard {n}"),
        PageKind::Tooltip => format!("Tooltip {n}"),
        PageKind::Drillthrough => format!("Details {n}"),
        PageKind::Report => format!("Page {}", app.project.report.pages.len() + 1),
    };
    let mut page = Page::new(id, &name, kind);
    if kind == PageKind::Worksheet {
        let vid = app.project.next_id();
        let mut v = VisualSpec::new(VisualKind::StackedColumn);
        v.set("ws.columns", vec![]);
        page.elements.push(Element { id: vid, name: name.clone(), x: 0.0, y: 0.0, w: page.width, h: page.height, kind: ElementKind::Visual(v), ..Element::default() });
        page.worksheet_visual = Some(vid);
    }
    if kind == PageKind::Dashboard {
        page.size_preset = "Dashboard 1366".into();
        page.width = 1366.0;
        page.height = 768.0;
    }
    app.project.report.pages.push(page);
    app.go_to_page(id);
    app.report.fit = true;
}

pub fn duplicate_page(app: &mut VizApp) {
    let Some(mut p) = app.current_page().cloned() else { return };
    app.checkpoint("Duplicate page");
    p.id = app.project.next_id();
    p.name = format!("Duplicate of {}", p.name);
    let mut map = HashMap::new();
    for e in p.elements.iter_mut() {
        let nid = app.project.next_id();
        map.insert(e.id, nid);
        e.id = nid;
    }
    for e in p.elements.iter_mut() {
        e.parent = e.parent.and_then(|x| map.get(&x).copied());
    }
    p.worksheet_visual = p.worksheet_visual.and_then(|x| map.get(&x).copied());
    let id = p.id;
    app.project.report.pages.push(p);
    app.go_to_page(id);
}

/// The ↗ arrow: opens a visual on its own new worksheet page (Tableau-style sheet).
pub fn open_as_sheet(app: &mut VizApp, element: u64) {
    let Some((pid, el)) = app.project.element(element).map(|(p, e)| (p, e.clone())) else { return };
    let Some(mut v) = el.visual().cloned() else { return };
    // Already opened before? Go there.
    if let Some(existing) = app.project.report.pages.iter().find(|p| p.kind == PageKind::Worksheet && p.opened_from == Some(pid) && p.name == sheet_name(&el)).map(|p| p.id) {
        app.go_to_page(existing);
        return;
    }
    app.checkpoint("Open as sheet");
    worksheet::shelves_from_wells(&mut v);
    let id = app.project.next_id();
    let vid = app.project.next_id();
    let mut page = Page::new(id, &sheet_name(&el), PageKind::Worksheet);
    page.opened_from = Some(pid);
    let mut e2 = el.clone();
    e2.id = vid;
    e2.x = 0.0;
    e2.y = 0.0;
    e2.w = page.width;
    e2.h = page.height;
    e2.kind = ElementKind::Visual(v);
    page.elements.push(e2);
    page.worksheet_visual = Some(vid);
    // Insert right after the source page.
    let at = app.project.report.pages.iter().position(|p| p.id == pid).map(|i| i + 1).unwrap_or(app.project.report.pages.len());
    app.project.report.pages.insert(at, page);
    app.go_to_page(id);
    app.report.fit = true;
    app.toast("Opened as a worksheet: edit it with the Columns/Rows shelves and the Marks card.");
}

fn sheet_name(el: &Element) -> String {
    let base = if el.name.is_empty() { el.kind_label() } else { el.name.clone() };
    format!("Sheet: {base}")
}

pub fn delete_selected(app: &mut VizApp) {
    let sel = app.selected.clone();
    if sel.is_empty() {
        return;
    }
    app.checkpoint("Delete");
    if let Some(pg) = app.current_page_mut() {
        let mut ids = sel.clone();
        let mut i = 0;
        while i < ids.len() {
            let cur = ids[i];
            ids.extend(pg.elements.iter().filter(|e| e.parent == Some(cur)).map(|e| e.id));
            i += 1;
        }
        pg.elements.retain(|e| !ids.contains(&e.id));
    }
    app.selected.clear();
    app.rt.selection = None;
}

pub fn duplicate(app: &mut VizApp) {
    clipboard_copy(app);
    paste_elements(app, 20.0);
}

fn clipboard_copy(app: &mut VizApp) {
    let sel = app.selected.clone();
    let Some(pg) = app.current_page() else { return };
    let mut ids = sel.clone();
    let mut i = 0;
    while i < ids.len() {
        let cur = ids[i];
        ids.extend(pg.elements.iter().filter(|e| e.parent == Some(cur)).map(|e| e.id));
        i += 1;
    }
    app.report.clipboard = pg.elements.iter().filter(|e| ids.contains(&e.id)).cloned().collect();
}

fn paste_elements(app: &mut VizApp, offset: f32) {
    if app.report.clipboard.is_empty() {
        return;
    }
    app.checkpoint("Paste");
    let mut map = HashMap::new();
    let mut els = app.report.clipboard.clone();
    for e in els.iter_mut() {
        let nid = app.project.next_id();
        map.insert(e.id, nid);
        e.id = nid;
        e.x += offset;
        e.y += offset;
    }
    for e in els.iter_mut() {
        e.parent = e.parent.and_then(|p| map.get(&p).copied());
    }
    let tops: Vec<u64> = els.iter().filter(|e| e.parent.is_none()).map(|e| e.id).collect();
    if let Some(pg) = app.current_page_mut() {
        pg.elements.extend(els);
    }
    app.selected = tops;
}

pub fn clipboard(app: &mut VizApp, ctx: &egui::Context, op: &str) {
    match op {
        "copy" => {
            clipboard_copy(app);
            ctx.copy_text(serde_json::to_string(&app.report.clipboard).unwrap_or_default());
        }
        "cut" => {
            clipboard_copy(app);
            delete_selected(app);
        }
        _ => paste_elements(app, 16.0),
    }
}

/// A tooltip page rendered for a hovered data point.
pub fn tooltip_page(app: &mut VizApp, ui: &mut Ui, tp: &Page, point: &[(FieldRef, Scalar)]) {
    let saved = app.rt.drillthrough.get(&tp.id).cloned();
    app.rt.drillthrough.insert(tp.id, point.to_vec());
    let w = tp.width.min(420.0);
    let zoom = w / tp.width.max(1.0);
    let (r, _) = ui.allocate_exact_size(egui::vec2(tp.width * zoom, tp.height * zoom), egui::Sense::hover());
    let t = app.tokens();
    // Tooltip pages must not hit the render cache of the same element on other contexts.
    for e in &tp.elements {
        app.report.render_cache.remove(&(e.id, tp.id));
    }
    canvas::paint_page_static(app, ui, ui.painter(), tp, canvas::PageXf { origin: r.min, zoom }, &t);
    match saved {
        Some(s) => {
            app.rt.drillthrough.insert(tp.id, s);
        }
        None => {
            app.rt.drillthrough.remove(&tp.id);
        }
    }
    for e in &tp.elements {
        app.report.render_cache.remove(&(e.id, tp.id));
    }
}

/// Drill through from the selected data point of a visual to a drill-through page.
pub fn drillthrough_from_selection(app: &mut VizApp, source: u64, page: u64) {
    let point = app.rt.selection.as_ref().filter(|s| s.source == source).and_then(|s| s.points.first().cloned()).unwrap_or_default();
    if point.is_empty() {
        app.toast("Select a data point first, then drill through.");
        return;
    }
    app.rt.drillthrough.insert(page, point);
    app.go_to_page(page);
}

pub fn export_data(app: &mut VizApp, id: u64) {
    let page = app.page;
    let data = app.report.render_cache.get(&(id, page)).and_then(|(_, r)| r.data.clone());
    let Some(d) = data else { return };
    let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name("data.csv").save_file() else { return };
    let esc = |s: String| if s.contains(',') || s.contains('"') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s };
    let mut out = d.cols.iter().map(|c| esc(c.label.clone())).collect::<Vec<_>>().join(",") + "\n";
    for r in 0..d.rows.len() {
        out += &(0..d.cols.len()).map(|c| esc(d.text(r, c))).collect::<Vec<_>>().join(",");
        out.push('\n');
    }
    match std::fs::write(&path, out) {
        Ok(()) => app.toast(format!("Exported {}", path.display())),
        Err(e) => app.error(format!("Export failed: {e}")),
    }
}

// ============================================================== contextual ribbon tabs

pub fn ribbon_format(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Interactions", |ui| {
        if big_button(ui, t, "filter", "Edit interactions", "Choose how the selected visual filters the others").clicked() {
            app.report.edit_interactions = !app.report.edit_interactions;
        }
    });
    group(ui, t, "Drill", |ui| {
        let id = app.selected.first().copied();
        column(ui, |ui| {
            if small_button(ui, t, "", "↑ Drill up", "").clicked()
                && let Some(id) = id
            {
                actions::drill(app, id, "up");
            }
            if small_button(ui, t, "", "↓ Drill mode", "").clicked()
                && let Some(id) = id
            {
                actions::drill(app, id, "mode");
            }
            if small_button(ui, t, "", "⇊ Next level", "").clicked()
                && let Some(id) = id
            {
                actions::drill(app, id, "next");
            }
        });
        column(ui, |ui| {
            if small_button(ui, t, "", "Expand next level", "").clicked()
                && let Some(id) = id
            {
                actions::drill(app, id, "expand");
            }
            if small_button(ui, t, "worksheet", "Open as sheet", "Open this visual on its own worksheet page").clicked()
                && let Some(id) = id
            {
                open_as_sheet(app, id);
            }
            if small_button(ui, t, "", "Show as table", "").clicked() {
                app.report.show_as_table = id;
            }
        });
    });
    group(ui, t, "Format", |ui| {
        if big_button(ui, t, "theme", "Format pane", "Open the Format tab of the Visualizations pane").clicked() {
            app.panes.visualizations = true;
            app.report.viz_tab = 1;
        }
        if big_button(ui, t, "sparkle", "Analytics", "Reference lines, trend, forecast, clusters").clicked() {
            app.panes.visualizations = true;
            app.report.viz_tab = 2;
        }
    });
}

pub fn ribbon_arrange(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Arrange", |ui| {
        column(ui, |ui| {
            if small_button(ui, t, "", "Bring forward", "Ctrl+]").clicked() {
                arrange::order(app, "forward");
            }
            if small_button(ui, t, "", "Bring to front", "").clicked() {
                arrange::order(app, "front");
            }
            if small_button(ui, t, "", "Send backward", "").clicked() {
                arrange::order(app, "backward");
            }
        });
        column(ui, |ui| {
            if small_button(ui, t, "", "Send to back", "").clicked() {
                arrange::order(app, "back");
            }
            if small_button(ui, t, "", "Group (Ctrl+G)", "").clicked() {
                arrange::group(app);
            }
            if small_button(ui, t, "", "Ungroup", "Ctrl+Shift+G").clicked() {
                arrange::ungroup(app);
            }
        });
    });
    group(ui, t, "Align", |ui| {
        column(ui, |ui| {
            for (l, op) in [("Align left", "left"), ("Align center", "center"), ("Align right", "right")] {
                if small_button(ui, t, "", l, "").clicked() {
                    arrange::align(app, op);
                }
            }
        });
        column(ui, |ui| {
            for (l, op) in [("Align top", "top"), ("Align middle", "middle"), ("Align bottom", "bottom")] {
                if small_button(ui, t, "", l, "").clicked() {
                    arrange::align(app, op);
                }
            }
        });
        column(ui, |ui| {
            if small_button(ui, t, "", "Distribute horizontally", "").clicked() {
                arrange::align(app, "dist-h");
            }
            if small_button(ui, t, "", "Distribute vertically", "").clicked() {
                arrange::align(app, "dist-v");
            }
            if small_button(ui, t, "", "Match width & height", "").clicked() {
                arrange::match_size(app, true, true);
            }
        });
    });
    group(ui, t, "Panes", |ui| {
        if big_button(ui, t, "model", "Selection", "Layers pane").clicked() {
            app.panes.selection = !app.panes.selection;
        }
    });
}

// ============================================================== base64

/// Standard base64 decoding.
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.split(',').next_back().unwrap_or(s);
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
