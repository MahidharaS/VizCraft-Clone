//! The node inspector: a settings form generated from the node type's schema (columns of the
//! upstream table in drop-downs, numbers, checkboxes, code editors, condition lists…).

use std::collections::HashMap;

use egui::{Color32, RichText, Ui, vec2};
use serde_json::{Value as J, json};
use vizcraft_core::doc::FlowNode;
use vizcraft_core::flow::{self, ParamKind, PortKind, Status};

use super::{FlowsUi, category_color};
use crate::app::VizApp;
use crate::theme::Tokens;

fn as_str(v: &J) -> String {
    match v {
        J::String(s) => s.clone(),
        J::Null => String::new(),
        other => other.to_string(),
    }
}

fn as_list(v: &J) -> Vec<String> {
    match v {
        J::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => vec![],
    }
}

/// Columns arriving at input port `port` of a node (runs the upstream node if needed).
fn input_columns(app: &VizApp, st: &mut FlowsUi, fi: usize, node: u64, port: usize) -> Vec<String> {
    let Some(f) = app.project.flows.get(fi) else { return vec![] };
    let Some(&(from, fport, _, _)) = f.links.iter().find(|l| l.2 == node && l.3 == port) else { return vec![] };
    let host = vizcraft_connectors::DesktopHost::default();
    let ctx = super::host_ctx(app, &host);
    // A Python node upstream is only read from its cache (it runs on demand).
    let python = f.nodes.iter().find(|n| n.id == from).is_some_and(|n| n.kind == "script.python");
    let out = if python { st.executor.cached(&f.name, from).cloned() } else { Some(st.executor.run(&ctx, f, from)) };
    match out {
        Some(Ok(o)) => o.table(fport).map(|t| t.columns.clone()).unwrap_or_default(),
        _ => vec![],
    }
}

fn combo(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, current: &str, options: &[String], width: f32, none_label: Option<&str>) -> Option<String> {
    let mut out = None;
    let shown = if current.is_empty() { none_label.unwrap_or("(choose)").to_string() } else { current.to_string() };
    egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_ui(ui, |ui| {
        if let Some(n) = none_label
            && ui.selectable_label(current.is_empty(), n).clicked()
        {
            out = Some(String::new());
        }
        for o in options {
            if ui.selectable_label(o == current, o).clicked() {
                out = Some(o.clone());
            }
        }
    });
    out
}

