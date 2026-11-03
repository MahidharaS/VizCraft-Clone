//! Data sheets: Excel-like editing of data (enter, paste, edit, formulas, fill, sort, filter,
//! find & replace), separate from Power Query (which is for cleaning). Sheets with "Load to
//! model" on become model tables.

mod grid;
pub mod ops;

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use egui::{Color32, RichText, Ui, vec2};
use vizcraft_core::doc::DataSheet;
use vizcraft_core::sheet::{self, CellValue};

use crate::app::VizApp;
use crate::theme::Tokens;
use ops::Range;

pub const FORMATS: &[(&str, &str)] = &[("", "General"), ("#,0", "Number"), ("#,0.00", "Number (2 decimals)"), ("$#,0.00", "Currency"), ("0%", "Percent"), ("0.0%", "Percent (1 decimal)"), ("yyyy-mm-dd", "Date"), ("dd mmm yyyy", "Long date"), ("0.00E+00", "Scientific")];
pub const TYPES: &[(&str, &str)] = &[("", "Automatic"), ("Text", "Text"), ("Whole Number", "Whole number"), ("Decimal Number", "Decimal number"), ("Date", "Date"), ("Date/Time", "Date/time"), ("True/False", "True/false")];

/// An in-progress cell edit.
#[derive(Clone, Debug)]
pub struct Edit {
    pub r: usize,
    pub c: usize,
    pub text: String,
    pub original: String,
    /// Started by typing (arrow keys commit) rather than F2 / double-click (arrows move the caret).
    pub typed: bool,
    pub focus_cell: bool,
    pub focus_bar: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FindState {
    pub open: bool,
    pub replace_mode: bool,
    pub find: String,
    pub replace: String,
    pub match_case: bool,
    pub whole: bool,
    pub formulas: bool,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct FilterPopup {
    pub col: usize,
    pub pos: egui::Pos2,
    pub search: String,
    /// Distinct display values and whether they're shown.
    pub values: Vec<(String, bool)>,
}

pub(crate) struct GridCache {
    pub key: u64,
    pub values: Vec<Vec<CellValue>>,
    /// Data rows (≥ 1) passing the filters, in order.
    pub visible: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Drag {
    None,
    Select,
    Fill,
    ColResize { col: usize, start_w: f32, start_x: f32 },
}

pub struct SheetUi {
    pub active: usize,
    pub anchor: (usize, usize),
    pub cursor: (usize, usize),
    pub edit: Option<Edit>,
    pub grid_focus: bool,
    pub(crate) cache: Option<GridCache>,
    /// Hidden values per sheet and column (filters).
    pub filters: BTreeMap<String, BTreeMap<usize, BTreeSet<String>>>,
    pub filter_buttons: bool,
    pub filter_popup: Option<FilterPopup>,
    pub freeze_header: bool,
    pub(crate) scroll_to: Option<(usize, usize)>,
    pub(crate) offset: egui::Vec2,
    pub(crate) drag: Drag,
    pub(crate) fill_target: Option<Range>,
    pub find: FindState,
    pub renaming: Option<(usize, String)>,
    pub from_query: Option<String>,
    /// Last copy: the TSV put on the clipboard, raw inputs and their origin.
    pub(crate) clip: Option<(String, Vec<Vec<String>>, (usize, usize))>,
    pub name_box: Option<String>,
    pub(crate) env_checked: bool,
    pub(crate) view_rows: usize,
    pub(crate) rows_end_cache: usize,
}

impl Default for SheetUi {
    fn default() -> Self {
        SheetUi {
            active: 0,
            anchor: (1, 0),
            cursor: (1, 0),
            edit: None,
            grid_focus: true,
            cache: None,
            filters: BTreeMap::new(),
            filter_buttons: true,
            filter_popup: None,
            freeze_header: true,
            scroll_to: None,
            offset: egui::Vec2::ZERO,
            drag: Drag::None,
            fill_target: None,
            find: FindState::default(),
            renaming: None,
            from_query: None,
            clip: None,
            name_box: None,
            env_checked: false,
            view_rows: 30,
            rows_end_cache: 1,
        }
    }
}

impl SheetUi {
    pub fn selection(&self) -> Range {
        Range::new(self.anchor, self.cursor)
    }
    pub fn select(&mut self, r: usize, c: usize, extend: bool) {
        self.cursor = (r, c);
        if !extend {
            self.anchor = (r, c);
        }
        self.scroll_to = Some((r, c));
    }
}

// =============================================================================================
// Public entry points
// =============================================================================================

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let mut st = std::mem::take(&mut app.sheet_ui);
    let t = app.tokens();
    if st.active >= app.project.sheets.len() {
        st.active = app.project.sheets.len().saturating_sub(1);
    }
    if app.project.sheets.is_empty() {
        empty_state(app, &mut st, ui, &t);
        app.sheet_ui = st;
        return;
    }
    refresh_cache(app, &mut st);
    toolbar(app, &mut st, ui, &t);
    formula_bar(app, &mut st, ui, &t);
    tabs_bar(app, &mut st, ui, &t);
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel)).show(ui, |ui| {
        grid::grid(app, &mut st, ui, &t);
    });
    app.sheet_ui = st;
}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let mut st = std::mem::take(&mut app.sheet_ui);
    if !st.env_checked {
        st.env_checked = true;
        if let Ok(v) = std::env::var("VIZCRAFT_OPEN") {
            match v.as_str() {
                "sheet-find" => {
                    st.find.open = true;
                    st.find.replace_mode = true;
                    st.find.find = "Online".into();
                    st.find.replace = "Web".into();
                }
                "sheet-from-query" => st.from_query = Some(String::new()),
                "sheet-filter" => {
                    st.filter_popup = Some(FilterPopup { col: 4, pos: egui::pos2(560.0, 250.0), search: String::new(), values: vec![] });
                }
                "sheet-demo" => {
                    // A small demo sheet with formulas.
                    demo_sheet(app);
                    st.active = app.project.sheets.len().saturating_sub(1);
                    st.anchor = (1, 4);
                    st.cursor = (1, 4);
                }
                "sheet-edit" => {
                    demo_sheet(app);
                    st.active = app.project.sheets.len().saturating_sub(1);
                    st.cache = None;
                    st.edit = Some(Edit { r: 6, c: 3, text: "=ROUND(E6/B6, 2) + COU".into(), original: String::new(), typed: true, focus_cell: true, focus_bar: false });
                    st.anchor = (6, 3);
                    st.cursor = (6, 3);
                }
                _ => {}
            }
        }
    }
    if app.view == crate::app::View::Sheets && !app.pq_open {
        find_dialog(app, &mut st, ctx);
        from_query_dialog(app, &mut st, ctx);
        grid::filter_popup(app, &mut st, ctx);
    }
    app.sheet_ui = st;
}

