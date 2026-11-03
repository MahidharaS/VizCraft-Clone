//! DAX query view: query tabs with a highlighted editor, Run (Ctrl+Enter / F5), results grid
//! with row count and timing, friendly errors, quick queries from the Data pane, a function
//! reference, and "Update model" for `DEFINE MEASURE` blocks.

use egui::{Color32, RichText, Sense, Ui, vec2};
use vizcraft_core::doc::Measure;
use vizcraft_core::scalar::Scalar;
use vizcraft_datamodel::QueryResult;

use super::editor::{self, Lang};
use super::{Act, PaneMode, data_pane, glyph_icon, icon_btn, run_act, strip_group};
use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets::big_button;

pub struct DaxTab {
    pub name: String,
    pub text: String,
    id: u64,
    result: Option<Result<(QueryResult, f64), String>>,
    widths: Vec<f32>,
}

pub struct DaxState {
    tabs: Vec<DaxTab>,
    active: usize,
    pub run_pending: bool,
    renaming: Option<(usize, String)>,
    side: usize,
    fn_search: String,
    next_id: u64,
}

const WELCOME: &str = "// DAX query view: write a query and run it with Ctrl+Enter or F5.\n// Right-click a table or measure in the Data pane for quick queries.\nEVALUATE\n\tSUMMARIZECOLUMNS(\n\t\tProduct[Category],\n\t\t\"Total Sales\", [Total Sales],\n\t\t\"Units\", [Units],\n\t\t\"Margin %\", [Margin %]\n\t)\nORDER BY Product[Category]";

impl Default for DaxState {
    fn default() -> Self {
        DaxState { tabs: vec![DaxTab { name: "Query 1".into(), text: WELCOME.into(), id: 1, result: None, widths: vec![] }], active: 0, run_pending: true, renaming: None, side: 0, fn_search: String::new(), next_id: 2 }
    }
}

impl DaxState {
    /// Opens a new tab with a query.
    pub fn open_query(&mut self, name: &str, text: String) {
        let id = self.next_id;
        self.next_id += 1;
        let base = name.to_string();
        let name = if self.tabs.iter().any(|t| t.name == base) { (2..).map(|i| format!("{base} ({i})")).find(|n| !self.tabs.iter().any(|t| &t.name == n)).unwrap_or(base) } else { base };
        self.tabs.push(DaxTab { name, text, id, result: None, widths: vec![] });
        self.active = self.tabs.len() - 1;
    }
}

/// `DEFINE MEASURE 'T'[Name] = expression` blocks of a query: (table, name, expression).
pub fn defined_measures(q: &str) -> Vec<(String, String, String)> {
    // Strip comments, find DEFINE … EVALUATE.
    let clean: String = q.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let upper = clean.to_ascii_uppercase();
    let Some(def) = upper.find("DEFINE") else { return vec![] };
    let end = upper[def..].find("EVALUATE").map(|e| e + def).unwrap_or(clean.len());
    let section = &clean[def + 6..end];
    let su = section.to_ascii_uppercase();
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0;
    while let Some(p) = su[i..].find("MEASURE") {
        let at = i + p;
        let before_ok = at == 0 || !su.as_bytes().get(at - 1).is_some_and(|b| b.is_ascii_alphanumeric());
        let after_ok = !su.as_bytes().get(at + 7).is_some_and(|b| b.is_ascii_alphanumeric());
        if before_ok && after_ok {
            starts.push(at);
        }
        i = at + 7;
    }
    let mut out = Vec::new();
    for (k, s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(section.len());
        let body = section[s + 7..e].trim();
        // Table name.
        let (table, rest) = if let Some(b) = body.strip_prefix('\'') {
            match b.find('\'') {
                Some(j) => (b[..j].to_string(), &b[j + 1..]),
                None => continue,
            }
        } else {
            match body.find('[') {
                Some(j) => (body[..j].trim().to_string(), &body[j..]),
                None => continue,
            }
        };
        let Some(rest) = rest.trim_start().strip_prefix('[') else { continue };
        let Some(j) = rest.find(']') else { continue };
        let name = rest[..j].to_string();
        let Some(expr) = rest[j + 1..].trim_start().strip_prefix('=') else { continue };
        // A VAR / COLUMN / TABLE definition ends a measure: keep it simple and stop at the next
        // top-level definition keyword on its own line.
        let mut lines = Vec::new();
        for l in expr.lines() {
            let tl = l.trim_start().to_ascii_uppercase();
            if (tl.starts_with("VAR ") || tl.starts_with("COLUMN ") || tl.starts_with("TABLE ")) && !lines.is_empty() && l.len() - l.trim_start().len() <= 1 {
                break;
            }
            lines.push(l);
        }
        let expr = dedent(&lines.join("\n"));
        out.push((table, name, expr.trim().to_string()));
    }
    out
}

