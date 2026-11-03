//! TMDL view: the semantic model as a TMDL script in tabs, with syntax highlighting, live parse
//! checks, and Apply (with a preview of the changes and a diff) that writes edits back to the
//! model. "Script TMDL" from the Data pane opens one object in a new tab.

use egui::{Color32, RichText, Sense, Ui, vec2};

use super::editor::{self, Lang};
use super::{PaneMode, data_pane, glyph_icon, icon_btn, run_act, strip_group};
use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets::big_button;

pub struct TmdlTab {
    name: String,
    text: String,
    /// The script as generated, for the diff.
    base: String,
    id: u64,
    /// The whole model (regenerated when the model changes and the tab isn't edited).
    whole: bool,
}

struct Preview {
    log: Result<Vec<String>, String>,
    diff: Vec<(usize, char, String)>,
}

#[derive(Default)]
pub struct TmdlState {
    tabs: Vec<TmdlTab>,
    active: usize,
    next_id: u64,
    preview: Option<Preview>,
    generation: u64,
    parse: Option<(String, Result<usize, String>)>,
}

impl TmdlState {
    /// Opens a new tab with a script.
    pub fn open_tab(&mut self, name: &str, text: String) {
        self.next_id += 1;
        let name = if self.tabs.iter().any(|t| t.name == name) { (2..).map(|i| format!("{name} ({i})")).find(|n| !self.tabs.iter().any(|t| &t.name == n)).unwrap_or(name.to_string()) } else { name.to_string() };
        self.tabs.push(TmdlTab { name, base: text.clone(), text, id: self.next_id, whole: false });
        self.active = self.tabs.len() - 1;
    }
}

fn model_script(app: &VizApp) -> String {
    format!("/// {} — the whole semantic model. Edit it and click Apply.\n{}", app.project.name, vizcraft_core::tmdl::write(&app.project, &app.built))
}

fn ensure(app: &mut VizApp) {
    let gen_now = app.generation;
    let script = if app.model_ui.tmdl.tabs.is_empty() || app.model_ui.tmdl.generation != gen_now { Some(model_script(app)) } else { None };
    let st = &mut app.model_ui.tmdl;
    if st.tabs.is_empty() {
        st.next_id += 1;
        let text = script.clone().unwrap_or_default();
        st.tabs.push(TmdlTab { name: "Model".into(), base: text.clone(), text, id: st.next_id, whole: true });
    }
    if st.generation != gen_now {
        st.generation = gen_now;
        // Keep unedited whole-model tabs in step with the model.
        if let Some(s) = script {
            for tab in st.tabs.iter_mut().filter(|t| t.whole && t.text == t.base) {
                tab.text = s.clone();
                tab.base = s.clone();
            }
        }
    }
    st.active = st.active.min(st.tabs.len().saturating_sub(1));
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    ensure(app);
    egui::Panel::top("tmdl_tools").frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 2 }).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| toolbar(app, ui, &t));
    });
    let mut acts = Vec::new();
    egui::Panel::right("tmdl_side").default_size(260.0).size_range(200.0..=440.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        acts = data_pane(app, ui, PaneMode::Tmdl);
        ui.label(RichText::new("Double-click or right-click ▸ Script TMDL to open an object in a new tab").size(10.5).color(t.muted));
    });
    egui::Panel::top("tmdl_tabs").exact_size(32.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(8, 3)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| tabs(app, ui, &t));
    egui::Panel::bottom("tmdl_status").exact_size(28.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(10, 4)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| status(app, ui, &t));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(8))).show(ui, |ui| {
        let measures: Vec<String> = app.built.model.measures.iter().map(|m| m.name.clone()).collect();
        let active = app.model_ui.tmdl.active;
        let h = ui.available_height() - 4.0;
        let mut apply = false;
        if let Some(tab) = app.model_ui.tmdl.tabs.get_mut(active) {
            let out = editor::code_editor(ui, egui::Id::unique(("tmdl_editor", tab.id)), &mut tab.text, Lang::Tmdl, &t, &measures, h, "");
            apply = out.run;
        }
        if apply {
            preview(app);
        }
    });
    preview_window(app, ui.ctx(), &t);
    for a in acts {
        run_act(app, a);
    }
}