/// A new empty sheet.
pub fn new_sheet(app: &mut VizApp) {
    let name = app.project.free_table_name("Sheet1");
    let name = if name == "Sheet1" { name } else { app.project.free_table_name("Sheet") };
    app.checkpoint("New data sheet");
    let mut cells = vec![vec!["Column1".to_string(), "Column2".into(), "Column3".into()]];
    cells.extend((0..3).map(|_| vec![]));
    app.project.sheets.push(DataSheet { name: name.clone(), cells, load: true, description: "Entered data".into(), ..DataSheet::default() });
    app.sheet_ui.active = app.project.sheets.len() - 1;
    app.sheet_ui.anchor = (0, 0);
    app.sheet_ui.cursor = (0, 0);
    app.view = crate::app::View::Sheets;
    app.pq_open = false;
    app.mark_model_dirty();
    app.toast(format!("New data sheet “{name}”: type to enter data."));
}

/// Copies a query's output into an editable sheet.
pub fn sheet_from_query(app: &mut VizApp, query: &str) {
    let host = vizcraft_connectors::DesktopHost::default();
    let mt = match vizcraft_core::build::evaluate_query(&app.project, query, &host) {
        Ok(t) => t,
        Err(e) => {
            app.error(format!("Couldn't evaluate “{query}”: {e}"));
            return;
        }
    };
    let td = vizcraft_core::build::TableData::from_m(query, &mt);
    let mut cells = vec![td.columns.clone()];
    for r in &td.rows {
        cells.push(
            r.iter()
                .enumerate()
                .map(|(i, v)| match v {
                    vizcraft_core::Scalar::Null => String::new(),
                    vizcraft_core::Scalar::Num(n) if td.types.get(i).is_some_and(|t| t.is_temporal()) => vizcraft_core::numfmt::format_date(*n, if n.fract() == 0.0 { "yyyy-mm-dd" } else { "yyyy-mm-dd hh:mm" }),
                    vizcraft_core::Scalar::Num(n) => format!("{n}"),
                    vizcraft_core::Scalar::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
                    vizcraft_core::Scalar::Text(t) => if sheet::parse_input(t) != vizcraft_core::Scalar::Text(t.clone()) || t.starts_with('=') { format!("'{t}") } else { t.clone() },
                })
                .collect(),
        );
    }
    let col_types: Vec<String> = td.types.iter().map(|t| match t {
        vizcraft_core::build::ColType::Any => String::new(),
        vizcraft_core::build::ColType::Number => "Decimal Number".into(),
        other => other.label().to_string(),
    }).collect();
    let col_formats: Vec<String> = td.types.iter().map(|t| if t.is_temporal() { "yyyy-mm-dd".to_string() } else { String::new() }).collect();
    let name = app.project.free_table_name(&format!("{query} (sheet)"));
    app.checkpoint("Sheet from query");
    app.project.sheets.push(DataSheet { name: name.clone(), cells, col_types, col_formats, load: false, from_query: Some(query.to_string()), description: format!("Editable copy of the query “{query}”."), ..DataSheet::default() });
    app.sheet_ui.active = app.project.sheets.len() - 1;
    app.sheet_ui.select(1, 0, false);
    app.view = crate::app::View::Sheets;
    app.pq_open = false;
    app.toast(format!("“{name}”: an editable copy of {query}. Turn on Load to model to use it in reports."));
}

fn demo_sheet(app: &mut VizApp) {
    let rows: Vec<Vec<&str>> = vec![
        vec!["Product", "Units", "Price", "Discount", "Revenue", "Tier"],
        vec!["Trail Runner", "12", "120", "0.1", "=B2*C2*(1-D2)", "=IF(E2>1000,\"High\",\"Low\")"],
        vec!["City Sneaker", "30", "85", "0", "=B3*C3*(1-D3)", "=IF(E3>1000,\"High\",\"Low\")"],
        vec!["Rain Jacket", "7", "140", "0.15", "=B4*C4*(1-D4)", "=IF(E4>1000,\"High\",\"Low\")"],
        vec!["Headlamp", "45", "40", "0.05", "=B5*C5*(1-D5)", "=IF(E5>1000,\"High\",\"Low\")"],
        vec!["Total", "=SUM(B2:B5)", "=AVERAGE(C2:C5)", "", "=SUM(E2:E5)", "=COUNTIF(F2:F5,\"High\")&\" high\""],
    ];
    let name = app.project.free_table_name("Price list");
    app.project.sheets.push(DataSheet {
        name,
        cells: rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect(),
        col_widths: vec![140.0],
        col_formats: vec![String::new(), "#,0".into(), "$#,0.00".into(), "0%".into(), "$#,0.00".into()],
        load: false,
        ..DataSheet::default()
    });
}