fn dedent(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let rest: Vec<&&str> = lines.iter().skip(1).filter(|l| !l.trim().is_empty()).collect();
    let common = rest.iter().map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if i == 0 {
            out.push(l.trim().to_string());
        } else {
            out.push(l.get(common.min(l.len() - l.trim_start().len())..).unwrap_or(l).to_string());
        }
    }
    out.join("\n")
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run(app: &mut VizApp) {
    let st = &app.model_ui.dax;
    let Some(tab) = st.tabs.get(st.active) else { return };
    let text = vizcraft_core::build::substitute_params(&tab.text, &app.project, &app.params());
    let t0 = std::time::Instant::now();
    let r = app.built.model.query(&text).map_err(|e| e.to_string());
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if let Some(tab) = app.model_ui.dax.tabs.get_mut(app.model_ui.dax.active) {
        tab.result = Some(r.map(|q| (q, ms)));
        tab.widths.clear();
    }
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    if app.model_ui.dax.tabs.is_empty() {
        app.model_ui.dax = DaxState::default();
    }
    if app.model_ui.dax.run_pending {
        app.model_ui.dax.run_pending = false;
        run(app);
    }
    // F5 runs the query here (instead of refreshing the data).
    if ui.input(|i| i.key_pressed(egui::Key::F5)) {
        app.needs_refresh = false;
        run(app);
    }
    egui::Panel::top("dax_tools").frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 2 }).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| toolbar(app, ui, &t));
    });
    let mut acts = Vec::new();
    egui::Panel::right("dax_side").default_size(270.0).size_range(210.0..=460.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (i, l) in ["Data", "Functions"].iter().enumerate() {
                let on = app.model_ui.dax.side == i;
                if ui.add(egui::Button::new(RichText::new(*l).color(if on { t.accent } else { t.text }).strong()).fill(if on { t.accent_soft } else { Color32::TRANSPARENT })).clicked() {
                    app.model_ui.dax.side = i;
                }
            }
        });
        ui.separator();
        if app.model_ui.dax.side == 0 {
            acts = data_pane(app, ui, PaneMode::Dax);
            ui.label(RichText::new("Double-click to insert · right-click for quick queries").size(10.5).color(t.muted));
        } else {
            let mut search = std::mem::take(&mut app.model_ui.dax.fn_search);
            if let Some(s) = editor::function_list(ui, &t, &mut search) {
                acts.push(Act::Insert(s));
            }
            app.model_ui.dax.fn_search = search;
        }
    });
    egui::Panel::top("dax_tabs").exact_size(32.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(8, 3)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| tabs(app, ui, &t));
    egui::Panel::top("dax_editor").resizable(true).default_size(330.0).size_range(120.0..=800.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(8))).show(ui, |ui| {
        let measures: Vec<String> = app.built.model.measures.iter().map(|m| m.name.clone()).collect();
        let active = app.model_ui.dax.active;
        let h = ui.available_height() - 4.0;
        if let Some(tab) = app.model_ui.dax.tabs.get_mut(active) {
            let id = egui::Id::unique(("dax_editor", tab.id));
            let out = editor::code_editor(ui, id, &mut tab.text, Lang::Dax, &t, &measures, h, "EVALUATE …");
            if out.run {
                app.model_ui.dax.run_pending = true;
            }
        }
    });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(8))).show(ui, |ui| results(app, ui, &t));
    for a in acts {
        match a {
            Act::Insert(s) => {
                let active = app.model_ui.dax.active;
                if let Some(tab) = app.model_ui.dax.tabs.get_mut(active) {
                    editor::insert_at_cursor(ui.ctx(), egui::Id::unique(("dax_editor", tab.id)), &mut tab.text, &s);
                }
            }
            other => run_act(app, other),
        }
    }
}

