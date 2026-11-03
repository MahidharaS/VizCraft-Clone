//! Table view (Power BI's Data view): a virtualized grid of the selected table's rows, the
//! Column / Measure / Table tools strip, a formula bar for calculated columns, measures and
//! calculated tables, and the Data pane.

use std::collections::{BTreeMap, HashMap};

use egui::{Color32, RichText, Ui};
use vizcraft_core::build::ColType;
use vizcraft_core::doc::*;
use vizcraft_datamodel::Value;

use super::editor::{self, Lang};
use super::{Act, PaneMode, Sel, column_props, col_ref, current_table, data_pane, fmt_count, run_act, tables_meta};
use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets::big_button;

thread_local! {
    /// The tools are drawn as a vertical Properties pane (Model view) instead of a strip.
    static VERTICAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn group(ui: &mut Ui, t: &Tokens, caption: &str, add: impl FnOnce(&mut Ui)) {
    if VERTICAL.with(|v| v.get()) {
        ui.add_space(6.0);
        ui.label(RichText::new(caption.to_uppercase()).size(10.5).strong().color(t.muted));
        ui.add_space(2.0);
        ui.vertical(add);
        ui.add_space(4.0);
        ui.separator();
    } else {
        super::strip_group(ui, t, caption, add);
    }
}

pub fn load_buffers_pub(app: &mut VizApp) {
    load_buffers(app);
}

/// The selection's properties, as a vertical pane (Model view).
pub fn properties_pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    VERTICAL.with(|v| v.set(true));
    let sel = app.model_ui.sel.clone();
    match &sel {
        Sel::Column(tn, c) => {
            ui.label(RichText::new(format!("{tn}[{c}]")).strong().size(13.0));
            column_tools(app, ui, t, tn, c);
        }
        Sel::Measure(m) if app.project.model.measures.iter().any(|x| &x.name == m) => {
            ui.label(RichText::new(format!("[{m}]")).strong().size(13.0));
            if let Some(e) = app.project.model.measures.iter().find(|x| &x.name == m).map(|x| x.expression.clone()) {
                let ms: Vec<String> = app.built.model.measures.iter().map(|x| x.name.clone()).collect();
                egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                    ui.add(egui::Label::new(editor::highlight(&e, Lang::Dax, t.dark, &ms, 11.5)).wrap());
                });
                if ui.small_button("Edit formula…").clicked() {
                    super::dialogs::edit_measure(app, m);
                }
            }
            measure_tools(app, ui, t, m);
        }
        Sel::Measure(m) => {
            ui.label(RichText::new(format!("[{m}]")).strong());
            ui.label(RichText::new("Created by a what-if parameter: it returns the value selected in the parameter's slicer.").color(t.muted));
        }
        Sel::Table(tn) => {
            ui.label(RichText::new(tn).strong().size(13.0));
            table_tools(app, ui, t, tn);
        }
        _ => {}
    }
    VERTICAL.with(|v| v.set(false));
}

#[derive(Default)]
pub struct TableViewState {
    sort: Option<(String, bool)>,
    filters: BTreeMap<String, String>,
    cache_key: String,
    order: Vec<u32>,
    menu: Option<(String, egui::Pos2)>,
    menu_filter: String,
    menu_fresh: bool,
    /// Buffers of the tools strip text fields and the formula bar, for `loaded_for`.
    loaded_for: Option<Sel>,
    name: String,
    folder: String,
    description: String,
    custom_format: String,
    formula: String,
    formula_error: Option<String>,
    widths: HashMap<(String, String), f32>,
}

pub const NUMBER_FORMATS: &[(&str, &str)] = &[
    ("General", ""),
    ("Whole number", "#,0"),
    ("Decimal (2)", "#,0.00"),
    ("Currency", "$#,0.00"),
    ("Currency (whole)", "$#,0"),
    ("Percentage", "0%"),
    ("Percentage (1 dp)", "0.0%"),
    ("Scientific", "0.00E+00"),
];
pub const DATE_FORMATS: &[(&str, &str)] = &[
    ("2024-03-14", "yyyy-mm-dd"),
    ("14/03/2024", "dd/mm/yyyy"),
    ("03/14/2024", "mm/dd/yyyy"),
    ("14 Mar 2024", "d mmm yyyy"),
    ("Mar 2024", "mmm yyyy"),
    ("March 14, 2024", "mmmm d, yyyy"),
    ("Thursday", "dddd"),
    ("2024-03-14 13:30", "yyyy-mm-dd hh:mm"),
];
pub const SUMMARIZE: &[&str] = &["Sum", "Average", "Min", "Max", "Count", "Distinct count", "None"];
pub const CATEGORIES: &[&str] =
    &["Uncategorized", "Address", "City", "Continent", "Country", "County", "Latitude", "Longitude", "Place", "Postal code", "State or Province", "Web URL", "Image URL", "Barcode"];
pub const DATA_TYPES: &[&str] = &["Text", "Whole Number", "Decimal Number", "Fixed Decimal", "Date", "Date/Time", "True/False"];

