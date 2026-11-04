//! Notebooks: Markdown, Python (every model table as a pandas DataFrame), DAX (EVALUATE against
//! the model) and M (Power Query) cells with their outputs.

use std::collections::{HashMap, HashSet};

use egui::{Color32, RichText, Ui, vec2};
use vizardcraft_core::build::TableData;
use vizardcraft_core::doc::NotebookCell;
use vizardcraft_core::flow::{self, PyJob};
use vizardcraft_core::scalar::Scalar;

use super::FlowsUi;
use crate::app::VizApp;
use crate::theme::Tokens;

#[derive(Clone, Debug, Default)]
pub struct CellOut {
    pub n: u32,
    pub text: String,
    pub error: Option<String>,
    pub table: Option<TableData>,
    pub ms: f64,
}

#[derive(Default)]
pub struct NotebookState {
    pub outputs: HashMap<u64, CellOut>,
    pub editing_md: HashSet<u64>,
    pub run_all: bool,
    pub run_cell: Option<u64>,
    pub counter: u32,
    pub python: Option<Result<String, String>>,
}

const KINDS: &[(&str, &str)] = &[("python", "Python"), ("dax", "DAX"), ("m", "M"), ("markdown", "Markdown")];

fn kind_color(k: &str) -> Color32 {
    match k {
        "python" => Color32::from_rgb(0x16, 0xA3, 0x4A),
        "dax" => Color32::from_rgb(0xEA, 0x8C, 0x0C),
        "m" => Color32::from_rgb(0x3B, 0x82, 0xF6),
        _ => Color32::from_rgb(0x64, 0x74, 0x8B),
    }
}

fn python_version() -> Result<String, String> {
    let exe = if cfg!(windows) { "python" } else { "python3" };
    match std::process::Command::new(exe).arg("-c").arg("import sys\ntry:\n import pandas; p='pandas '+pandas.__version__\nexcept Exception:\n p='no pandas (csv lists)'\nprint(sys.version.split()[0], '·', p)").output() {
        Ok(o) if o.status.success() => Ok(format!("Python {}", String::from_utf8_lossy(&o.stdout).trim())),
        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
        Err(e) => Err(format!("{exe} not found: {e}")),
    }
}