// =============================================================================================
// Evaluation cache
// =============================================================================================

fn refresh_cache(app: &VizApp, st: &mut SheetUi) {
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    st.active.hash(&mut h);
    for sh in &app.project.sheets {
        sh.name.hash(&mut h);
        sh.cells.hash(&mut h);
        sh.col_types.hash(&mut h);
    }
    if let Some(f) = st.filters.get(&s.name) {
        for (c, set) in f {
            c.hash(&mut h);
            set.hash(&mut h);
        }
    }
    let key = h.finish();
    if st.cache.as_ref().is_some_and(|c| c.key == key) {
        return;
    }
    let values = sheet::evaluate_grid_with(s, &app.project.sheets);
    let filters = st.filters.get(&s.name);
    let visible = (1..s.cells.len())
        .filter(|r| match filters {
            None => true,
            Some(f) => f.iter().all(|(c, hidden)| hidden.is_empty() || !hidden.contains(&display_value(s, &values, *r, *c))),
        })
        .collect();
    st.cache = Some(GridCache { key, values, visible });
}

/// The text a cell shows (with the column format).
pub fn display_value(s: &DataSheet, grid: &[Vec<CellValue>], r: usize, c: usize) -> String {
    let v = grid.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
    v.display(&cell_format(s, r, c))
}

pub fn cell_format(s: &DataSheet, r: usize, c: usize) -> String {
    if r == 0 {
        return String::new();
    }
    let f = s.col_formats.get(c).cloned().unwrap_or_default();
    if f.is_empty() {
        let ty = s.col_types.get(c).map(String::as_str).unwrap_or("");
        if ty == "Date" {
            return "yyyy-mm-dd".into();
        }
        if ty == "Date/Time" {
            return "yyyy-mm-dd hh:mm".into();
        }
    }
    f
}

// =============================================================================================
// Edits (commit, clear, …) shared by the grid and the formula bar
// =============================================================================================

pub(crate) fn commit_edit(app: &mut VizApp, st: &mut SheetUi) {
    let Some(e) = st.edit.take() else { return };
    if e.text == e.original {
        return;
    }
    let mut text = e.text.clone();
    // Close unbalanced parentheses like Excel.
    if text.starts_with('=') {
        let open = text.matches('(').count();
        let close = text.matches(')').count();
        if open > close {
            text.push_str(&")".repeat(open - close));
        }
    }
    apply(app, st, "Edit cell", |s| ops::set(s, e.r, e.c, &text));
}

/// Runs a change on the active sheet with an undo step and marks the model dirty.
pub(crate) fn apply(app: &mut VizApp, st: &mut SheetUi, label: &str, f: impl FnOnce(&mut DataSheet)) {
    if app.project.sheets.get(st.active).is_none() {
        return;
    }
    app.checkpoint(label);
    if let Some(s) = app.project.sheets.get_mut(st.active) {
        f(s);
        ops::trim(s);
        if s.cells.is_empty() {
            s.cells.push(vec![]);
        }
    }
    let load = app.project.sheets.get(st.active).is_some_and(|s| s.load);
    // Other sheets may refer to this one; any loaded sheet needs a rebuild.
    if load || app.project.sheets.iter().any(|s| s.load) {
        app.mark_model_dirty();
    }
}

pub(crate) fn clear_selection(app: &mut VizApp, st: &mut SheetUi) {
    let r = st.selection();
    apply(app, st, "Clear cells", |s| {
        for row in r.r1..=r.r2 {
            for c in r.c1..=r.c2 {
                ops::set(s, row, c, "");
            }
        }
    });
}

pub(crate) fn copy(app: &mut VizApp, st: &mut SheetUi, ctx: &egui::Context, cut: bool) {
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let r = st.selection();
    let grid = st.cache.as_ref().map(|c| c.values.as_slice()).unwrap_or(&[]);
    let tsv = ops::copy_values(grid, s, r);
    let raw = ops::copy_raw(s, r);
    ctx.copy_text(tsv.clone());
    st.clip = Some((tsv, raw, (r.r1, r.c1)));
    if cut {
        clear_selection(app, st);
        st.clip = None;
        app.toast(format!("Cut {}", r.label()));
    }
}

pub(crate) fn paste_text(app: &mut VizApp, st: &mut SheetUi, text: &str) {
    let (block, origin) = match &st.clip {
        Some((tsv, raw, origin)) if normalize_nl(tsv) == normalize_nl(text) => (raw.clone(), Some(*origin)),
        _ => (ops::parse_tsv(text), None),
    };
    if block.is_empty() {
        return;
    }
    let sel = st.selection();
    let at = (sel.r1, sel.c1);
    let mut pasted = sel;
    apply(app, st, "Paste", |s| pasted = ops::paste(s, &block, at, sel, origin));
    st.anchor = (pasted.r1, pasted.c1);
    st.cursor = (pasted.r2, pasted.c2);
    app.toast(format!("Pasted {} × {}", pasted.rows(), pasted.cols()));
}

fn normalize_nl(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end_matches('\n').to_string()
}

pub(crate) fn sort_by(app: &mut VizApp, st: &mut SheetUi, col: usize, desc: bool) {
    let grid = st.cache.as_ref().map(|c| c.values.clone()).unwrap_or_default();
    apply(app, st, if desc { "Sort descending" } else { "Sort ascending" }, |s| ops::sort_rows(s, &grid, col, desc));
}

pub(crate) fn set_col_format(app: &mut VizApp, st: &mut SheetUi, fmt: &str) {
    let r = st.selection();
    apply(app, st, "Number format", |s| {
        for c in r.c1..=r.c2 {
            if s.col_formats.len() <= c {
                s.col_formats.resize(c + 1, String::new());
            }
            if let Some(f) = s.col_formats.get_mut(c) {
                *f = fmt.to_string();
            }
        }
    });
}