/// A cell's text with the column's format.
pub fn cell_text(v: &Value, ty: ColType, fmt: &str) -> String {
    match v {
        Value::Blank => String::new(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Text(s) => s.to_string(),
        Value::Number(n) => {
            if ty.is_temporal() {
                vizcraft_core::numfmt::format_date(*n, if fmt.is_empty() { ty.default_format() } else { fmt })
            } else if fmt.is_empty() {
                vizcraft_core::numfmt::general(*n)
            } else {
                vizcraft_core::numfmt::format(*n, fmt)
            }
        }
    }
}

fn effective_format(p: &Project, table: &str, column: &str, ty: ColType) -> String {
    column_props(p, table, column).map(|c| c.format.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| ty.default_format().to_string())
}

/// The selected table (from the selection, else the default one).
fn shown_table(app: &VizApp) -> Option<String> {
    match &app.model_ui.sel {
        Sel::Table(t) | Sel::Column(t, _) => Some(t.clone()),
        _ => current_table(app),
    }
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    load_buffers(app);
    egui::Panel::top("tv_tools").frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 2 }).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| tools(app, ui, &t));
    });
    if formula_target(app).is_some() {
        egui::Panel::top("tv_formula").frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(10, 6)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| formula_bar(app, ui, &t));
    }
    let mut acts = Vec::new();
    egui::Panel::right("tv_data").default_size(270.0).size_range(200.0..=460.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        acts = data_pane(app, ui, PaneMode::Table);
    });
    egui::Panel::bottom("tv_status").exact_size(26.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(10, 4))).show(ui, |ui| status(app, ui, &t));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel)).show(ui, |ui| grid(app, ui, &t));
    for a in acts {
        run_act(app, a);
    }
}

fn status(app: &VizApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        let Some(tn) = shown_table(app) else { return };
        let rows = app.built.model.table(&tn).map(|x| x.row_count()).unwrap_or(0);
        let shown = app.model_ui.table.order.len();
        let mut s = format!("Table: {tn} ({} rows)", fmt_count(rows));
        if shown != rows && !app.model_ui.table.filters.is_empty() {
            s += &format!(" · {} shown", fmt_count(shown));
        }
        if let Sel::Column(_, c) = &app.model_ui.sel
            && let Some(col) = app.built.model.table(&tn).and_then(|x| x.column_index(c).and_then(|i| x.column(i)))
        {
            s += &format!("    Column: {c} ({} distinct values)", fmt_count(col.cardinality()));
        }
        ui.label(RichText::new(s).size(11.0).color(t.muted));
    });
}

// ------------------------------------------------------------------------------------- tools

fn load_buffers(app: &mut VizApp) {
    let sel = app.model_ui.sel.clone();
    if app.model_ui.table.loaded_for.as_ref() == Some(&sel) {
        return;
    }
    let st = &mut app.model_ui.table;
    st.loaded_for = Some(sel.clone());
    st.formula_error = None;
    let p = &app.project;
    match &sel {
        Sel::Column(tn, c) => {
            let cp = column_props(p, tn, c);
            st.name = c.clone();
            st.folder = cp.map(|x| x.display_folder.clone()).unwrap_or_default();
            st.description = cp.map(|x| x.description.clone()).unwrap_or_default();
            st.custom_format = cp.map(|x| x.format.clone()).unwrap_or_default();
            st.formula = p.model.calculated_columns.iter().find(|x| &x.table == tn && &x.name == c).map(|x| x.expression.clone()).unwrap_or_default();
        }
        Sel::Measure(m) => {
            let ms = p.model.measures.iter().find(|x| &x.name == m);
            st.name = m.clone();
            st.folder = ms.map(|x| x.display_folder.clone()).unwrap_or_default();
            st.description = ms.map(|x| x.description.clone()).unwrap_or_default();
            st.custom_format = ms.map(|x| x.format.clone()).unwrap_or_default();
            st.formula = ms.map(|x| x.expression.clone()).unwrap_or_default();
        }
        Sel::Table(tn) => {
            let mt = p.model.tables.iter().find(|x| &x.name == tn);
            st.name = tn.clone();
            st.description = mt.map(|x| x.description.clone()).unwrap_or_default();
            st.formula = match mt.map(|x| &x.source) {
                Some(TableSource::Calculated(d)) => d.clone(),
                _ => String::new(),
            };
        }
        _ => {}
    }
}

/// What the formula bar edits: a calculated column, a measure or a calculated table.
fn formula_target(app: &VizApp) -> Option<Sel> {
    let p = &app.project;
    match &app.model_ui.sel {
        Sel::Column(tn, c) if p.model.calculated_columns.iter().any(|x| &x.table == tn && &x.name == c) => Some(app.model_ui.sel.clone()),
        Sel::Measure(m) if p.model.measures.iter().any(|x| &x.name == m) => Some(app.model_ui.sel.clone()),
        Sel::Table(tn) if p.model.tables.iter().any(|x| &x.name == tn && matches!(x.source, TableSource::Calculated(_))) => Some(app.model_ui.sel.clone()),
        _ => None,
    }
}

fn labeled(ui: &mut Ui, t: &Tokens, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([74.0, 20.0], egui::Label::new(RichText::new(label).size(11.5).color(t.muted)));
        add(ui);
    });
}

fn col_props_mut<'a>(p: &'a mut Project, table: &str, column: &str) -> Option<&'a mut ColumnProps> {
    let mt = p.model.tables.iter_mut().find(|x| x.name == table)?;
    if !mt.columns.iter().any(|c| c.name == column) {
        mt.columns.push(ColumnProps { name: column.to_string(), ..ColumnProps::default() });
    }
    mt.columns.iter_mut().find(|c| c.name == column)
}