fn toolbar(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    strip_group(ui, t, "Query", |ui| {
        if big_button(ui, t, "dax", "Run", "Run the query (Ctrl+Enter or F5)").clicked() {
            run(app);
        }
        if big_button(ui, t, "new", "New query", "Open a new query tab").clicked() {
            let n = (1..).find(|i| !app.model_ui.dax.tabs.iter().any(|x| x.name == format!("Query {i}"))).unwrap_or(1);
            app.model_ui.dax.open_query(&format!("Query {n}"), "EVALUATE\n\t".into());
        }
    });
    strip_group(ui, t, "Quick queries", |ui| {
        ui.vertical(|ui| {
            if ui.button("Evaluate all measures").on_hover_text("One query showing every measure").clicked() {
                let ms: Vec<(String, String)> = app.built.model.measures.iter().map(|m| (m.name.clone(), m.name.clone())).collect();
                let body = ms.iter().map(|(n, _)| format!("\t\t\"{}\", [{}]", n.replace('"', "\"\""), n)).collect::<Vec<_>>().join(",\n");
                app.model_ui.dax.open_query("All measures", format!("EVALUATE\n\tROW(\n{body}\n\t)"));
                app.model_ui.dax.run_pending = true;
            }
            if ui.button("Model tables").on_hover_text("Row counts of every table").clicked() {
                let names: Vec<String> = app.built.model.tables.iter().map(|t| t.name.clone()).collect();
                let rows = names.iter().map(|n| format!("\t\t(\"{}\", COUNTROWS({}))", n.replace('"', "\"\""), super::qt(n))).collect::<Vec<_>>().join(",\n");
                app.model_ui.dax.open_query("Tables", format!("EVALUATE\n\t{{\n{rows}\n\t}}"));
                app.model_ui.dax.run_pending = true;
            }
        });
    });
    // Update the model from DEFINE MEASURE blocks.
    let active = app.model_ui.dax.active;
    let defs = app.model_ui.dax.tabs.get(active).map(|x| defined_measures(&x.text)).unwrap_or_default();
    let mut new = 0;
    let mut changed = 0;
    for (_, n, e) in &defs {
        match app.project.model.measures.iter().find(|m| m.name.eq_ignore_ascii_case(n)) {
            Some(m) if norm(&m.expression) != norm(e) => changed += 1,
            None => new += 1,
            _ => {}
        }
    }
    if new + changed > 0 {
        strip_group(ui, t, "Model", |ui| {
            let label = match (new, changed) {
                (0, c) => format!("Update model: {c} changed"),
                (n, 0) => format!("Update model: {n} new"),
                (n, c) => format!("Update model: {n} new, {c} changed"),
            };
            if big_button(ui, t, "model", &label, "Save the DEFINE MEASURE definitions of this query to the model").clicked() {
                app.checkpoint("Update model from DAX query");
                for (tn, n, e) in defs {
                    let table = app.project.model.tables.iter().find(|x| x.name.eq_ignore_ascii_case(&tn)).map(|x| x.name.clone());
                    match app.project.model.measures.iter_mut().find(|m| m.name.eq_ignore_ascii_case(&n)) {
                        Some(m) => {
                            m.expression = e;
                            if let Some(tb) = table {
                                m.table = tb;
                            }
                        }
                        None => app.project.model.measures.push(Measure { table: table.unwrap_or(tn), name: n, expression: e, origin: "DAX query view".into(), ..Measure::default() }),
                    }
                }
                app.mark_model_dirty();
                app.toast("Model updated from the query's DEFINE MEASURE blocks");
            }
        });
    }
}