pub(crate) fn set_col_type(app: &mut VizApp, st: &mut SheetUi, ty: &str) {
    let r = st.selection();
    apply(app, st, "Column data type", |s| {
        for c in r.c1..=r.c2 {
            if s.col_types.len() <= c {
                s.col_types.resize(c + 1, String::new());
            }
            if let Some(f) = s.col_types.get_mut(c) {
                *f = ty.to_string();
            }
            if (ty == "Date" || ty == "Date/Time") && s.col_formats.get(c).is_none_or(|f| f.is_empty()) {
                if s.col_formats.len() <= c {
                    s.col_formats.resize(c + 1, String::new());
                }
                if let Some(f) = s.col_formats.get_mut(c) {
                    *f = if ty == "Date" { "yyyy-mm-dd".into() } else { "yyyy-mm-dd hh:mm".into() };
                }
            }
        }
    });
}

pub(crate) fn import_csv(app: &mut VizApp, st: &mut SheetUi) {
    let Some(path) = rfd::FileDialog::new().add_filter("CSV / text", &["csv", "tsv", "txt"]).pick_file() else { return };
    match std::fs::read(&path) {
        Ok(bytes) => {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Imported".into());
            let name = app.project.free_table_name(&stem);
            let sh = ops::sheet_from_csv(&name, &String::from_utf8_lossy(&bytes));
            let rows = sh.cells.len().saturating_sub(1);
            app.checkpoint("Import CSV");
            app.project.sheets.push(sh);
            st.active = app.project.sheets.len() - 1;
            st.select(1, 0, false);
            app.mark_model_dirty();
            app.toast(format!("Imported {rows} rows into “{name}”."));
        }
        Err(e) => app.error(format!("Couldn't read {}: {e}", path.display())),
    }
}

pub(crate) fn export_csv(app: &mut VizApp, st: &SheetUi) {
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name(format!("{}.csv", s.name)).save_file() else { return };
    let t = sheet::evaluate_with(s, &app.project.sheets);
    match std::fs::write(&path, vizcraft_core::flow::write_csv(&t, ',')) {
        Ok(()) => app.toast(format!("Exported {} rows to {}", t.rows.len(), path.display())),
        Err(e) => app.error(format!("Couldn't write {}: {e}", path.display())),
    }
}

// =============================================================================================
// Chrome: empty state, toolbar, formula bar, tabs
// =============================================================================================

fn empty_state(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui, t: &Tokens) {
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.28);
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), r, "sheet", t.accent);
            ui.add_space(8.0);
            ui.label(RichText::new("Enter and edit data like in Excel").size(18.0).strong().color(t.text));
            ui.label(RichText::new("Data sheets hold typed or pasted data, with formulas. Turn on “Load to model” to use a sheet in reports.").color(t.muted));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 430.0).max(0.0) / 2.0);
                if ui.add(egui::Button::new(RichText::new("New sheet").color(Color32::WHITE)).fill(t.accent)).clicked() {
                    new_sheet(app);
                }
                if ui.button("New sheet from query…").clicked() {
                    st.from_query = Some(String::new());
                }
                if ui.button("Import CSV…").clicked() {
                    import_csv(app, st);
                }
            });
        });
    });
}

fn tool_button(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, tip: &str) -> egui::Response {
    let galley_w = crate::widgets::text_w(ui, label);
    let w = galley_w + if icon.is_empty() { 14.0 } else { 30.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.accent_soft);
    }
    let mut x = rect.left() + 7.0;
    if !icon.is_empty() {
        crate::icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(x - 1.0, rect.center().y - 8.0), vec2(16.0, 16.0)), icon, t.accent);
        x += 18.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.0), t.text);
    resp.on_hover_text(tip)
}

fn toggle_button(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, tip: &str, active: bool) -> egui::Response {
    let galley_w = crate::widgets::text_w(ui, label);
    let w = galley_w + if icon.is_empty() { 14.0 } else { 30.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), egui::Sense::click());
    if active {
        ui.painter().rect(rect, 4.0, t.accent_soft, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.accent_soft);
    }
    let mut x = rect.left() + 7.0;
    if !icon.is_empty() {
        crate::icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(x - 1.0, rect.center().y - 8.0), vec2(16.0, 16.0)), icon, t.accent);
        x += 18.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.0), if active { t.accent } else { t.text });
    resp.on_hover_text(tip)
}

fn glyph_button(ui: &mut Ui, t: &Tokens, glyph: &str, tip: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), egui::Sense::click());
    if active {
        ui.painter().rect(rect, 4.0, t.accent_soft, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.accent_soft);
    }
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(13.0), if active { t.accent } else { t.text });
    resp.on_hover_text(tip)
}

fn vsep(ui: &mut Ui, t: &Tokens) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 22.0), egui::Sense::hover());
    ui.painter().line_segment([r.center_top(), r.center_bottom()], egui::Stroke::new(1.0, t.border));
}