fn format_picker(ui: &mut Ui, id: &str, current: &str, temporal: bool) -> Option<String> {
    let presets = if temporal { DATE_FORMATS } else { NUMBER_FORMATS };
    let label = presets.iter().find(|(_, f)| *f == current).map(|(l, _)| l.to_string()).unwrap_or_else(|| if current.is_empty() { "General".into() } else { "Custom".into() });
    let mut out = None;
    egui::ComboBox::from_id_salt(id).selected_text(label).width(150.0).show_ui(ui, |ui| {
        for (l, f) in presets {
            let sample = if temporal { vizcraft_core::numfmt::format_date(45365.5625, if f.is_empty() { "yyyy-mm-dd" } else { f }) } else { vizcraft_core::numfmt::format(-1234.5678, f) };
            if ui.selectable_label(*f == current, format!("{l}    {sample}")).clicked() {
                out = Some(f.to_string());
            }
        }
    });
    out
}

/// Number format buttons: currency, percent, thousands, decimals.
fn format_buttons(ui: &mut Ui, current: &str) -> Option<String> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if ui.small_button("$").on_hover_text("Currency").clicked() {
            out = Some(if current.starts_with('$') { current.trim_start_matches('$').to_string() } else { format!("${}", if current.is_empty() { "#,0.00" } else { current }) });
        }
        if ui.small_button("%").on_hover_text("Percentage").clicked() {
            out = Some(if current.ends_with('%') { current.trim_end_matches('%').to_string() } else { "0.0%".into() });
        }
        if ui.small_button(",").on_hover_text("Thousands separator").clicked() {
            out = Some(if current.contains(',') { current.replace("#,", "") } else { format!("#,{}", if current.is_empty() { "0" } else { current.trim_start_matches('$') }) });
        }
        let decimals = current.split_once('.').map(|(_, d)| d.chars().filter(|c| *c == '0' || *c == '#').count()).unwrap_or(0);
        if ui.small_button(".0←").on_hover_text("Fewer decimals").clicked() && decimals > 0 {
            let (a, b) = current.split_once('.').unwrap_or((current, ""));
            let rest: String = b.chars().skip(1).collect();
            out = Some(if decimals == 1 { format!("{a}{}", rest.trim_start_matches(['0', '#'])) } else { format!("{a}.{rest}") });
        }
        if ui.small_button("→.00").on_hover_text("More decimals").clicked() {
            out = Some(match current.split_once('.') {
                Some((a, b)) => format!("{a}.0{b}"),
                None => {
                    let base = if current.is_empty() { "#,0" } else { current };
                    let end = base.rfind(['0', '#']).map(|i| i + 1).unwrap_or(base.len());
                    format!("{}.0{}", &base[..end], &base[end..])
                }
            });
        }
    });
    out
}

fn tools(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let sel = app.model_ui.sel.clone();
    match &sel {
        Sel::Column(tn, c) => column_tools(app, ui, t, tn, c),
        Sel::Measure(m) if app.project.model.measures.iter().any(|x| &x.name == m) => measure_tools(app, ui, t, m),
        _ => {
            if let Some(tn) = shown_table(app) {
                table_tools(app, ui, t, &tn);
            }
        }
    }
    common_buttons(app, ui, t);
}

fn name_field(app: &mut VizApp, ui: &mut Ui, enabled: bool, tip: &str) -> bool {
    let r = ui.add_enabled(enabled, egui::TextEdit::singleline(&mut app.model_ui.table.name).desired_width(170.0));
    let r = if tip.is_empty() { r } else { r.on_disabled_hover_text(tip) };
    r.lost_focus() && app.model_ui.table.name.trim() != match &app.model_ui.sel {
        Sel::Column(_, c) => c.as_str(),
        Sel::Measure(m) => m.as_str(),
        Sel::Table(tn) => tn.as_str(),
        _ => "",
    }
}