pub fn show(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, ni: usize) {
    if st.nb.python.is_none() {
        st.nb.python = Some(python_version());
    }
    // Pending runs.
    if st.nb.run_all {
        st.nb.run_all = false;
        let ids: Vec<u64> = app.project.notebooks.get(ni).map(|n| n.cells.iter().filter(|c| c.kind != "markdown").map(|c| c.id).collect()).unwrap_or_default();
        for id in ids {
            run_cell(app, st, ni, id);
        }
    }
    if let Some(id) = st.nb.run_cell.take() {
        run_cell(app, st, ni, id);
    }
    // Toolbar.
    egui::Panel::top("nb_toolbar").exact_size(38.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 5)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if super::tbtn(ui, t, "play", "Run all", "Run every cell from the top", true).clicked() {
                st.nb.run_all = true;
            }
            ui.add_space(6.0);
            for (k, l) in [("python", "+ Python"), ("dax", "+ DAX"), ("m", "+ M"), ("markdown", "+ Markdown")] {
                if super::tbtn(ui, t, "", l, "Add a cell at the end", false).clicked() {
                    add_cell(app, ni, None, k);
                }
            }
            if super::tbtn(ui, t, "", "Clear outputs", "Remove every cell output", false).clicked() {
                st.nb.outputs.clear();
                st.nb.counter = 0;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &st.nb.python {
                Some(Ok(v)) => {
                    ui.label(RichText::new(v).size(11.5).color(t.muted));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new("Python unavailable").size(11.5).color(t.danger)).on_hover_text(e);
                }
                None => {}
            });
        });
    });
    // Tables (DataFrame names).
    egui::Panel::left("nb_tables").exact_size(212.0).resizable(false).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(10, 10)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.label(RichText::new("Model tables").strong().size(13.0));
        ui.label(RichText::new("Available in Python cells as DataFrames (click to copy the name).").size(11.0).color(t.muted));
        ui.add_space(6.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut copied = None;
            for (name, data) in &app.built.tables {
                let py = flow::py_ident(name);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 5.0, t.accent_soft);
                }
                crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), vec2(16.0, 16.0)), "table", t.accent);
                ui.painter().text(rect.left_top() + vec2(26.0, 11.0), egui::Align2::LEFT_CENTER, &py, egui::FontId::monospace(12.0), t.text);
                ui.painter().text(rect.left_top() + vec2(26.0, 27.0), egui::Align2::LEFT_CENTER, format!("{} rows · {} cols", data.rows.len(), data.columns.len()), egui::FontId::proportional(10.5), t.muted);
                if resp.on_hover_text(data.columns.join(", ")).clicked() {
                    ui.ctx().copy_text(py.clone());
                    copied = Some(py);
                }
            }
            if let Some(py) = copied {
                app.toast(format!("Copied “{py}”"));
            }
            if app.built.tables.is_empty() {
                ui.label(RichText::new("No tables loaded yet.").color(t.muted));
            }
        });
    });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(12.0);
            let width = (ui.available_width() - 40.0).min(980.0);
            let left = ((ui.available_width() - width) / 2.0).max(12.0);
            let cells: Vec<NotebookCell> = app.project.notebooks.get(ni).map(|n| n.cells.clone()).unwrap_or_default();
            let mut action: Option<(&str, u64)> = None;
            for c in &cells {
                ui.horizontal(|ui| {
                    ui.add_space(left);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        if let Some(a) = cell_ui(app, st, ui, t, ni, c) {
                            action = Some((a, c.id));
                        }
                    });
                });
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                ui.add_space(left);
                for (k, l) in [("python", "+ Python"), ("dax", "+ DAX"), ("m", "+ M"), ("markdown", "+ Markdown")] {
                    if ui.button(l).clicked() {
                        add_cell(app, ni, None, k);
                    }
                }
            });
            ui.add_space(40.0);
            match action {
                Some(("run", id)) => st.nb.run_cell = Some(id),
                Some(("delete", id)) => {
                    app.checkpoint("Delete cell");
                    if let Some(n) = app.project.notebooks.get_mut(ni) {
                        n.cells.retain(|c| c.id != id);
                    }
                }
                Some((dir @ ("up" | "down"), id)) => {
                    app.checkpoint("Move cell");
                    if let Some(n) = app.project.notebooks.get_mut(ni)
                        && let Some(i) = n.cells.iter().position(|c| c.id == id)
                    {
                        let j = if dir == "up" { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < n.cells.len()) };
                        if let Some(j) = j {
                            n.cells.swap(i, j);
                        }
                    }
                }
                Some((k @ ("below_python" | "below_md"), id)) => add_cell(app, ni, Some(id), if k == "below_python" { "python" } else { "markdown" }),
                _ => {}
            }
        });
    });
}

fn add_cell(app: &mut VizApp, ni: usize, after: Option<u64>, kind: &str) {
    app.checkpoint("Add cell");
    let id = app.project.next_id();
    let source = match kind {
        "dax" => "EVALUATE\n".to_string(),
        "m" => "let\n    Source = Product\nin\n    Source".to_string(),
        "markdown" => "## Notes\n".to_string(),
        _ => String::new(),
    };
    if let Some(n) = app.project.notebooks.get_mut(ni) {
        let at = after.and_then(|a| n.cells.iter().position(|c| c.id == a)).map(|i| i + 1).unwrap_or(n.cells.len());
        n.cells.insert(at, NotebookCell { id, kind: kind.into(), source });
    }
}