fn toolbar(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui, t: &Tokens) {
    egui::Panel::top("sheet_toolbar").exact_size(38.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 5)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if tool_button(ui, t, "new", "New sheet", "A new empty data sheet").clicked() {
                new_sheet(app);
                st.active = app.project.sheets.len().saturating_sub(1);
                st.cache = None;
            }
            if tool_button(ui, t, "transform", "From query…", "An editable copy of a Power Query query's output").clicked() {
                st.from_query = Some(String::new());
            }
            if tool_button(ui, t, "open", "Import CSV", "Import a CSV file into a new sheet").clicked() {
                import_csv(app, st);
            }
            if tool_button(ui, t, "export", "Export CSV", "Save this sheet's values as CSV").clicked() {
                export_csv(app, st);
            }
            vsep(ui, t);
            let sel = st.selection();
            if tool_button(ui, t, "", "Sort A-Z", "Sort ascending by the current column").clicked() {
                sort_by(app, st, sel.c1, false);
            }
            if tool_button(ui, t, "", "Sort Z-A", "Sort descending by the current column").clicked() {
                sort_by(app, st, sel.c1, true);
            }
            let fb = st.filter_buttons;
            if toggle_button(ui, t, "filter", "Filter", "Filter buttons on the header row", fb).clicked() {
                st.filter_buttons = !fb;
            }
            let has_filters = app.project.sheets.get(st.active).and_then(|s| st.filters.get(&s.name)).is_some_and(|f| f.values().any(|v| !v.is_empty()));
            if has_filters && tool_button(ui, t, "", "Clear filters", "Show every row").clicked()
                && let Some(s) = app.project.sheets.get(st.active)
            {
                st.filters.remove(&s.name);
            }
            if toggle_button(ui, t, "", "Find & replace", "Find and replace (Ctrl+F / Ctrl+H)", st.find.open).clicked() {
                st.find.open = !st.find.open;
            }
            vsep(ui, t);
            if tool_button(ui, t, "", "+ Row", "Insert rows above the selection").clicked() {
                let n = sel.rows();
                apply(app, st, "Insert rows", |s| ops::insert_rows(s, sel.r1.max(1), n));
            }
            if tool_button(ui, t, "", "+ Column", "Insert columns left of the selection").clicked() {
                let n = sel.cols();
                apply(app, st, "Insert columns", |s| ops::insert_cols(s, sel.c1, n));
            }
            if tool_button(ui, t, "", "Fill down", "Copy the top cells down the selection (Ctrl+D)").clicked() {
                apply(app, st, "Fill down", |s| ops::fill_down(s, sel));
            }
            vsep(ui, t);
            // Column number format and data type.
            let Some(s) = app.project.sheets.get(st.active) else { return };
            let cur_fmt = s.col_formats.get(sel.c1).cloned().unwrap_or_default();
            let cur_ty = s.col_types.get(sel.c1).cloned().unwrap_or_default();
            ui.label(RichText::new("Format").size(11.5).color(t.muted));
            let mut new_fmt = None;
            egui::ComboBox::from_id_salt("sheet_fmt").width(130.0).selected_text(FORMATS.iter().find(|f| f.0 == cur_fmt).map(|f| f.1.to_string()).unwrap_or(cur_fmt.clone())).show_ui(ui, |ui| {
                for (f, label) in FORMATS {
                    let sample = if f.is_empty() { String::new() } else if f.contains('y') { vizcraft_core::numfmt::format_date(45322.0, f) } else { vizcraft_core::numfmt::format(1234.5, f) };
                    if ui.selectable_label(cur_fmt == *f, format!("{label}   {sample}")).clicked() {
                        new_fmt = Some(f.to_string());
                    }
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Type").size(11.5).color(t.muted));
            let mut new_ty = None;
            egui::ComboBox::from_id_salt("sheet_type").width(110.0).selected_text(TYPES.iter().find(|f| f.0 == cur_ty).map(|f| f.1.to_string()).unwrap_or(cur_ty.clone())).show_ui(ui, |ui| {
                for (ty, label) in TYPES {
                    if ui.selectable_label(cur_ty == *ty, *label).clicked() {
                        new_ty = Some(ty.to_string());
                    }
                }
            });
            if let Some(f) = new_fmt {
                set_col_format(app, st, &f);
            }
            if let Some(ty) = new_ty {
                set_col_type(app, st, &ty);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let Some(s) = app.project.sheets.get(st.active) else { return };
                let from_query = s.from_query.clone();
                let mut load = s.load;
                let r = ui.add(egui::Checkbox::new(&mut load, RichText::new("Load to model").size(12.0))).on_hover_text("Use this sheet as a table of the semantic model");
                if r.changed() {
                    app.checkpoint(if load { "Load sheet to model" } else { "Don't load sheet" });
                    if let Some(s) = app.project.sheets.get_mut(st.active) {
                        s.load = load;
                    }
                    app.mark_model_dirty();
                }
                let mut fz = st.freeze_header;
                ui.add(egui::Checkbox::new(&mut fz, RichText::new("Freeze header").size(12.0))).on_hover_text("Keep the header row visible while scrolling");
                st.freeze_header = fz;
                if let Some(q) = &from_query {
                    ui.label(RichText::new(format!("copy of {q}")).size(11.0).color(t.muted));
                }
            });
        });
    });
}