fn column_tools(app: &mut VizApp, ui: &mut Ui, t: &Tokens, tn: &str, c: &str) {
    let ty = super::column_type(&app.project, &app.built, tn, c);
    let calc = app.project.model.calculated_columns.iter().any(|x| x.table == tn && x.name == c);
    let cp = column_props(&app.project, tn, c).cloned().unwrap_or_default();
    group(ui, t, "Structure", |ui| {
        ui.vertical(|ui| {
            labeled(ui, t, "Name", |ui| {
                if name_field(app, ui, calc, "Data columns are renamed in Power Query (Transform data).") {
                    rename_column(app, tn, c);
                }
            });
            labeled(ui, t, "Data type", |ui| {
                let current = if cp.data_type.is_empty() { ty.label().to_string() } else { cp.data_type.clone() };
                ui.add_enabled_ui(!calc, |ui| {
                    egui::ComboBox::from_id_salt("tv_dtype").selected_text(&current).width(150.0).show_ui(ui, |ui| {
                        for d in DATA_TYPES {
                            if ui.selectable_label(current == *d, *d).clicked() && current != *d {
                                app.checkpoint(&format!("Data type of {c}"));
                                if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                                    p.data_type = d.to_string();
                                }
                                app.mark_data_dirty();
                                app.toast(format!("{c} converted to {d}"));
                            }
                        }
                    });
                });
            });
            labeled(ui, t, "", |ui| {
                ui.label(RichText::new(if calc { "Calculated column" } else { "Data column" }).size(11.0).color(t.muted));
            });
        });
    });
    group(ui, t, "Formatting", |ui| {
        ui.vertical(|ui| {
            let current = if cp.format.is_empty() { ty.default_format().to_string() } else { cp.format.clone() };
            let mut new = None;
            labeled(ui, t, "Format", |ui| new = format_picker(ui, "tv_fmt", &current, ty.is_temporal()));
            labeled(ui, t, "", |ui| {
                if !ty.is_temporal()
                    && let Some(f) = format_buttons(ui, &current)
                {
                    new = Some(f);
                }
            });
            labeled(ui, t, "Custom", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.model_ui.table.custom_format).desired_width(150.0).hint_text("#,0.00"));
                if r.lost_focus() && app.model_ui.table.custom_format != current {
                    new = Some(app.model_ui.table.custom_format.clone());
                }
            });
            if let Some(f) = new {
                app.checkpoint(&format!("Format of {c}"));
                if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                    p.format = f.clone();
                }
                app.model_ui.table.custom_format = f;
                app.mark_model_dirty();
            }
        });
    });
    group(ui, t, "Properties", |ui| {
        ui.vertical(|ui| {
            let sum = if cp.summarize_by.is_empty() {
                if ty.is_numeric() && !c.to_ascii_lowercase().ends_with("id") { "Sum".to_string() } else { "None".to_string() }
            } else {
                cp.summarize_by.clone()
            };
            labeled(ui, t, "Summarization", |ui| {
                egui::ComboBox::from_id_salt("tv_sum").selected_text(if sum == "None" { "Don't summarize" } else { sum.as_str() }).width(150.0).show_ui(ui, |ui| {
                    for s in SUMMARIZE {
                        if ui.selectable_label(sum == *s, if *s == "None" { "Don't summarize" } else { s }).clicked() && sum != *s {
                            app.checkpoint(&format!("Summarization of {c}"));
                            if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                                p.summarize_by = s.to_string();
                            }
                            app.mark_model_dirty();
                        }
                    }
                });
            });
            let cat = if cp.data_category.is_empty() { "Uncategorized".to_string() } else { cp.data_category.clone() };
            labeled(ui, t, "Data category", |ui| {
                egui::ComboBox::from_id_salt("tv_cat").selected_text(&cat).width(150.0).show_ui(ui, |ui| {
                    for s in CATEGORIES {
                        if ui.selectable_label(cat == *s, *s).clicked() && cat != *s {
                            app.checkpoint(&format!("Data category of {c}"));
                            if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                                p.data_category = if *s == "Uncategorized" { String::new() } else { s.to_string() };
                            }
                            app.mark_model_dirty();
                        }
                    }
                });
            });
            labeled(ui, t, "Folder", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.model_ui.table.folder).desired_width(150.0).hint_text("Display folder"));
                if r.lost_focus() && app.model_ui.table.folder != cp.display_folder {
                    let f = app.model_ui.table.folder.clone();
                    app.checkpoint("Display folder");
                    if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                        p.display_folder = f;
                    }
                }
            });
        });
    });
    group(ui, t, "Sort", |ui| {
        ui.vertical(|ui| {
            let current = cp.sort_by.clone().unwrap_or_default();
            ui.label(RichText::new("Sort by column").size(11.5).color(t.muted));
            let cols: Vec<String> = tables_meta(&app.project, &app.built).into_iter().find(|m| m.name == tn).map(|m| m.columns.into_iter().map(|c| c.name).collect()).unwrap_or_default();
            egui::ComboBox::from_id_salt("tv_sortby").selected_text(if current.is_empty() { c.to_string() } else { current.clone() }).width(150.0).show_ui(ui, |ui| {
                for other in cols {
                    let is = if current.is_empty() { other == c } else { other == current };
                    if ui.selectable_label(is, &other).clicked() && !is {
                        match check_sort_by(app, tn, c, &other) {
                            Ok(()) => {
                                app.checkpoint(&format!("Sort {c} by {other}"));
                                if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                                    p.sort_by = if other == c { None } else { Some(other.clone()) };
                                }
                                app.mark_model_dirty();
                            }
                            Err(e) => app.error(e),
                        }
                    }
                }
            });
        });
    });
    group(ui, t, "Visibility", |ui| {
        ui.vertical(|ui| {
            let mut hidden = cp.hidden;
            if ui.checkbox(&mut hidden, "Hide in report view").changed() {
                app.checkpoint(&format!("Hide {c}"));
                if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                    p.hidden = hidden;
                }
                app.mark_model_dirty();
            }
            let r = ui.add(egui::TextEdit::multiline(&mut app.model_ui.table.description).desired_width(170.0).desired_rows(2).hint_text("Description"));
            if r.lost_focus() && app.model_ui.table.description != cp.description {
                let d = app.model_ui.table.description.clone();
                app.checkpoint("Description");
                if let Some(p) = col_props_mut(&mut app.project, tn, c) {
                    p.description = d;
                }
            }
        });
    });
}