pub fn show(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, fi: usize, statuses: &HashMap<u64, Status>) {
    egui::Panel::right("flows_inspector").exact_size(304.0).resizable(false).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(12, 10)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        let Some(f) = app.project.flows.get(fi) else { return };
        let Some(node) = st.selected.and_then(|id| f.nodes.iter().find(|n| n.id == id)).cloned() else {
            flow_info(app, st, ui, t, fi);
            return;
        };
        let Some(nt) = flow::node_type(&node.kind) else {
            ui.label(format!("Unknown node type {}", node.kind));
            return;
        };
        // Header.
        let col = category_color(nt.category);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 9.0, col);
            super::glyphs::paint(ui.painter(), r.shrink(7.0), nt.icon, Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let mut label = node.label.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut label).font(egui::FontId::proportional(14.0)).frame(egui::Frame::NONE).desired_width(220.0));
                if r.changed() {
                    super::checkpoint_grouped(app, st, &format!("label{}", node.id), "Rename node");
                    if let Some(n) = app.project.flows.get_mut(fi).and_then(|f| f.nodes.iter_mut().find(|n| n.id == node.id)) {
                        n.label = label;
                    }
                }
                ui.label(RichText::new(format!("{} · {}", nt.label, nt.category)).size(11.0).color(col));
            });
        });
        ui.add_space(4.0);
        ui.label(RichText::new(nt.help).size(11.5).color(t.muted));
        ui.add_space(6.0);
        // Status and actions.
        let status = statuses.get(&node.id).copied().unwrap_or(Status::Stale);
        let fname = app.project.flows.get(fi).map(|f| f.name.clone()).unwrap_or_default();
        match (status, st.executor.cached(&fname, node.id)) {
            (Status::Error, Some(Err(e))) => {
                egui::Frame::new().fill(t.danger.gamma_multiply(0.08)).corner_radius(6.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(e).size(11.5).color(t.danger));
                });
            }
            (Status::Ok, Some(Ok(o))) => {
                ui.label(RichText::new(format!("Up to date · ran in {:.0} ms", o.ms)).size(11.5).color(t.good));
            }
            _ => {
                ui.label(RichText::new(if node.kind == "script.python" { "Not run: Python runs when you press Run." } else { "Not run with the current settings." }).size(11.5).color(t.muted));
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Run").color(Color32::WHITE)).fill(t.accent)).on_hover_text("Run this node (Ctrl+Enter in a script)").clicked() {
                st.run_request = Some(super::RunRequest::Node(node.id));
            }
            if nt.outputs.first().is_some_and(|p| p.kind == PortKind::Table) {
                match super::in_model(app, fi, node.id) {
                    Some(name) => {
                        ui.label(RichText::new(format!("In model: {name}")).size(11.5).color(t.good));
                    }
                    None => {
                        if ui.button("Add to model").on_hover_text("Make this output a table of the semantic model").clicked() {
                            super::add_to_model(app, fi, node.id);
                        }
                    }
                }
            }
        });
        ui.add_space(6.0);
        ui.separator();
        // Settings form.
        let mut changes: Vec<(String, J)> = Vec::new();
        let mut relabel: Option<String> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if nt.params.is_empty() {
                ui.label(RichText::new("This node has no settings.").color(t.muted));
            }
            for p in &nt.params {
                ui.add_space(6.0);
                ui.label(RichText::new(p.label).size(12.0).strong().color(t.text));
                let cur = flow::setting(&node, p.key);
                let key = p.key.to_string();
                let id = (node.id, p.key);
                match &p.kind {
                    ParamKind::Text => {
                        let mut s = as_str(&cur);
                        if ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY)).changed() {
                            changes.push((key, json!(s)));
                        }
                    }
                    ParamKind::Code(lang) => {
                        let mut s = as_str(&cur);
                        let rows = if *lang == "python" { 14 } else { 3 };
                        let r = ui.add(egui::TextEdit::multiline(&mut s).code_editor().desired_rows(rows).desired_width(f32::INFINITY).font(egui::FontId::monospace(12.0)));
                        if r.changed() {
                            changes.push((key, json!(s)));
                        }
                        if r.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
                            st.run_request = Some(super::RunRequest::Node(node.id));
                        }
                        if *lang == "formula" {
                            let cols = input_columns(app, st, fi, node.id, 0);
                            if !cols.is_empty() {
                                ui.label(RichText::new("Insert a column:").size(10.5).color(t.muted));
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                                    for c in cols.iter().take(40) {
                                        if ui.add(egui::Button::new(RichText::new(format!("[{c}]")).size(11.0).monospace()).small()).clicked() {
                                            let mut s2 = as_str(&cur);
                                            s2.push_str(&format!("[{c}]"));
                                            changes.push((p.key.to_string(), json!(s2)));
                                        }
                                    }
                                });
                            }
                        }
                    }
                    ParamKind::Number { min, max, step } => {
                        let mut v = cur.as_f64().or_else(|| as_str(&cur).parse().ok()).unwrap_or(0.0);
                        let int = *step >= 1.0;
                        let dv = egui::DragValue::new(&mut v).range(*min..=*max).speed((*step).max(0.001) * if int { 0.2 } else { 1.0 });
                        let dv = if int { dv.fixed_decimals(0) } else { dv.max_decimals(4) };
                        if ui.add(dv).changed() {
                            changes.push((key, if int { json!(v.round() as i64) } else { json!(v) }));
                        }
                    }
                    ParamKind::Bool => {
                        let mut b = cur.as_bool().unwrap_or(false);
                        if ui.checkbox(&mut b, RichText::new("On").size(12.0)).changed() {
                            changes.push((key, json!(b)));
                        }
                    }
                    ParamKind::Choice(opts) => {
                        let c = as_str(&cur);
                        let shown = opts.iter().find(|o| o.0 == c).map(|o| o.1).unwrap_or(c.as_str()).to_string();
                        egui::ComboBox::from_id_salt(id).width(ui.available_width() - 8.0).selected_text(shown).show_ui(ui, |ui| {
                            for (k, l) in *opts {
                                if ui.selectable_label(*k == c, *l).clicked() {
                                    changes.push((key.clone(), json!(k)));
                                }
                            }
                        });
                    }
                    ParamKind::Column(port) => {
                        let cols = input_columns(app, st, fi, node.id, *port);
                        let c = as_str(&cur);
                        if cols.is_empty() {
                            ui.label(RichText::new("Connect and run the input to choose a column.").size(11.0).color(t.muted));
                        }
                        if let Some(v) = combo(ui, id, &c, &cols, ui.available_width() - 8.0, Some("(none)")) {
                            changes.push((key, json!(v)));
                        }
                    }
                    ParamKind::Columns(port) => {
                        let cols = input_columns(app, st, fi, node.id, *port);
                        let mut chosen = as_list(&cur);
                        if cols.is_empty() {
                            ui.label(RichText::new("Connect and run the input to choose columns.").size(11.0).color(t.muted));
                        } else {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(if chosen.is_empty() { "None chosen: automatic".to_string() } else { format!("{} chosen", chosen.len()) }).size(11.0).color(t.muted));
                                if ui.small_button("All").clicked() {
                                    changes.push((key.clone(), json!(cols.clone())));
                                }
                                if ui.small_button("None").clicked() {
                                    changes.push((key.clone(), json!([])));
                                }
                            });
                            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(5.0).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                egui::ScrollArea::vertical().id_salt(("cols", node.id, p.key)).max_height(150.0).show(ui, |ui| {
                                    for c in &cols {
                                        let mut on = chosen.iter().any(|x| x.eq_ignore_ascii_case(c));
                                        if ui.checkbox(&mut on, RichText::new(c).size(12.0)).changed() {
                                            if on {
                                                chosen.push(c.clone());
                                            } else {
                                                chosen.retain(|x| !x.eq_ignore_ascii_case(c));
                                            }
                                            // Keep the input order.
                                            let ordered: Vec<String> = cols.iter().filter(|x| chosen.iter().any(|y| y.eq_ignore_ascii_case(x))).cloned().collect();
                                            changes.push((key.clone(), json!(ordered)));
                                        }
                                    }
                                });
                            });
                        }
                    }
                    ParamKind::Query | ParamKind::Sheet | ParamKind::ModelTable => {
                        let names: Vec<String> = match p.kind {
                            ParamKind::Query => app.project.queries.queries.iter().map(|q| q.name.clone()).collect(),
                            ParamKind::Sheet => app.project.sheets.iter().map(|s| s.name.clone()).collect(),
                            _ => app.project.model.tables.iter().map(|t| t.name.clone()).collect(),
                        };
                        let c = as_str(&cur);
                        if let Some(v) = combo(ui, id, &c, &names, ui.available_width() - 8.0, None) {
                            if node.label == c || node.label == nt.label {
                                relabel = Some(v.clone());
                            }
                            changes.push((key, json!(v)));
                        }
                    }
                    ParamKind::FileOpen | ParamKind::FileSave => {
                        let mut s = as_str(&cur);
                        ui.horizontal(|ui| {
                            if ui.add(egui::TextEdit::singleline(&mut s).desired_width(ui.available_width() - 76.0).hint_text("path")).changed() {
                                changes.push((key.clone(), json!(s.clone())));
                            }
                            if ui.button("Browse…").clicked() {
                                let dlg = rfd::FileDialog::new().add_filter("CSV / text", &["csv", "tsv", "txt"]);
                                let pick = if p.kind == ParamKind::FileOpen { dlg.pick_file() } else { dlg.set_file_name(format!("{}.csv", node.label)).save_file() };
                                if let Some(path) = pick {
                                    changes.push((key.clone(), json!(path.display().to_string())));
                                }
                            }
                        });
                    }
                    ParamKind::Conditions => {
                        let cols = input_columns(app, st, fi, node.id, 0);
                        let mut list: Vec<serde_json::Map<String, J>> = match &cur {
                            J::Array(a) => a.iter().filter_map(|v| v.as_object().cloned()).collect(),
                            _ => vec![],
                        };
                        let mut changed = false;
                        let mut remove = None;
                        for (i, c) in list.iter_mut().enumerate() {
                            egui::Frame::new().fill(t.panel_alt).corner_radius(6.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let col = c.get("column").map(as_str).unwrap_or_default();
                                    if let Some(v) = combo(ui, (id, i, "c"), &col, &cols, ui.available_width() - 34.0, None) {
                                        c.insert("column".into(), json!(v));
                                        changed = true;
                                    }
                                    if super::x_button(ui, t, "Remove").clicked() {
                                        remove = Some(i);
                                    }
                                });
                                ui.horizontal(|ui| {
                                    let op = c.get("op").map(as_str).unwrap_or_else(|| "=".into());
                                    let shown = flow::FILTER_OPS.iter().find(|o| o.0 == op).map(|o| o.1).unwrap_or("equals");
                                    egui::ComboBox::from_id_salt((id, i, "op")).width(118.0).selected_text(shown).show_ui(ui, |ui| {
                                        for (k, l) in flow::FILTER_OPS {
                                            if ui.selectable_label(*k == op, *l).clicked() {
                                                c.insert("op".into(), json!(k));
                                                changed = true;
                                            }
                                        }
                                    });
                                    if !matches!(op.as_str(), "blank" | "not_blank") {
                                        let mut v = c.get("value").map(as_str).unwrap_or_default();
                                        if ui.add(egui::TextEdit::singleline(&mut v).desired_width(ui.available_width()).hint_text("value")).changed() {
                                            c.insert("value".into(), json!(v));
                                            changed = true;
                                        }
                                    }
                                });
                            });
                        }
                        if let Some(i) = remove {
                            list.remove(i);
                            changed = true;
                        }
                        if ui.button("+ Add condition").clicked() {
                            let mut m = serde_json::Map::new();
                            m.insert("column".into(), json!(cols.first().cloned().unwrap_or_default()));
                            m.insert("op".into(), json!("="));
                            m.insert("value".into(), json!(""));
                            list.push(m);
                            changed = true;
                        }
                        if changed {
                            changes.push((key, J::Array(list.into_iter().map(J::Object).collect())));
                        }
                    }
                    ParamKind::Aggregates => {
                        let cols = input_columns(app, st, fi, node.id, 0);
                        let mut list: Vec<serde_json::Map<String, J>> = match &cur {
                            J::Array(a) => a.iter().filter_map(|v| v.as_object().cloned()).collect(),
                            _ => vec![],
                        };
                        let mut changed = false;
                        let mut remove = None;
                        for (i, c) in list.iter_mut().enumerate() {
                            egui::Frame::new().fill(t.panel_alt).corner_radius(6.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let op = c.get("op").map(as_str).unwrap_or_else(|| "sum".into());
                                    let shown = flow::AGGREGATIONS.iter().find(|o| o.0 == op).map(|o| o.1).unwrap_or("Sum");
                                    egui::ComboBox::from_id_salt((id, i, "op")).width(96.0).selected_text(shown).show_ui(ui, |ui| {
                                        for (k, l) in flow::AGGREGATIONS {
                                            if ui.selectable_label(*k == op, *l).clicked() {
                                                c.insert("op".into(), json!(k));
                                                changed = true;
                                            }
                                        }
                                    });
                                    let col = c.get("column").map(as_str).unwrap_or_default();
                                    if let Some(v) = combo(ui, (id, i, "c"), &col, &cols, ui.available_width() - 34.0, if op == "count" { Some("(rows)") } else { None }) {
                                        c.insert("column".into(), json!(v));
                                        changed = true;
                                    }
                                    if super::x_button(ui, t, "Remove").clicked() {
                                        remove = Some(i);
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("as").size(11.0).color(t.muted));
                                    let mut v = c.get("name").map(as_str).unwrap_or_default();
                                    if ui.add(egui::TextEdit::singleline(&mut v).desired_width(ui.available_width()).hint_text("output name (optional)")).changed() {
                                        c.insert("name".into(), json!(v));
                                        changed = true;
                                    }
                                });
                            });
                        }
                        if let Some(i) = remove {
                            list.remove(i);
                            changed = true;
                        }
                        if ui.button("+ Add aggregation").clicked() {
                            let mut m = serde_json::Map::new();
                            m.insert("column".into(), json!(cols.iter().rev().find(|_| true).cloned().unwrap_or_default()));
                            m.insert("op".into(), json!("sum"));
                            m.insert("name".into(), json!(""));
                            list.push(m);
                            changed = true;
                        }
                        if changed {
                            changes.push((key, J::Array(list.into_iter().map(J::Object).collect())));
                        }
                    }
                    ParamKind::SortKeys => {
                        let cols = input_columns(app, st, fi, node.id, 0);
                        let mut list: Vec<serde_json::Map<String, J>> = match &cur {
                            J::Array(a) => a.iter().filter_map(|v| v.as_object().cloned()).collect(),
                            _ => vec![],
                        };
                        let mut changed = false;
                        let mut remove = None;
                        for (i, c) in list.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let col = c.get("column").map(as_str).unwrap_or_default();
                                if let Some(v) = combo(ui, (id, i, "c"), &col, &cols, ui.available_width() - 104.0, None) {
                                    c.insert("column".into(), json!(v));
                                    changed = true;
                                }
                                let mut desc = c.get("desc").and_then(J::as_bool).unwrap_or(false);
                                if ui.selectable_label(!desc, "Asc").clicked() {
                                    desc = false;
                                    c.insert("desc".into(), json!(desc));
                                    changed = true;
                                }
                                if ui.selectable_label(desc, "Desc").clicked() {
                                    c.insert("desc".into(), json!(true));
                                    changed = true;
                                }
                                if super::x_button(ui, t, "Remove").clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = remove {
                            list.remove(i);
                            changed = true;
                        }
                        if ui.button("+ Add sort column").clicked() {
                            let mut m = serde_json::Map::new();
                            m.insert("column".into(), json!(cols.first().cloned().unwrap_or_default()));
                            m.insert("desc".into(), json!(false));
                            list.push(m);
                            changed = true;
                        }
                        if changed {
                            changes.push((key, J::Array(list.into_iter().map(J::Object).collect())));
                        }
                    }
                }
                if !p.help.is_empty() {
                    ui.label(RichText::new(p.help).size(10.5).color(t.muted));
                }
            }
            ui.add_space(12.0);
            // Ports summary.
            ui.separator();
            ui.label(RichText::new("Ports").size(11.0).strong().color(t.muted));
            for (dir, ports) in [("In", &nt.inputs), ("Out", &nt.outputs)] {
                for pt in ports {
                    ui.label(RichText::new(format!("{dir}: {} ({}{})", pt.name, if pt.kind == PortKind::Model { "model" } else { "table" }, if pt.optional { ", optional" } else { "" })).size(11.0).color(t.muted));
                }
            }
        });
        if !changes.is_empty() {
            let keyname = format!("cfg{}{}", node.id, changes.first().map(|c| c.0.clone()).unwrap_or_default());
            super::checkpoint_grouped(app, st, &keyname, "Edit node settings");
            if let Some(n) = app.project.flows.get_mut(fi).and_then(|f| f.nodes.iter_mut().find(|n| n.id == node.id)) {
                apply_changes(n, changes, relabel);
            }
        }
    });
}