fn formula_bar(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui, t: &Tokens) {
    egui::Panel::top("sheet_fx").exact_size(34.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 4)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            // Name box.
            let label = st.selection().label();
            let mut nb = st.name_box.clone().unwrap_or(label.clone());
            let r = ui.add(egui::TextEdit::singleline(&mut nb).desired_width(90.0).font(egui::FontId::monospace(12.5)).align(egui::Align2::CENTER_CENTER)).on_hover_text("Name box: type an address (B12) or range (A1:C9) to go there");
            if r.gained_focus() || r.has_focus() {
                st.name_box = Some(nb.clone());
                st.grid_focus = false;
            }
            if r.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let txt = nb.trim().to_ascii_uppercase();
                    let parts: Vec<&str> = txt.split(':').collect();
                    let a = parts.first().and_then(|p| sheet::parse_address(p));
                    let b = parts.get(1).and_then(|p| sheet::parse_address(p));
                    if let Some(a) = a {
                        st.anchor = a;
                        st.cursor = b.unwrap_or(a);
                        st.scroll_to = Some(st.cursor);
                    }
                }
                st.name_box = None;
                st.grid_focus = true;
            }
            let (fx, _) = ui.allocate_exact_size(vec2(28.0, 24.0), egui::Sense::hover());
            ui.painter().line_segment([fx.left_top() + vec2(0.0, 3.0), fx.left_bottom() - vec2(0.0, 3.0)], egui::Stroke::new(1.0, t.border));
            ui.painter().text(fx.center(), egui::Align2::CENTER_CENTER, "ƒx", egui::FontId::proportional(13.0), t.muted);
            // Raw input of the cursor cell (editable).
            let (r0, c0) = st.cursor;
            let raw = app.project.sheets.get(st.active).map(|s| ops::get(s, r0, c0).to_string()).unwrap_or_default();
            let bar_id = ui.scope_id().with("fx_edit");
            let editing_here = st.edit.as_ref().is_some_and(|e| e.r == r0 && e.c == c0);
            let mut text = st.edit.as_ref().filter(|_| editing_here).map(|e| e.text.clone()).unwrap_or(raw.clone());
            let te = egui::TextEdit::singleline(&mut text).id(bar_id).desired_width(ui.available_width() - 4.0).font(egui::FontId::monospace(12.5)).hint_text("Type a value or a formula such as =SUM(B2:B10)");
            if st.edit.as_ref().is_some_and(|e| e.focus_bar) {
                ui.memory_mut(|m| m.request_focus(bar_id));
                if let Some(e) = st.edit.as_mut() {
                    e.focus_bar = false;
                }
            }
            let resp = ui.add(te);
            if resp.gained_focus() && st.edit.is_none() {
                st.edit = Some(Edit { r: r0, c: c0, text: raw.clone(), original: raw.clone(), typed: false, focus_cell: false, focus_bar: false });
                st.grid_focus = false;
            }
            if resp.changed()
                && let Some(e) = st.edit.as_mut()
            {
                e.text = text.clone();
            }
            if resp.lost_focus() && st.edit.is_some() && !st.edit.as_ref().is_some_and(|e| e.focus_cell) {
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    st.edit = None;
                } else if ui.ctx().memory(|m| m.focused().is_none()) || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit_edit(app, st);
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let n = grid::next_row(st, st.cursor.0, 1);
                        st.select(n, st.cursor.1, false);
                    }
                }
                st.grid_focus = true;
            }
            // Function hints while typing a formula.
            if let Some(e) = &st.edit
                && e.text.starts_with('=')
            {
                let word: String = e.text.chars().rev().take_while(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect();
                if word.len() >= 2 {
                    let up = word.to_ascii_uppercase();
                    let hits: Vec<&str> = sheet::FUNCTIONS.iter().copied().filter(|f| f.starts_with(&up)).take(8).collect();
                    if !hits.is_empty() {
                        let pos = resp.rect.left_bottom() + vec2(0.0, 2.0);
                        egui::Area::new(ui.scope_id().with("fx_hints")).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for f in hits {
                                        ui.label(RichText::new(format!("{f}(")).monospace().size(11.5).color(t.accent));
                                    }
                                });
                            });
                        });
                    }
                }
            }
        });
    });
}