/// Every value of `column` must map to one value of `by`.
fn check_sort_by(app: &VizApp, table: &str, column: &str, by: &str) -> Result<(), String> {
    if column == by {
        return Ok(());
    }
    let Some(tb) = app.built.model.table(table) else { return Ok(()) };
    let (Some(a), Some(b)) = (tb.column_index(column).and_then(|i| tb.column(i)), tb.column_index(by).and_then(|i| tb.column(i))) else { return Ok(()) };
    let mut map: HashMap<vizcraft_datamodel::value::Key, vizcraft_datamodel::value::Key> = HashMap::new();
    for r in 0..tb.row_count() {
        let k = a.value(r).key();
        let v = b.value(r).key();
        if let Some(prev) = map.get(&k) {
            if *prev != v {
                return Err(format!("Can't sort {column} by {by}: \"{}\" has more than one {by} value.", a.value(r).to_text()));
            }
        } else {
            map.insert(k, v);
        }
    }
    Ok(())
}

fn rename_column(app: &mut VizApp, tn: &str, old: &str) {
    let new = app.model_ui.table.name.trim().to_string();
    if new.is_empty() || new.contains(['[', ']']) {
        app.error("That isn't a valid name.");
        return;
    }
    if app.built.model.table(tn).is_some_and(|t| t.column_index(&new).is_some()) || app.project.model.measures.iter().any(|m| m.name.eq_ignore_ascii_case(&new)) {
        app.error(format!("The name {new} is already used."));
        return;
    }
    app.checkpoint(&format!("Rename {old}"));
    if let Some(c) = app.project.model.calculated_columns.iter_mut().find(|x| x.table == tn && x.name == old) {
        c.name = new.clone();
    }
    if let Some(cp) = app.project.model.tables.iter_mut().find(|x| x.name == tn).and_then(|t| t.columns.iter_mut().find(|c| c.name == old)) {
        cp.name = new.clone();
    }
    replace_refs(&mut app.project, &format!("[{old}]"), &format!("[{new}]"));
    app.model_ui.sel = Sel::Column(tn.to_string(), new);
    app.mark_model_dirty();
}

/// Rewrites `[old]` references in every DAX expression of the model.
pub fn replace_refs(p: &mut Project, old: &str, new: &str) {
    for m in &mut p.model.measures {
        m.expression = m.expression.replace(old, new);
    }
    for c in &mut p.model.calculated_columns {
        c.expression = c.expression.replace(old, new);
    }
    for t in &mut p.model.tables {
        if let TableSource::Calculated(d) = &mut t.source {
            *d = d.replace(old, new);
        }
    }
}

fn measure_tools(app: &mut VizApp, ui: &mut Ui, t: &Tokens, m: &str) {
    let Some(ms) = app.project.model.measures.iter().find(|x| x.name == m).cloned() else { return };
    group(ui, t, "Structure", |ui| {
        ui.vertical(|ui| {
            labeled(ui, t, "Name", |ui| {
                if name_field(app, ui, true, "") {
                    let new = app.model_ui.table.name.trim().to_string();
                    if new.is_empty() || new.contains(['[', ']']) || app.project.model.measures.iter().any(|x| x.name.eq_ignore_ascii_case(&new) && x.name != m) {
                        app.error(format!("{new} can't be used as the measure's name."));
                    } else {
                        app.checkpoint(&format!("Rename measure {m}"));
                        if let Some(x) = app.project.model.measures.iter_mut().find(|x| x.name == m) {
                            x.name = new.clone();
                        }
                        replace_refs(&mut app.project, &format!("[{m}]"), &format!("[{new}]"));
                        app.model_ui.sel = Sel::Measure(new);
                        app.mark_model_dirty();
                    }
                }
            });
            labeled(ui, t, "Home table", |ui| {
                egui::ComboBox::from_id_salt("tv_home").selected_text(&ms.table).width(150.0).show_ui(ui, |ui| {
                    let names: Vec<String> = app.project.model.tables.iter().map(|x| x.name.clone()).collect();
                    for n in names {
                        if ui.selectable_label(n == ms.table, &n).clicked() && n != ms.table {
                            app.checkpoint("Home table");
                            if let Some(x) = app.project.model.measures.iter_mut().find(|x| x.name == m) {
                                x.table = n.clone();
                            }
                            app.mark_model_dirty();
                        }
                    }
                });
            });
            labeled(ui, t, "", |ui| {
                ui.label(RichText::new(if ms.origin.is_empty() { "Measure".to_string() } else { format!("Measure ({})", ms.origin) }).size(11.0).color(t.muted));
            });
        });
    });
    group(ui, t, "Formatting", |ui| {
        ui.vertical(|ui| {
            let mut new = None;
            labeled(ui, t, "Format", |ui| new = format_picker(ui, "tv_mfmt", &ms.format, false));
            labeled(ui, t, "", |ui| {
                if let Some(f) = format_buttons(ui, &ms.format) {
                    new = Some(f);
                }
            });
            labeled(ui, t, "Custom", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.model_ui.table.custom_format).desired_width(150.0).hint_text("$#,0"));
                if r.lost_focus() && app.model_ui.table.custom_format != ms.format {
                    new = Some(app.model_ui.table.custom_format.clone());
                }
            });
            if let Some(f) = new {
                app.checkpoint(&format!("Format of {m}"));
                if let Some(x) = app.project.model.measures.iter_mut().find(|x| x.name == m) {
                    x.format = f.clone();
                }
                app.model_ui.table.custom_format = f;
                app.mark_model_dirty();
            }
        });
    });
    group(ui, t, "Properties", |ui| {
        ui.vertical(|ui| {
            labeled(ui, t, "Folder", |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.model_ui.table.folder).desired_width(150.0).hint_text("Display folder"));
                if r.lost_focus() && app.model_ui.table.folder != ms.display_folder {
                    let f = app.model_ui.table.folder.clone();
                    app.checkpoint("Display folder");
                    if let Some(x) = app.project.model.measures.iter_mut().find(|x| x.name == m) {
                        x.display_folder = f;
                    }
                }
            });
            let mut hidden = ms.hidden;
            if ui.checkbox(&mut hidden, "Hide in report view").changed() {
                app.checkpoint("Hide measure");
                if let Some(x) = app.project.model.measures.iter_mut().find(|x| x.name == m) {
                    x.hidden = hidden;
                }
                app.mark_model_dirty();
            }
            let r = ui.add(egui::TextEdit::singleline(&mut app.model_ui.table.description).desired_width(170.0).hint_text("Description"));
            if r.lost_focus() && app.model_ui.table.description != ms.description {
                let d = app.model_ui.table.description.clone();
                app.checkpoint("Description");
                if let Some(x) = app.project.model.measures.iter_mut().find(|x| x.name == m) {
                    x.description = d;
                }
            }
        });
    });
}