fn tabs(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal_centered(|ui| {
        let n = app.model_ui.dax.tabs.len();
        let mut close = None;
        for i in 0..n {
            let active = app.model_ui.dax.active == i;
            if let Some((ri, buf)) = &mut app.model_ui.dax.renaming
                && *ri == i
            {
                let r = ui.add(egui::TextEdit::singleline(buf).desired_width(120.0));
                r.request_focus();
                if r.lost_focus() {
                    let new = buf.trim().to_string();
                    if let Some(tab) = app.model_ui.dax.tabs.get_mut(i)
                        && !new.is_empty()
                    {
                        tab.name = new;
                    }
                    app.model_ui.dax.renaming = None;
                }
                continue;
            }
            let Some(tab) = app.model_ui.dax.tabs.get(i) else { continue };
            let name = tab.name.clone();
            let failed = matches!(tab.result, Some(Err(_)));
            egui::Frame::new().fill(if active { t.panel } else { Color32::TRANSPARENT }).corner_radius(4.0).stroke(if active { egui::Stroke::new(1.0, t.border) } else { egui::Stroke::NONE }).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let r = ui.add(egui::Label::new(RichText::new(&name).size(12.0).color(if active { t.accent } else { t.text })).sense(Sense::click()));
                    if failed {
                        let (rr, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        ui.painter().circle_filled(rr.center(), 3.0, t.danger);
                    }
                    if r.clicked() {
                        app.model_ui.dax.active = i;
                    }
                    if r.double_clicked() {
                        app.model_ui.dax.renaming = Some((i, name.clone()));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            app.model_ui.dax.renaming = Some((i, name.clone()));
                            ui.close();
                        }
                        if ui.button("Duplicate").clicked() {
                            let text = app.model_ui.dax.tabs.get(i).map(|x| x.text.clone()).unwrap_or_default();
                            app.model_ui.dax.open_query(&name, text);
                            ui.close();
                        }
                        if ui.button("Close").clicked() {
                            close = Some(i);
                            ui.close();
                        }
                    });
                    let (xr, xresp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                    if xresp.hovered() {
                        ui.painter().rect_filled(xr, 3.0, t.border);
                    }
                    glyph_icon(ui.painter(), xr.shrink(2.0), "x", t.muted);
                    if xresp.on_hover_text("Close").clicked() {
                        close = Some(i);
                    }
                });
            });
        }
        if icon_btn(ui, t, "plus", "New query", Some(t.accent)).clicked() {
            let n = (1..).find(|i| !app.model_ui.dax.tabs.iter().any(|x| x.name == format!("Query {i}"))).unwrap_or(1);
            app.model_ui.dax.open_query(&format!("Query {n}"), "EVALUATE\n\t".into());
        }
        if let Some(i) = close {
            let st = &mut app.model_ui.dax;
            st.tabs.remove(i);
            if st.tabs.is_empty() {
                st.open_query("Query 1", "EVALUATE\n\t".into());
            }
            st.active = st.active.min(st.tabs.len() - 1);
        }
    });
}

fn cell(v: &vizcraft_datamodel::Value) -> String {
    match v {
        vizcraft_datamodel::Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                vizcraft_core::numfmt::format(*n, "#,0")
            } else {
                vizcraft_core::numfmt::format(*n, "#,0.####")
            }
        }
        vizcraft_datamodel::Value::Blank => String::new(),
        other => Scalar::from_dm(other).text(),
    }
}