fn tabs_bar(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui, t: &Tokens) {
    egui::Panel::bottom("sheet_tabs").exact_size(32.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(6, 3)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if glyph_button(ui, t, "+", "New sheet", false).clicked() {
                new_sheet(app);
                st.active = app.project.sheets.len().saturating_sub(1);
            }
            ui.add_space(4.0);
            let mut action: Option<(&str, usize)> = None;
            for i in 0..app.project.sheets.len() {
                let Some(s) = app.project.sheets.get(i) else { continue };
                let active = i == st.active;
                if let Some((ri, name)) = st.renaming.as_mut()
                    && *ri == i
                {
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(120.0).font(egui::FontId::proportional(12.0)));
                    r.request_focus();
                    if r.lost_focus() {
                        action = Some(("rename_done", i));
                    }
                    continue;
                }
                let label = s.name.clone();
                let w = crate::widgets::text_w(ui, &label) + if s.load { 34.0 } else { 22.0 };
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), egui::Sense::click());
                let fill = if active { t.panel } else if resp.hovered() { t.accent_soft } else { t.panel_alt };
                ui.painter().rect(rect, egui::CornerRadius { nw: 0, ne: 0, sw: 5, se: 5 }, fill, egui::Stroke::new(1.0, if active { t.border } else { Color32::TRANSPARENT }), egui::StrokeKind::Inside);
                if active {
                    ui.painter().line_segment([rect.left_bottom() + vec2(4.0, -1.0), rect.right_bottom() + vec2(-4.0, -1.0)], egui::Stroke::new(2.5, t.good));
                }
                ui.painter().text(rect.left_center() + vec2(10.0, 0.0), egui::Align2::LEFT_CENTER, &label, egui::FontId::proportional(12.0), if active { t.text } else { t.muted });
                if s.load {
                    ui.painter().circle_filled(rect.right_center() - vec2(11.0, 0.0), 3.0, t.accent);
                }
                let resp = resp.on_hover_text(if s.load { format!("{label}: loaded to the model as a table") } else { format!("{label}: not loaded to the model") });
                if resp.clicked() {
                    action = Some(("select", i));
                }
                if resp.double_clicked() {
                    action = Some(("rename", i));
                }
                resp.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        action = Some(("rename", i));
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        action = Some(("duplicate", i));
                        ui.close();
                    }
                    let mut l = s.load;
                    if ui.checkbox(&mut l, "Load to model").clicked() {
                        action = Some(("toggle_load", i));
                        ui.close();
                    }
                    if ui.button("Export CSV…").clicked() {
                        action = Some(("export", i));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(RichText::new("Delete sheet").color(t.danger)).clicked() {
                        action = Some(("delete", i));
                        ui.close();
                    }
                });
            }
            match action {
                Some(("select", i)) => {
                    if st.active != i {
                        commit_edit(app, st);
                        st.active = i;
                        st.select(1, 0, false);
                        st.cache = None;
                    }
                }
                Some(("rename", i)) => st.renaming = app.project.sheets.get(i).map(|s| (i, s.name.clone())),
                Some(("rename_done", i)) => {
                    if let Some((_, new)) = st.renaming.take() {
                        let new = new.trim().to_string();
                        let old = app.project.sheets.get(i).map(|s| s.name.clone()).unwrap_or_default();
                        if !new.is_empty() && new != old {
                            if app.project.free_table_name(&new) != new && !new.eq_ignore_ascii_case(&old) {
                                app.error(format!("The name “{new}” is already used by a table, query or sheet."));
                            } else {
                                rename_sheet(app, i, &old, &new);
                            }
                        }
                    }
                }
                Some(("duplicate", i)) => {
                    if let Some(s) = app.project.sheets.get(i).cloned() {
                        let name = app.project.free_table_name(&format!("{} copy", s.name));
                        app.checkpoint("Duplicate sheet");
                        app.project.sheets.insert(i + 1, DataSheet { name, load: false, ..s });
                        st.active = i + 1;
                    }
                }
                Some(("toggle_load", i)) => {
                    app.checkpoint("Load to model");
                    if let Some(s) = app.project.sheets.get_mut(i) {
                        s.load = !s.load;
                    }
                    app.mark_model_dirty();
                }
                Some(("export", i)) => {
                    let prev = st.active;
                    st.active = i;
                    export_csv(app, st);
                    st.active = prev;
                }
                Some(("delete", i)) => {
                    let name = app.project.sheets.get(i).map(|s| s.name.clone()).unwrap_or_default();
                    app.checkpoint("Delete sheet");
                    app.project.sheets.remove(i);
                    st.active = st.active.min(app.project.sheets.len().saturating_sub(1));
                    st.cache = None;
                    app.mark_model_dirty();
                    app.toast(format!("Deleted “{name}” (Ctrl+Z to undo)."));
                }
                _ => {}
            }
            // Selection statistics (like Excel's status bar).
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let Some(c) = &st.cache else { return };
                let r = st.selection();
                let mut count = 0usize;
                let mut nums = 0usize;
                let mut sum = 0.0;
                let mut rows_done = 0;
                for row in r.r1..=r.r2.min(c.values.len().saturating_sub(1)) {
                    rows_done += 1;
                    if rows_done > 200_000 {
                        break;
                    }
                    for col in r.c1..=r.c2 {
                        match c.values.get(row).and_then(|x| x.get(col)) {
                            Some(CellValue::Num(n)) if row > 0 => {
                                nums += 1;
                                count += 1;
                                sum += n;
                            }
                            Some(CellValue::Empty) | None => {}
                            Some(_) => count += 1,
                        }
                    }
                }
                let rows_total = app.project.sheets.get(st.active).map(|s| s.cells.len().saturating_sub(1)).unwrap_or(0);
                let shown = c.visible.len();
                let txt = if count > 1 && nums > 0 {
                    format!("Average {}    Count {}    Sum {}", vizcraft_core::numfmt::format(sum / nums as f64, "#,0.##"), count, vizcraft_core::numfmt::format(sum, "#,0.##"))
                } else if count > 1 {
                    format!("Count {count}")
                } else {
                    String::new()
                };
                ui.label(RichText::new(txt).size(11.5).color(t.text));
                ui.add_space(14.0);
                let rows_txt = if shown < rows_total { format!("{shown} of {rows_total} rows (filtered)") } else { format!("{rows_total} rows") };
                ui.label(RichText::new(rows_txt).size(11.5).color(t.muted));
            });
        });
    });
}

fn rename_sheet(app: &mut VizApp, i: usize, old: &str, new: &str) {
    app.checkpoint("Rename sheet");
    if let Some(s) = app.project.sheets.get_mut(i) {
        s.name = new.to_string();
    }
    // Keep the model table (and its metadata) and other sheets' references.
    for t in app.project.model.tables.iter_mut() {
        if t.source == vizcraft_core::doc::TableSource::Sheet(old.to_string()) {
            t.source = vizcraft_core::doc::TableSource::Sheet(new.to_string());
            if t.name == old {
                t.name = new.to_string();
            }
        }
    }
    let quoted_old = if old.chars().all(|c| c.is_alphanumeric() || c == '_') { format!("{old}!") } else { format!("'{}'!", old.replace('\'', "''")) };
    let quoted_new = if new.chars().all(|c| c.is_alphanumeric() || c == '_') { format!("{new}!") } else { format!("'{}'!", new.replace('\'', "''")) };
    for s in app.project.sheets.iter_mut() {
        for row in s.cells.iter_mut() {
            for c in row.iter_mut() {
                if c.starts_with('=') && c.contains(&quoted_old) {
                    *c = c.replace(&quoted_old, &quoted_new);
                }
            }
        }
    }
    app.mark_model_dirty();
}

// =============================================================================================
// Dialogs
// =============================================================================================