fn table_tools(app: &mut VizApp, ui: &mut Ui, t: &Tokens, tn: &str) {
    let Some(mt) = app.project.model.tables.iter().find(|x| x.name == tn).cloned() else { return };
    let calc = matches!(mt.source, TableSource::Calculated(_));
    group(ui, t, "Structure", |ui| {
        ui.vertical(|ui| {
            if app.model_ui.table.loaded_for != Some(Sel::Table(tn.to_string())) {
                app.model_ui.table.name = tn.to_string();
            }
            labeled(ui, t, "Name", |ui| {
                if name_field(app, ui, calc, "Tables loaded from queries are renamed in Power Query.") {
                    let new = app.model_ui.table.name.trim().to_string();
                    if new.is_empty() || app.project.model.tables.iter().any(|x| x.name.eq_ignore_ascii_case(&new)) {
                        app.error(format!("{new} can't be used as the table's name."));
                    } else {
                        app.checkpoint(&format!("Rename table {tn}"));
                        rename_table(&mut app.project, tn, &new);
                        app.model_ui.sel = Sel::Table(new);
                        app.mark_model_dirty();
                    }
                }
            });
            let source = match &mt.source {
                TableSource::Query(q) => format!("Power Query: {q}"),
                TableSource::Sheet(s) => format!("Data sheet: {s}"),
                TableSource::Calculated(_) => "Calculated table".into(),
                TableSource::Flow(f) => format!("Flow: {f}"),
                TableSource::Parameter(p) => format!("Parameter: {p}"),
                TableSource::None => "—".into(),
            };
            labeled(ui, t, "Source", |ui| {
                ui.label(RichText::new(source).size(11.5));
            });
            labeled(ui, t, "Rows", |ui| {
                ui.label(RichText::new(fmt_count(app.built.model.table(tn).map(|x| x.row_count()).unwrap_or(0))).size(11.5));
            });
        });
    });
    group(ui, t, "Properties", |ui| {
        ui.vertical(|ui| {
            let mut hidden = mt.hidden;
            if ui.checkbox(&mut hidden, "Hide in report view").changed() {
                app.checkpoint("Hide table");
                if let Some(x) = app.project.model.tables.iter_mut().find(|x| x.name == tn) {
                    x.hidden = hidden;
                }
                app.mark_model_dirty();
            }
            let r = ui.add(egui::TextEdit::multiline(&mut app.model_ui.table.description).desired_width(180.0).desired_rows(2).hint_text("Description"));
            if r.lost_focus() && app.model_ui.table.description != mt.description {
                let d = app.model_ui.table.description.clone();
                app.checkpoint("Description");
                if let Some(x) = app.project.model.tables.iter_mut().find(|x| x.name == tn) {
                    x.description = d;
                }
            }
        });
    });
    group(ui, t, "Calendars", |ui| {
        if big_button(ui, t, "data", if mt.is_date_table { "Date table (on)" } else { "Mark as date table" }, "Mark this table as the model's date table").clicked() {
            super::mark_date_table(app);
        }
    });
}

pub fn rename_table(p: &mut Project, old: &str, new: &str) {
    if let Some(x) = p.model.tables.iter_mut().find(|x| x.name == old) {
        x.name = new.to_string();
    }
    for m in &mut p.model.measures {
        if m.table == old {
            m.table = new.to_string();
        }
    }
    for c in &mut p.model.calculated_columns {
        if c.table == old {
            c.table = new.to_string();
        }
    }
    for r in &mut p.model.relationships {
        if r.from_table == old {
            r.from_table = new.to_string();
        }
        if r.to_table == old {
            r.to_table = new.to_string();
        }
    }
    let (oq, nq) = (super::qt(old), super::qt(new));
    replace_refs(p, &format!("{oq}["), &format!("{nq}["));
    replace_refs(p, &format!("'{old}'["), &format!("{nq}["));
}