fn toolbar(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    strip_group(ui, t, "Script", |ui| {
        if big_button(ui, t, "tmdl", "Apply", "Preview and apply the script's changes to the model (Ctrl+Enter)").clicked() {
            preview(app);
        }
        if big_button(ui, t, "refresh", "Revert", "Discard edits: regenerate this tab from the model").clicked() {
            revert(app);
        }
    });
    strip_group(ui, t, "Script TMDL", |ui| {
        if big_button(ui, t, "model", "Whole model", "Open the whole model in a new tab").clicked() {
            let s = model_script(app);
            app.model_ui.tmdl.open_tab("Model", s);
            if let Some(tab) = app.model_ui.tmdl.tabs.last_mut() {
                tab.whole = true;
            }
        }
        if big_button(ui, t, "relationship", "Relationships", "Script every relationship").clicked() {
            let mut s = "createOrReplace\n".to_string();
            for r in &app.project.model.relationships {
                let one = vizcraft_core::tmdl::write_object(&app.project, &app.built, &vizcraft_core::tmdl::TmdlObject::Relationship(r.id));
                s += one.trim_start_matches("createOrReplace\n");
                s.push('\n');
            }
            app.model_ui.tmdl.open_tab("Relationships", s.trim_end().to_string());
        }
        if big_button(ui, t, "security", "Roles", "Script every role").clicked() {
            let mut s = "createOrReplace\n".to_string();
            for r in &app.project.model.roles {
                let one = vizcraft_core::tmdl::write_object(&app.project, &app.built, &vizcraft_core::tmdl::TmdlObject::Role(r.name.clone()));
                s += one.trim_start_matches("createOrReplace\n");
                s.push('\n');
            }
            if app.project.model.roles.is_empty() {
                s += "\n\trole 'New role'\n\t\tmodelPermission: read\n\t\ttablePermission Store = [Region] = \"West\"\n";
            }
            app.model_ui.tmdl.open_tab("Roles", s.trim_end().to_string());
        }
        if big_button(ui, t, "parameter", "Expressions", "Power Query parameters and connection-only queries").clicked() {
            let s = vizcraft_core::tmdl::write_object(&app.project, &app.built, &vizcraft_core::tmdl::TmdlObject::Expressions);
            app.model_ui.tmdl.open_tab("Expressions", s);
        }
    });
    strip_group(ui, t, "Help", |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("Indent with tabs (or 4 spaces). Objects: table, measure, column,").size(11.0).color(t.muted));
            ui.label(RichText::new("hierarchy, partition, relationship, role, expression. Multi-line").size(11.0).color(t.muted));
            ui.label(RichText::new("DAX goes between ``` fences. createOrReplace scripts never delete.").size(11.0).color(t.muted));
        });
    });
}

fn revert(app: &mut VizApp) {
    let active = app.model_ui.tmdl.active;
    let whole = app.model_ui.tmdl.tabs.get(active).is_some_and(|t| t.whole);
    let script = if whole { Some(model_script(app)) } else { None };
    if let Some(tab) = app.model_ui.tmdl.tabs.get_mut(active) {
        tab.text = script.unwrap_or_else(|| tab.base.clone());
        tab.base = tab.text.clone();
    }
}

fn preview(app: &mut VizApp) {
    let active = app.model_ui.tmdl.active;
    let Some(tab) = app.model_ui.tmdl.tabs.get(active) else { return };
    let text = tab.text.clone();
    let base = tab.base.clone();
    let mut copy = app.project.clone();
    let log = vizcraft_core::tmdl::apply_with(&mut copy, Some(&app.built), &text);
    let diff = vizcraft_core::tmdl::diff_lines(&base, &text);
    app.model_ui.tmdl.preview = Some(Preview { log, diff });
}