fn results(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let active = app.model_ui.dax.active;
    let Some(tab) = app.model_ui.dax.tabs.get(active) else { return };
    match &tab.result {
        None => {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Run the query to see results").size(15.0).color(t.text));
                ui.label(RichText::new("Ctrl+Enter or F5 runs it · results show here with their row count and timing").color(t.muted));
            });
        }
        Some(Err(e)) => {
            let e = e.clone();
            ui.add_space(4.0);
            egui::Frame::new().fill(t.danger.gamma_multiply(0.08)).stroke(egui::Stroke::new(1.0, t.danger.gamma_multiply(0.6))).corner_radius(6.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    glyph_icon(ui.painter(), r, "warn", t.danger);
                    ui.label(RichText::new("The query couldn't run").strong().size(14.0).color(t.danger));
                });
                ui.add_space(4.0);
                ui.label(RichText::new(&e).monospace().size(12.5));
                ui.add_space(6.0);
                let hint = if e.contains("syntax") {
                    "Check the brackets and commas. A query starts with EVALUATE followed by a table expression, such as EVALUATE VALUES(Product[Category])."
                } else if e.contains("table expression was expected") {
                    "EVALUATE needs a table: wrap a single value in ROW(\"Name\", …) or { … }."
                } else if e.contains("no table") || e.contains("no column") {
                    "Check the table and column names in the Data pane (double-click one to insert it)."
                } else {
                    "Check the expression; the Functions tab lists what's available."
                };
                ui.label(RichText::new(hint).size(11.5).color(t.muted));
            });
        }
        Some(Ok((r, ms))) => {
            let (nrows, ncols, ms) = (r.rows.len(), r.columns.len(), *ms);
            ui.horizontal(|ui| {
                let (rr, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                glyph_icon(ui.painter(), rr, "check", t.good);
                ui.label(RichText::new(format!("{} row{} · {} column{}", super::fmt_count(nrows), if nrows == 1 { "" } else { "s" }, ncols, if ncols == 1 { "" } else { "s" })).strong());
                ui.label(RichText::new(format!("ran in {ms:.1} ms")).color(t.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Copy as CSV").clicked() {
                        let mut s = r.columns.join(",") + "\n";
                        for row in &r.rows {
                            s += &row.iter().map(|v| {
                                let x = Scalar::from_dm(v);
                                let txt = if x.is_null() { String::new() } else { x.text() };
                                if txt.contains([',', '"', '\n']) { format!("\"{}\"", txt.replace('"', "\"\"")) } else { txt }
                            }).collect::<Vec<_>>().join(",");
                            s.push('\n');
                        }
                        ui.ctx().copy_text(s);
                    }
                });
            });
            ui.add_space(4.0);
            let numeric: Vec<bool> = (0..ncols).map(|c| r.rows.iter().any(|row| matches!(row.get(c), Some(vizcraft_datamodel::Value::Number(_)))) && r.rows.iter().all(|row| matches!(row.get(c), Some(vizcraft_datamodel::Value::Number(_) | vizcraft_datamodel::Value::Blank) | None))).collect();
            let widths: Vec<f32> = if tab.widths.len() == ncols {
                tab.widths.clone()
            } else {
                let w: Vec<f32> = (0..ncols).map(|c| super::grid::fit_width(ui, r.columns.get(c).map(String::as_str).unwrap_or(""), r.rows.iter().map(|row| row.get(c).map(cell).unwrap_or_default()))).collect();
                w
            };
            let cols: Vec<super::grid::GridCol> = r
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| super::grid::GridCol {
                    name: c.clone(),
                    glyph: String::new(),
                    ty: None,
                    numeric: numeric.get(i).copied().unwrap_or(false),
                    width: widths.get(i).copied().unwrap_or(120.0),
                    selected: false,
                    sorted: None,
                    filtered: false,
                    dim: false,
                })
                .collect();
            let rows = &r.rows;
            let f = |ri: usize, ci: usize| rows.get(ri).and_then(|row| row.get(ci)).map(cell).unwrap_or_default();
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
                super::grid::show(ui, t, &format!("dax_grid_{active}"), &cols, nrows, &f);
            });
            if let Some(tab) = app.model_ui.dax.tabs.get_mut(active) {
                tab.widths = widths;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_define_measures() {
        let q = "DEFINE\n\tMEASURE Sales[Big] =\n\t\tVAR x = [Units]\n\t\tRETURN x * 2\n\tMEASURE 'Sales Data'[Other] = 1 // note\nEVALUATE ROW(\"a\", [Big])";
        let d = defined_measures(q);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], ("Sales".into(), "Big".into(), "VAR x = [Units]\nRETURN x * 2".into()));
        assert_eq!(d[1], ("Sales Data".into(), "Other".into(), "1".into()));
    }
}