fn common_buttons(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Calculations", |ui| {
        if big_button(ui, t, "measure", "New measure", "A DAX measure (formula bar)").clicked() {
            super::new_measure(app, None);
        }
        if big_button(ui, t, "column", "New column", "A calculated column").clicked() {
            let tn = shown_table(app);
            super::new_column(app, tn.as_deref());
        }
        if big_button(ui, t, "table", "New table", "A calculated table (DAX)").clicked() {
            super::new_table(app);
        }
        if big_button(ui, t, "sparkle", "Quick measure", "Measures from templates").clicked() {
            super::quick_measure(app);
        }
    });
    group(ui, t, "Relationships", |ui| {
        if big_button(ui, t, "relationship", "Manage relationships", "").clicked() {
            super::manage_relationships(app);
        }
    });
    if matches!(app.model_ui.sel, Sel::Column(..) | Sel::Measure(_)) {
        group(ui, t, "Calendars", |ui| {
            if big_button(ui, t, "data", "Mark as date table", "").clicked() {
                super::mark_date_table(app);
            }
        });
    }
}

// ------------------------------------------------------------------------------------- formula bar

fn formula_bar(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(target) = formula_target(app) else { return };
    let measures: Vec<String> = app.built.model.measures.iter().map(|m| m.name.clone()).collect();
    let (label, lang_hint) = match &target {
        Sel::Measure(m) => (format!("{m} ="), "measure"),
        Sel::Column(_, c) => (format!("{c} ="), "calculated column"),
        Sel::Table(tn) => (format!("{tn} ="), "calculated table"),
        _ => (String::new(), ""),
    };
    let mut commit = false;
    let mut cancel = false;
    ui.horizontal_top(|ui| {
        if super::icon_btn(ui, t, "x", "Discard changes", Some(t.danger)).clicked() {
            cancel = true;
        }
        if super::icon_btn(ui, t, "check", "Commit (Ctrl+Enter)", Some(t.good)).clicked() {
            commit = true;
        }
        ui.label(RichText::new(label).monospace().strong().color(t.text));
        let lines = app.model_ui.table.formula.lines().count().clamp(1, 8);
        let h = lines as f32 * 17.0 + 12.0;
        let out = editor::code_editor(ui, ui.scope_id().with("tv_formula_editor"), &mut app.model_ui.table.formula, Lang::Dax, t, &measures, h, &format!("DAX for this {lang_hint}"));
        if out.run {
            commit = true;
        }
    });
    if let Some(e) = &app.model_ui.table.formula_error {
        ui.label(RichText::new(e).color(t.danger).size(11.5));
    }
    if cancel {
        app.model_ui.table.loaded_for = None;
    }
    if commit {
        let text = app.model_ui.table.formula.trim().to_string();
        let parsed = match &target {
            Sel::Table(_) => app.built.model.query(&text).map(|_| ()).map_err(|e| e.to_string()),
            _ => vizcraft_datamodel::dax::parse(&text).map(|_| ()).map_err(|e| e.to_string()),
        };
        if let Err(e) = parsed {
            app.model_ui.table.formula_error = Some(e);
            return;
        }
        app.model_ui.table.formula_error = None;
        app.checkpoint("Edit formula");
        match &target {
            Sel::Measure(m) => {
                if let Some(x) = app.project.model.measures.iter_mut().find(|x| &x.name == m) {
                    x.expression = text;
                }
            }
            Sel::Column(tn, c) => {
                if let Some(x) = app.project.model.calculated_columns.iter_mut().find(|x| &x.table == tn && &x.name == c) {
                    x.expression = text;
                }
            }
            Sel::Table(tn) => {
                if let Some(x) = app.project.model.tables.iter_mut().find(|x| &x.name == tn) {
                    x.source = TableSource::Calculated(text);
                }
            }
            _ => {}
        }
        app.mark_model_dirty();
    }
}

// ------------------------------------------------------------------------------------- grid