fn cell_ui(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, ni: usize, c: &NotebookCell) -> Option<&'static str> {
    let mut action = None;
    let col = kind_color(&c.kind);
    let editing_md = c.kind == "markdown" && (st.nb.editing_md.contains(&c.id) || c.source.trim().is_empty());
    egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // Header.
        ui.horizontal(|ui| {
            let out_n = st.nb.outputs.get(&c.id).map(|o| o.n);
            if c.kind != "markdown" {
                let (r, resp) = ui.allocate_exact_size(vec2(26.0, 22.0), egui::Sense::click());
                ui.painter().rect_filled(r, 5.0, if resp.hovered() { col } else { col.gamma_multiply(0.15) });
                let cc = r.center();
                ui.painter().add(egui::Shape::convex_polygon(vec![cc + vec2(-3.5, -5.0), cc + vec2(5.0, 0.0), cc + vec2(-3.5, 5.0)], if resp.hovered() { Color32::WHITE } else { col }, egui::Stroke::NONE));
                if resp.on_hover_text("Run (Ctrl+Enter)").clicked() {
                    action = Some("run");
                }
                ui.label(RichText::new(out_n.map(|n| format!("[{n}]")).unwrap_or_else(|| "[ ]".into())).monospace().size(11.0).color(t.muted));
            }
            let mut kind = c.kind.clone();
            egui::ComboBox::from_id_salt(("cell_kind", c.id)).width(92.0).selected_text(RichText::new(KINDS.iter().find(|k| k.0 == c.kind).map(|k| k.1).unwrap_or("Python")).color(col).strong().size(12.0)).show_ui(ui, |ui| {
                for (k, l) in KINDS {
                    ui.selectable_value(&mut kind, k.to_string(), *l);
                }
            });
            if kind != c.kind {
                app.checkpoint("Cell type");
                if let Some(cell) = app.project.notebooks.get_mut(ni).and_then(|n| n.cells.iter_mut().find(|x| x.id == c.id)) {
                    cell.kind = kind;
                }
            }
            if let Some(o) = st.nb.outputs.get(&c.id)
                && o.ms > 0.0
            {
                ui.label(RichText::new(format!("{:.0} ms", o.ms)).size(11.0).color(t.muted));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if super::x_button(ui, t, "Delete cell").clicked() {
                    action = Some("delete");
                }
                let more = ui.add(egui::Button::new(RichText::new("More").size(11.0)).frame(false));
                egui::Popup::menu(&more).show(|ui| {
                    if ui.button("Move up").clicked() {
                        action = Some("up");
                        ui.close();
                    }
                    if ui.button("Move down").clicked() {
                        action = Some("down");
                        ui.close();
                    }
                    if ui.button("Insert Python cell below").clicked() {
                        action = Some("below_python");
                        ui.close();
                    }
                    if ui.button("Insert Markdown cell below").clicked() {
                        action = Some("below_md");
                        ui.close();
                    }
                });
                if c.kind == "markdown" && editing_md && ui.add(egui::Button::new(RichText::new("Done").size(11.0)).frame(false)).clicked() {
                    st.nb.editing_md.remove(&c.id);
                }
            });
        });
        ui.add_space(4.0);
        // Body.
        if c.kind == "markdown" && !editing_md {
            let r = ui.scope(|ui| markdown(ui, t, &c.source)).response;
            let r = ui.interact(r.rect, ui.scope_id().with(("md", c.id)), egui::Sense::click()).on_hover_text("Double-click to edit");
            if r.double_clicked() {
                st.nb.editing_md.insert(c.id);
            }
        } else {
            let mut src = c.source.clone();
            let lines = src.lines().count().max(1);
            let r = ui.add(egui::TextEdit::multiline(&mut src).code_editor().font(egui::FontId::monospace(12.5)).desired_rows(lines.clamp(2, 30)).desired_width(f32::INFINITY).lock_focus(true));
            if r.changed() {
                super::checkpoint_grouped(app, st, &format!("cell{}", c.id), "Edit cell");
                if let Some(cell) = app.project.notebooks.get_mut(ni).and_then(|n| n.cells.iter_mut().find(|x| x.id == c.id)) {
                    cell.source = src;
                }
            }
            if r.has_focus() && ui.input(|i| (i.modifiers.command || i.modifiers.shift) && i.key_pressed(egui::Key::Enter)) {
                if c.kind == "markdown" {
                    st.nb.editing_md.remove(&c.id);
                } else {
                    action = Some("run");
                }
            }
        }
        // Output.
        if let Some(o) = st.nb.outputs.get(&c.id).cloned() {
            if !o.text.trim().is_empty() || o.error.is_some() || o.table.is_some() {
                ui.add_space(6.0);
                ui.separator();
            }
            if !o.text.trim().is_empty() {
                ui.label(RichText::new(o.text.trim_end()).monospace().size(12.0).color(t.text));
            }
            if let Some(e) = &o.error {
                egui::Frame::new().fill(t.danger.gamma_multiply(0.08)).corner_radius(6.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(e.trim_end()).monospace().size(11.5).color(t.danger));
                });
            }
            if let Some(tb) = &o.table {
                ui.label(RichText::new(format!("{} rows × {} columns", tb.rows.len(), tb.columns.len())).size(11.0).color(t.muted));
                super::preview::data_grid(ui, t, tb, ("nb_out", c.id), 280.0);
            }
        }
    });
    action
}