fn find_dialog(app: &mut VizApp, st: &mut SheetUi, ctx: &egui::Context) {
    if !st.find.open {
        return;
    }
    let mut open = true;
    let mut action = None;
    egui::Window::new(if st.find.replace_mode { "Find and replace" } else { "Find" }).open(&mut open).collapsible(false).resizable(false).default_pos(ctx.content_rect().right_top() + vec2(-380.0, 170.0)).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut st.find.replace_mode, false, "Find");
            ui.selectable_value(&mut st.find.replace_mode, true, "Replace");
        });
        ui.add_space(4.0);
        egui::Grid::new("find_grid").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
            ui.label("Find what");
            let r = ui.add(egui::TextEdit::singleline(&mut st.find.find).desired_width(220.0));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                action = Some("next");
            }
            ui.end_row();
            if st.find.replace_mode {
                ui.label("Replace with");
                ui.add(egui::TextEdit::singleline(&mut st.find.replace).desired_width(220.0));
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut st.find.match_case, "Match case");
            ui.checkbox(&mut st.find.whole, "Entire cell");
            ui.checkbox(&mut st.find.formulas, "Look in formulas");
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Find next").clicked() {
                action = Some("next");
            }
            if st.find.replace_mode {
                if ui.button("Replace").clicked() {
                    action = Some("replace");
                }
                if ui.button("Replace all").clicked() {
                    action = Some("all");
                }
            }
        });
        if !st.find.message.is_empty() {
            ui.label(RichText::new(&st.find.message).size(11.5).color(crate::theme::tokens(ui.ctx()).muted));
        }
    });
    if !open {
        st.find.open = false;
    }
    let Some(a) = action else { return };
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let grid = st.cache.as_ref().map(|c| c.values.clone()).unwrap_or_default();
    let f = st.find.clone();
    if f.find.is_empty() {
        st.find.message = "Type something to find.".into();
        return;
    }
    let norm = |x: &str| if f.match_case { x.to_string() } else { x.to_lowercase() };
    let needle = norm(&f.find);
    let text_of = |s: &DataSheet, r: usize, c: usize| -> String {
        let raw = ops::get(s, r, c);
        if f.formulas || !raw.starts_with('=') { raw.to_string() } else { display_value(s, &grid, r, c) }
    };
    let is_hit = |txt: &str| if f.whole { norm(txt) == needle } else { norm(txt).contains(&needle) };
    let w = ops::width(s);
    match a {
        "next" | "replace" => {
            if a == "replace" {
                let (r, c) = st.cursor;
                let raw = ops::get(s, r, c).to_string();
                if is_hit(&text_of(s, r, c)) && (!raw.starts_with('=') || f.formulas) {
                    let new = replace_in(&raw, &f);
                    apply(app, st, "Replace", |s| ops::set(s, r, c, &new));
                }
            }
            let Some(s) = app.project.sheets.get(st.active) else { return };
            let total = s.cells.len() * w.max(1);
            let start = st.cursor.0 * w.max(1) + st.cursor.1 + 1;
            for k in 0..total {
                let idx = (start + k) % total.max(1);
                let (r, c) = (idx / w.max(1), idx % w.max(1));
                if is_hit(&text_of(s, r, c)) {
                    st.select(r, c, false);
                    st.find.message = format!("Found in {}", sheet::address(r, c));
                    return;
                }
            }
            st.find.message = "No matches.".into();
        }
        _ => {
            let mut hits = Vec::new();
            for r in 0..s.cells.len() {
                for c in 0..w {
                    let raw = ops::get(s, r, c);
                    if raw.starts_with('=') && !f.formulas {
                        continue;
                    }
                    if is_hit(raw) {
                        hits.push((r, c, replace_in(raw, &f)));
                    }
                }
            }
            let n = hits.len();
            if n > 0 {
                apply(app, st, "Replace all", |s| {
                    for (r, c, v) in &hits {
                        ops::set(s, *r, *c, v);
                    }
                });
            }
            st.find.message = format!("Replaced {n} cell{}.", if n == 1 { "" } else { "s" });
        }
    }
}

fn replace_in(raw: &str, f: &FindState) -> String {
    if f.whole {
        return f.replace.clone();
    }
    if f.match_case {
        return raw.replace(&f.find, &f.replace);
    }
    // Case-insensitive replace.
    let lower = raw.to_lowercase();
    let needle = f.find.to_lowercase();
    let mut out = String::new();
    let mut i = 0;
    while let Some(p) = lower.get(i..).and_then(|s| s.find(&needle)) {
        out.push_str(raw.get(i..i + p).unwrap_or(""));
        out.push_str(&f.replace);
        i += p + needle.len();
    }
    out.push_str(raw.get(i..).unwrap_or(""));
    out
}

fn from_query_dialog(app: &mut VizApp, st: &mut SheetUi, ctx: &egui::Context) {
    let Some(sel) = st.from_query.clone() else { return };
    let mut open = true;
    let mut chosen = sel;
    let mut go = false;
    egui::Window::new("New sheet from query").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -60.0)).show(ctx, |ui| {
        let t = crate::theme::tokens(ui.ctx());
        ui.label(RichText::new("Copies a query's current output into a sheet you can edit freely.").color(t.muted));
        ui.add_space(6.0);
        if app.project.queries.queries.is_empty() {
            ui.label("There are no queries yet: get data first (Data → Get data).");
        }
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for q in &app.project.queries.queries {
                let r = ui.selectable_label(chosen == q.name, RichText::new(&q.name).size(13.0));
                if r.clicked() {
                    chosen = q.name.clone();
                }
                if r.double_clicked() {
                    chosen = q.name.clone();
                    go = true;
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!chosen.is_empty(), egui::Button::new("Create sheet")).clicked() {
                go = true;
            }
            if ui.button("Cancel").clicked() {
                chosen.clear();
                st.from_query = None;
            }
        });
    });
    if st.from_query.is_none() {
        return;
    }
    if !open {
        st.from_query = None;
        return;
    }
    if go && !chosen.is_empty() {
        st.from_query = None;
        app.sheet_ui = std::mem::take(st);
        sheet_from_query(app, &chosen);
        *st = std::mem::take(&mut app.sheet_ui);
        st.cache = None;
    } else {
        st.from_query = Some(chosen);
    }
}