fn apply_changes(n: &mut FlowNode, changes: Vec<(String, J)>, relabel: Option<String>) {
    for (k, v) in changes {
        n.config.insert(k, v);
    }
    if let Some(l) = relabel {
        n.label = l;
    }
}

fn flow_info(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, fi: usize) {
    let Some(f) = app.project.flows.get(fi).cloned() else { return };
    ui.label(RichText::new(&f.name).strong().size(15.0).color(t.text));
    ui.add_space(2.0);
    ui.label(RichText::new(format!("{} nodes · {} connections", f.nodes.len(), f.links.len())).size(11.5).color(t.muted));
    ui.add_space(8.0);
    ui.label(RichText::new("Description").size(12.0).strong());
    let mut d = f.description.clone();
    if ui.add(egui::TextEdit::multiline(&mut d).desired_rows(3).desired_width(f32::INFINITY)).changed() {
        super::checkpoint_grouped(app, st, &format!("desc{fi}"), "Flow description");
        if let Some(f) = app.project.flows.get_mut(fi) {
            f.description = d;
        }
    }
    ui.add_space(12.0);
    ui.label(RichText::new("Select a node to edit its settings and preview its output.").color(t.muted));
    ui.add_space(10.0);
    ui.label(RichText::new("Tips").size(12.0).strong());
    for tip in [
        "Drag from an output port (right) to an input port (left) to connect.",
        "Drag a connected input to move or remove its wire; hover a wire to delete it.",
        "Click a palette node to add it after the selected node.",
        "Scroll to pan, Ctrl+scroll to zoom, double-click a node to run it.",
        "Save as model table → Add to model makes the output a table for reports.",
    ] {
        ui.label(RichText::new(format!("• {tip}")).size(11.5).color(t.muted));
    }
    let tables: Vec<String> = app.project.model.tables.iter().filter(|t| matches!(&t.source, vizcraft_core::doc::TableSource::Flow(s) if flow::parse_spec(s).0 == f.name)).map(|t| t.name.clone()).collect();
    if !tables.is_empty() {
        ui.add_space(10.0);
        ui.label(RichText::new("Model tables from this flow").size(12.0).strong());
        for tb in tables {
            ui.label(RichText::new(format!("• {tb}")).size(11.5).color(t.good));
        }
    }
}