/// A small Markdown renderer: headings, bullets, paragraphs, **bold** and `code`.
fn markdown(ui: &mut Ui, t: &Tokens, src: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    for line in src.lines() {
        let l = line.trim_end();
        if l.trim().is_empty() {
            ui.add_space(4.0);
            continue;
        }
        let (size, strong, body, bullet) = if let Some(x) = l.strip_prefix("### ") {
            (14.0, true, x, false)
        } else if let Some(x) = l.strip_prefix("## ") {
            (16.5, true, x, false)
        } else if let Some(x) = l.strip_prefix("# ") {
            (20.0, true, x, false)
        } else if let Some(x) = l.trim_start().strip_prefix("- ").or_else(|| l.trim_start().strip_prefix("* ")) {
            (13.0, false, x, true)
        } else {
            (13.0, false, l, false)
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if bullet {
                ui.label(RichText::new("  •  ").size(size).color(t.muted));
            }
            for (txt, bold, code) in inline_segments(body) {
                let mut rt = RichText::new(txt).size(size).color(t.text);
                if strong || bold {
                    rt = rt.strong();
                }
                if code {
                    rt = rt.monospace().size(size - 1.0).background_color(t.panel_alt).color(t.accent);
                }
                ui.label(rt);
            }
        });
    }
}

fn inline_segments(s: &str) -> Vec<(String, bool, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut bold = false;
    let mut code = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        if c == '`' {
            out.push((std::mem::take(&mut cur), bold, code));
            code = !code;
            i += 1;
            continue;
        }
        if !code && c == '*' && chars.get(i + 1) == Some(&'*') {
            out.push((std::mem::take(&mut cur), bold, code));
            bold = !bold;
            i += 2;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    out.push((cur, bold, code));
    out.into_iter().filter(|s| !s.0.is_empty()).collect()
}

fn run_cell(app: &mut VizApp, st: &mut FlowsUi, ni: usize, id: u64) {
    let Some(nb) = app.project.notebooks.get(ni) else { return };
    let Some(pos) = nb.cells.iter().position(|c| c.id == id) else { return };
    let Some(cell) = nb.cells.get(pos).cloned() else { return };
    let t0 = std::time::Instant::now();
    let mut out = CellOut::default();
    match cell.kind.as_str() {
        "python" => {
            let prelude: String = nb.cells.iter().take(pos).filter(|c| c.kind == "python").map(|c| c.source.clone()).collect::<Vec<_>>().join("\n\n");
            let inputs: Vec<(String, TableData)> = app.built.tables.iter().map(|(k, v)| (k.clone(), (**v).clone())).collect();
            match flow::run_python(&PyJob { prelude, code: cell.source.clone(), inputs, want_out_df: false, timeout_s: 300 }) {
                Ok(r) => {
                    out.text = r.stdout.clone();
                    if let Some(txt) = &r.result_text {
                        if !out.text.is_empty() && !out.text.ends_with('\n') {
                            out.text.push('\n');
                        }
                        out.text.push_str(txt);
                    }
                    if !r.ok || !r.stderr.trim().is_empty() {
                        let err = r.stderr.trim();
                        // Keep the useful end of a traceback.
                        let lines: Vec<&str> = err.lines().collect();
                        let tail = if lines.len() > 12 { lines.get(lines.len() - 12..).map(|x| x.join("\n")).unwrap_or_default() } else { err.to_string() };
                        if !tail.is_empty() {
                            out.error = Some(tail);
                        }
                    }
                    out.table = r.result_table;
                }
                Err(e) => out.error = Some(e),
            }
        }
        "dax" => match app.built.model.query(&vizardcraft_core::build::substitute_params(&cell.source, &app.project, &app.params())) {
            Ok(res) => {
                let cols: Vec<String> = res.columns.iter().map(|c| c.rsplit_once('[').map(|(_, r)| r.trim_end_matches(']').to_string()).unwrap_or(c.clone())).collect();
                let mut tb = TableData { name: "Result".into(), columns: cols, types: vec![], rows: res.rows.iter().map(|r| r.iter().map(Scalar::from_dm).collect()).collect() };
                tb.infer_types();
                out.table = Some(tb);
            }
            Err(e) => out.error = Some(e.to_string()),
        },
        "m" => {
            let host = vizardcraft_connectors::DesktopHost::default();
            let ph = vizardcraft_core::build::ProjectHost::new(&app.project, &host);
            match vizardcraft_query::evaluate(&cell.source, &ph) {
                Ok(vizardcraft_query::Value::Table(tb)) => out.table = Some(TableData::from_m("Result", &tb)),
                Ok(v) => out.text = v.display(),
                Err(e) => out.error = Some(e.message()),
            }
        }
        _ => {
            st.nb.editing_md.remove(&id);
            return;
        }
    }
    st.nb.counter += 1;
    out.n = st.nb.counter;
    out.ms = t0.elapsed().as_secs_f64() * 1000.0;
    st.nb.outputs.insert(id, out);
}