fn preview_window(app: &mut VizApp, ctx: &egui::Context, t: &Tokens) {
    let Some(p) = &app.model_ui.tmdl.preview else { return };
    let mut open = true;
    let mut apply = false;
    let mut cancel = false;
    egui::Window::new("Apply TMDL changes").collapsible(false).resizable(true).default_width(640.0).pivot(egui::Align2::CENTER_TOP).default_pos(egui::pos2(800.0, 120.0)).open(&mut open).show(ctx, |ui| {
        match &p.log {
            Err(e) => {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    glyph_icon(ui.painter(), r, "warn", t.danger);
                    ui.label(RichText::new("The script has errors; nothing was changed.").strong().color(t.danger));
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for l in e.lines() {
                        ui.label(RichText::new(l).monospace().size(12.0));
                    }
                });
            }
            Ok(log) if log.is_empty() => {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    glyph_icon(ui.painter(), r, "check", t.good);
                    ui.label(RichText::new("No changes: the script matches the model.").strong());
                });
            }
            Ok(log) => {
                ui.label(RichText::new(format!("{} change{} to apply", log.len(), if log.len() == 1 { "" } else { "s" })).strong().size(14.0));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().id_salt("tmdl_log").max_height(220.0).show(ui, |ui| {
                    for l in log {
                        let (mark, color) = if l.starts_with("Created") {
                            ("+", t.good)
                        } else if l.starts_with("Deleted") {
                            ("−", t.danger)
                        } else {
                            ("~", t.accent)
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(mark).monospace().strong().color(color));
                            ui.label(RichText::new(l).size(12.5));
                        });
                    }
                });
            }
        }
        if !p.diff.is_empty() {
            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new(format!("Script differences ({} lines)", p.diff.len())).size(12.0)).id_salt("tmdl_diff").default_open(p.diff.len() <= 40).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("tmdl_diff_s").max_height(240.0).show(ui, |ui| {
                    for (line, k, text) in p.diff.iter().take(400) {
                        let (bg, fg) = if *k == '+' { (t.good.gamma_multiply(0.12), t.good) } else { (t.danger.gamma_multiply(0.12), t.danger) };
                        egui::Frame::new().fill(bg).inner_margin(egui::Margin::symmetric(4, 0)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(format!("{line:>5} {k} {}", text.replace('\t', "    "))).monospace().size(11.5).color(fg));
                        });
                    }
                });
            });
        }
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can = matches!(&p.log, Ok(l) if !l.is_empty());
            if ui.add_enabled(can, egui::Button::new(RichText::new("Apply changes").color(Color32::WHITE).strong()).fill(t.accent).min_size(vec2(110.0, 26.0))).clicked() {
                apply = true;
            }
            if ui.add(egui::Button::new("Close").min_size(vec2(76.0, 26.0))).clicked() {
                cancel = true;
            }
        });
    });
    if apply {
        let active = app.model_ui.tmdl.active;
        let text = app.model_ui.tmdl.tabs.get(active).map(|t| t.text.clone()).unwrap_or_default();
        app.checkpoint("Apply TMDL");
        let built = app.built.clone();
        match vizcraft_core::tmdl::apply_with(&mut app.project, Some(&built), &text) {
            Ok(log) => {
                let data = log.iter().any(|l| l.contains("Power Query") || l.contains("data type") || l.contains("Created table"));
                if data {
                    app.mark_data_dirty();
                } else {
                    app.mark_model_dirty();
                }
                if let Some(tab) = app.model_ui.tmdl.tabs.get_mut(active) {
                    tab.base = tab.text.clone();
                }
                app.toast(format!("Applied {} change{}", log.len(), if log.len() == 1 { "" } else { "s" }));
            }
            Err(e) => app.error(e),
        }
        app.model_ui.tmdl.preview = None;
        return;
    }
    if !open || cancel {
        app.model_ui.tmdl.preview = None;
    }
}

fn status(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let active = app.model_ui.tmdl.active;
    let Some(tab) = app.model_ui.tmdl.tabs.get(active) else { return };
    let text = tab.text.clone();
    let edited = tab.text != tab.base;
    let st = &mut app.model_ui.tmdl;
    if st.parse.as_ref().is_none_or(|(s, _)| *s != text) {
        let r = vizcraft_core::tmdl::parse(&text).map(|(n, _)| n.len());
        st.parse = Some((text.clone(), r));
    }
    ui.horizontal(|ui| {
        match st.parse.as_ref().map(|(_, r)| r) {
            Some(Ok(n)) => {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                glyph_icon(ui.painter(), r, "check", t.good);
                ui.label(RichText::new(format!("Script is valid · {n} top-level objects")).size(11.5).color(t.muted));
            }
            Some(Err(e)) => {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                glyph_icon(ui.painter(), r, "warn", t.danger);
                ui.label(RichText::new(e).size(11.5).color(t.danger));
            }
            None => {}
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{} lines", text.lines().count())).size(11.5).color(t.muted));
            if edited {
                ui.label(RichText::new("Edited · Apply to update the model").size(11.5).color(t.accent));
            }
        });
    });
}

fn tabs(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal_centered(|ui| {
        let n = app.model_ui.tmdl.tabs.len();
        let mut close = None;
        for i in 0..n {
            let active = app.model_ui.tmdl.active == i;
            let Some(tab) = app.model_ui.tmdl.tabs.get(i) else { continue };
            let name = tab.name.clone();
            let edited = tab.text != tab.base;
            egui::Frame::new().fill(if active { t.panel } else { Color32::TRANSPARENT }).corner_radius(4.0).stroke(if active { egui::Stroke::new(1.0, t.border) } else { egui::Stroke::NONE }).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let r = ui.add(egui::Label::new(RichText::new(&name).size(12.0).color(if active { t.accent } else { t.text })).sense(Sense::click()));
                    if edited {
                        let (rr, _) = ui.allocate_exact_size(vec2(10.0, 12.0), Sense::hover());
                        ui.painter().circle_filled(rr.center(), 3.0, t.accent);
                    }
                    if r.clicked() {
                        app.model_ui.tmdl.active = i;
                    }
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
        if icon_btn(ui, t, "plus", "New script tab (whole model)", Some(t.accent)).clicked() {
            let s = model_script(app);
            app.model_ui.tmdl.open_tab("Model", s);
            if let Some(tab) = app.model_ui.tmdl.tabs.last_mut() {
                tab.whole = true;
            }
        }
        if let Some(i) = close {
            let st = &mut app.model_ui.tmdl;
            st.tabs.remove(i);
            st.active = st.active.min(st.tabs.len().saturating_sub(1));
        }
    });
}