fn grid(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(tn) = shown_table(app) else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("No data loaded yet.\nGet data (Home ▸ Get data) or add a data sheet, then Refresh.").size(14.0).color(t.muted));
        });
        return;
    };
    let Some(tb) = app.built.model.table(&tn) else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(format!("{tn} has no data yet. Refresh to load it.")).color(t.muted));
        });
        return;
    };
    let meta = tables_meta(&app.project, &app.built).into_iter().find(|m| m.name == tn);
    let Some(meta) = meta else { return };
    let cols: Vec<(usize, &super::ColMeta)> = meta.columns.iter().filter_map(|c| tb.column_index(&c.name).map(|i| (i, c))).collect();
    // Row order: filters, then sort.
    let st = &app.model_ui.table;
    let key = format!("{tn}|{}|{:?}|{:?}", app.generation, st.sort, st.filters);
    if key != st.cache_key {
        let mut order: Vec<u32> = (0..tb.row_count() as u32).collect();
        for (col, f) in &st.filters {
            if f.is_empty() {
                continue;
            }
            let Some(c) = tb.column_index(col).and_then(|i| tb.column(i)) else { continue };
            let f_low = f.to_lowercase();
            let exact = f_low.strip_prefix('=').map(str::to_string);
            order.retain(|r| {
                let txt = c.value(*r as usize).to_text().to_lowercase();
                match &exact {
                    Some(e) => txt == *e,
                    None => txt.contains(&f_low),
                }
            });
        }
        if let Some((col, asc)) = &st.sort
            && let Some(c) = tb.column_index(col).and_then(|i| tb.column(i))
        {
            order.sort_by(|a, b| {
                let o = vizcraft_datamodel::value::sort_order(c.value(*a as usize), c.value(*b as usize));
                if *asc { o } else { o.reverse() }
            });
        }
        let st = &mut app.model_ui.table;
        st.cache_key = key;
        st.order = order;
    }
    let sel_col = match &app.model_ui.sel {
        Sel::Column(t2, c) if *t2 == tn => Some(c.clone()),
        _ => None,
    };
    let formats: Vec<String> = cols.iter().map(|(_, c)| effective_format(&app.project, &tn, &c.name, c.ty)).collect();
    let mut gcols = Vec::with_capacity(cols.len());
    for ((ci, c), _fmt) in cols.iter().zip(formats.iter()) {
        let key = (tn.clone(), c.name.clone());
        let width = match app.model_ui.table.widths.get(&key) {
            Some(w) => *w,
            None => {
                let col = tb.column(*ci);
                let samples = (0..tb.row_count().min(60)).map(|r| col.map(|x| cell_text(x.value(r), c.ty, _fmt)).unwrap_or_default());
                let w = super::grid::fit_width(ui, &c.name, samples);
                app.model_ui.table.widths.insert(key, w);
                w
            }
        };
        gcols.push(super::grid::GridCol {
            name: c.name.clone(),
            glyph: String::new(),
            ty: Some((c.ty, c.calc, c.summarize)),
            numeric: c.ty.is_numeric(),
            width,
            selected: sel_col.as_deref() == Some(c.name.as_str()),
            sorted: app.model_ui.table.sort.as_ref().filter(|(s, _)| *s == c.name).map(|(_, a)| *a),
            filtered: app.model_ui.table.filters.get(&c.name).is_some_and(|f| !f.is_empty()),
            dim: c.hidden,
        });
    }
    let order = &app.model_ui.table.order;
    let cell = |r: usize, c: usize| -> String {
        let Some(row) = order.get(r) else { return String::new() };
        let Some((ci, meta)) = cols.get(c) else { return String::new() };
        let fmt = formats.get(c).map(String::as_str).unwrap_or("");
        tb.column(*ci).map(|col| cell_text(col.value(*row as usize), meta.ty, fmt)).unwrap_or_default()
    };
    let out = super::grid::show(ui, t, &format!("tv_grid_{tn}"), &gcols, order.len(), &cell);
    if let Some(ci) = out.header_clicked.or(out.cell_clicked.map(|(_, c)| c))
        && let Some((_, c)) = cols.get(ci)
    {
        app.model_ui.sel = Sel::Column(tn.clone(), c.name.clone());
    }
    if let Some((ci, pos)) = out.header_menu
        && let Some((_, c)) = cols.get(ci)
    {
        app.model_ui.table.menu_filter = app.model_ui.table.filters.get(&c.name).cloned().unwrap_or_default();
        app.model_ui.table.menu = Some((c.name.clone(), pos));
        app.model_ui.table.menu_fresh = true;
        app.model_ui.sel = Sel::Column(tn.clone(), c.name.clone());
    }
    header_menu(app, ui, t, &tn);
}

fn header_menu(app: &mut VizApp, ui: &mut Ui, t: &Tokens, tn: &str) {
    let Some((col, pos)) = app.model_ui.table.menu.clone() else { return };
    let mut close = false;
    let mut acts = Vec::new();
    let area = egui::Area::new(ui.scope_id().with("tv_header_menu")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(220.0);
            let st = &mut app.model_ui.table;
            if ui.button("Sort ascending").clicked() {
                st.sort = Some((col.clone(), true));
                close = true;
            }
            if ui.button("Sort descending").clicked() {
                st.sort = Some((col.clone(), false));
                close = true;
            }
            if st.sort.is_some() && ui.button("Clear sort").clicked() {
                st.sort = None;
                close = true;
            }
            ui.separator();
            ui.label(RichText::new("Filter (contains, or =value)").size(11.0).color(t.muted));
            let r = ui.add(egui::TextEdit::singleline(&mut st.menu_filter).hint_text("Text to find").desired_width(200.0));
            if r.changed() {
                st.filters.insert(col.clone(), st.menu_filter.clone());
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                close = true;
            }
            if st.filters.get(&col).is_some_and(|f| !f.is_empty()) && ui.button("Clear filter").clicked() {
                st.filters.remove(&col);
                st.menu_filter.clear();
                close = true;
            }
            if !st.filters.is_empty() && ui.button("Clear all filters").clicked() {
                st.filters.clear();
                st.menu_filter.clear();
                close = true;
            }
            ui.separator();
            if ui.button("Copy column reference").clicked() {
                ui.ctx().copy_text(col_ref(tn, &col));
                close = true;
            }
            if ui.button("New measure").clicked() {
                acts.push(Act::NewMeasure(tn.to_string()));
                close = true;
            }
            if ui.button("New column").clicked() {
                acts.push(Act::NewColumn(tn.to_string()));
                close = true;
            }
            if ui.button("Hide in report view").clicked() {
                acts.push(Act::ToggleHidden(Sel::Column(tn.to_string(), col.clone())));
                close = true;
            }
            if app.project.model.calculated_columns.iter().any(|x| x.table == tn && x.name == col) && ui.button("Delete column").clicked() {
                acts.push(Act::Delete(Sel::Column(tn.to_string(), col.clone())));
                close = true;
            }
        });
    });
    let fresh = std::mem::take(&mut app.model_ui.table.menu_fresh);
    if close || (!fresh && area.response.clicked_elsewhere()) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.model_ui.table.menu = None;
    }
    let _ = Color32::TRANSPARENT;
    for a in acts {
        run_act(app, a);
    }
}
