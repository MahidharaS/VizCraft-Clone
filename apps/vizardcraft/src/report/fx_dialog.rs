//! The fx dialog: bind any property to data — a template with live values, a DAX expression,
//! rules, a gradient, or a field value. This is what lets titles, colours, sizes and any
//! other setting change dynamically with the numbers.

use egui::{RichText, Ui};
use vizardcraft_core::color::Color;
use vizardcraft_core::doc::*;
use vizardcraft_core::format::PropKind;

use crate::app::VizApp;
use crate::widgets;

#[derive(Clone)]
pub struct FxDialog {
    pub element: u64,
    pub key: String,
    pub kind: PropKind,
    pub mode: usize,
    pub template: String,
    pub dax: String,
    pub rules: Vec<FxRule>,
    pub default: Prop,
    pub min: String,
    pub max: String,
    pub min_color: Color,
    pub mid: bool,
    pub mid_color: Color,
    pub max_color: Color,
    pub preview: String,
}

pub fn summary(fx: &Fx) -> String {
    match fx {
        Fx::Template { text } => vizardcraft_core::scene::ellipsize(text, 11.0, 90.0),
        Fx::Expression { dax } => vizardcraft_core::scene::ellipsize(dax, 11.0, 90.0),
        Fx::Rules { rules, .. } => format!("{} rules", rules.len()),
        Fx::Gradient { .. } => "gradient".into(),
        Fx::FieldValue { dax } => format!("field {dax}"),
    }
}

pub fn open(app: &mut VizApp, element: u64, key: &str, kind: &PropKind, cur: Option<Fx>) {
    open_owned(app, element, key.to_string(), kind.clone(), cur);
}

pub fn open_owned(app: &mut VizApp, element: u64, key: String, kind: PropKind, cur: Option<Fx>) {
    let first_measure = app.project.model.measures.first().map(|m| format!("[{}]", m.name)).unwrap_or_default();
    let mut d = FxDialog {
        element,
        key,
        kind: kind.clone(),
        mode: if matches!(kind, PropKind::Color) { 2 } else if matches!(kind, PropKind::Template | PropKind::Text) { 0 } else { 1 },
        template: String::new(),
        dax: first_measure,
        rules: vec![],
        default: Prop::Color(Color::rgb(0x9C, 0xA3, 0xAF)),
        min: String::new(),
        max: String::new(),
        min_color: Color::rgb(0xFE, 0xE2, 0xE2),
        mid: false,
        mid_color: Color::rgb(0xFE, 0xF9, 0xC3),
        max_color: Color::rgb(0x16, 0xA3, 0x4A),
        preview: String::new(),
    };
    if matches!(kind, PropKind::Color) {
        d.rules = vec![
            FxRule { op: "<".into(), value: 0.0, value2: 0.0, text: String::new(), percent: false, output: Prop::Color(Color::rgb(0xDC, 0x26, 0x26)) },
            FxRule { op: ">=".into(), value: 0.0, value2: 0.0, text: String::new(), percent: false, output: Prop::Color(Color::rgb(0x16, 0xA3, 0x4A)) },
        ];
    }
    match cur {
        Some(Fx::Template { text }) => {
            d.mode = 0;
            d.template = text;
        }
        Some(Fx::Expression { dax }) => {
            d.mode = 1;
            d.dax = dax;
        }
        Some(Fx::Rules { dax, rules, default }) => {
            d.mode = 2;
            d.dax = dax;
            d.rules = rules;
            d.default = *default;
        }
        Some(Fx::Gradient { dax, min, max, min_color, mid_color, max_color }) => {
            d.mode = 3;
            d.dax = dax;
            d.min = min.map(|x| x.to_string()).unwrap_or_default();
            d.max = max.map(|x| x.to_string()).unwrap_or_default();
            d.min_color = min_color;
            d.mid = mid_color.is_some();
            d.mid_color = mid_color.unwrap_or(d.mid_color);
            d.max_color = max_color;
        }
        Some(Fx::FieldValue { dax }) => {
            d.mode = 4;
            d.dax = dax;
        }
        None => {}
    }
    app.report.fx = Some(d);
}

fn build(d: &FxDialog) -> Fx {
    match d.mode {
        0 => Fx::Template { text: d.template.clone() },
        1 => Fx::Expression { dax: d.dax.clone() },
        2 => Fx::Rules { dax: d.dax.clone(), rules: d.rules.clone(), default: Box::new(d.default.clone()) },
        3 => Fx::Gradient {
            dax: d.dax.clone(),
            min: d.min.trim().parse().ok(),
            max: d.max.trim().parse().ok(),
            min_color: d.min_color,
            mid_color: if d.mid { Some(d.mid_color) } else { None },
            max_color: d.max_color,
        },
        _ => Fx::FieldValue { dax: d.dax.clone() },
    }
}

pub fn show(app: &mut VizApp, ctx: &egui::Context) {
    let Some(mut d) = app.report.fx.clone() else { return };
    let mut open = true;
    let mut apply = false;
    let measures: Vec<String> = app.project.model.measures.iter().map(|m| format!("[{}]", m.name)).collect();
    egui::Window::new(format!("ƒx  {}", d.key)).open(&mut open).resizable(true).default_width(520.0).collapsible(false).show(ctx, |ui| {
        ui.label(RichText::new("Bind this property to data. It recalculates with every filter, slicer and selection.").size(11.0));
        ui.horizontal(|ui| {
            for (i, m) in ["Template text", "DAX expression", "Rules", "Gradient", "Field value"].iter().enumerate() {
                ui.selectable_value(&mut d.mode, i, *m);
            }
        });
        ui.separator();
        match d.mode {
            0 => {
                ui.label("Text with live values in braces:");
                ui.add(egui::TextEdit::multiline(&mut d.template).desired_rows(3).desired_width(f32::INFINITY).hint_text("Sales are {[Total Sales]:$#,0} ({[Margin %]:0.0%} margin) in {Selected:Region}"));
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Insert:").size(11.0));
                    for m in measures.iter().take(12) {
                        if ui.small_button(m).clicked() {
                            d.template.push_str(&format!("{{{m}}}"));
                        }
                    }
                    for extra in ["{Page}", "{Today:yyyy-mm-dd}", "{Filters}"] {
                        if ui.small_button(extra).clicked() {
                            d.template.push_str(extra);
                        }
                    }
                });
                ui.label(RichText::new("Formats: {[M]:$#,0}  {[M]:0.0%}  {[M]:#,0.0,,\"M\"}  {[M]:auto}").size(10.5));
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.label(match d.mode {
                        1 => "Expression",
                        4 => "Field / expression returning the value",
                        _ => "Based on",
                    });
                    egui::ComboBox::from_id_salt("fx-measure").selected_text("measures ▾").show_ui(ui, |ui| {
                        for m in &measures {
                            if ui.button(m).clicked() {
                                d.dax = m.clone();
                            }
                        }
                    });
                });
                ui.add(egui::TextEdit::multiline(&mut d.dax).code_editor().desired_rows(2).desired_width(f32::INFINITY).hint_text("[Margin %]  or  SUM(Sales[Quantity])  or  IF([Profit] > 0, \"#16A34A\", \"#DC2626\")"));
                if matches!(app.report.fx.as_ref().map(|x| x.key.as_str()), Some(k) if k.starts_with("cf.")) {
                    ui.label(RichText::new("Leave the expression empty to use the cell's own value.").size(10.5));
                }
                if d.mode == 2 {
                    rules_editor(ui, &mut d);
                }
                if d.mode == 3 {
                    egui::Grid::new("grad").num_columns(3).show(ui, |ui| {
                        ui.label("Lowest");
                        ui.add(egui::TextEdit::singleline(&mut d.min).hint_text("auto").desired_width(80.0));
                        widgets::color_edit(ui, &mut d.min_color);
                        ui.end_row();
                        ui.checkbox(&mut d.mid, "Middle");
                        ui.label("");
                        widgets::color_edit(ui, &mut d.mid_color);
                        ui.end_row();
                        ui.label("Highest");
                        ui.add(egui::TextEdit::singleline(&mut d.max).hint_text("auto").desired_width(80.0));
                        widgets::color_edit(ui, &mut d.max_color);
                        ui.end_row();
                    });
                    // Preview strip.
                    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
                    for i in 0..60 {
                        let tt = i as f64 / 59.0;
                        let c = vizardcraft_core::fx::gradient(tt, 0.0, 1.0, d.min_color, if d.mid { Some(d.mid_color) } else { None }, d.max_color);
                        let x0 = r.left() + r.width() * i as f32 / 60.0;
                        ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(x0, r.top()), egui::pos2(x0 + r.width() / 60.0 + 1.0, r.bottom())), 0.0, crate::theme::c32(c));
                    }
                }
            }
        }
        ui.separator();
        if ui.button("Test").clicked() {
            d.preview = test(app, &d);
        }
        if !d.preview.is_empty() {
            ui.label(RichText::new(format!("Current value: {}", d.preview)).monospace());
        }
        ui.horizontal(|ui| {
            if ui.button(RichText::new("OK").strong()).clicked() {
                apply = true;
            }
            if ui.button("Cancel").clicked() {
                app.report.fx = None;
            }
        });
    });
    if apply {
        let fx = build(&d);
        app.checkpoint("fx binding");
        let key = d.key.clone();
        if let Some(e) = app.project.element_mut(d.element) {
            if let Some(idx) = key.strip_prefix("run.").and_then(|r| r.strip_suffix(".color")).and_then(|i| i.parse::<usize>().ok()) {
                if let ElementKind::Text(ts) = &mut e.kind
                    && let Some(run) = ts.text.runs.get_mut(idx)
                {
                    run.color = Some(Prop::Fx(fx));
                }
            } else {
                e.props.insert(key, Prop::Fx(fx));
            }
        }
        app.report.render_cache.clear();
        app.report.text_cache.clear();
        app.report.fx = None;
        return;
    }
    if !open {
        app.report.fx = None;
    } else if app.report.fx.is_some() {
        app.report.fx = Some(d);
    }
}

fn rules_editor(ui: &mut Ui, d: &mut FxDialog) {
    ui.label(RichText::new("Rules (first match wins)").strong());
    let ops = [">", ">=", "<", "<=", "=", "<>", "between", "contains", "equals text", "is blank", "is not blank"];
    let mut remove = None;
    for (i, r) in d.rules.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label("If value");
            egui::ComboBox::from_id_salt(("rop", i)).width(70.0).selected_text(r.op.as_str()).show_ui(ui, |ui| {
                for o in ops {
                    ui.selectable_value(&mut r.op, o.to_string(), o);
                }
            });
            if matches!(r.op.as_str(), "contains" | "equals text") {
                ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(80.0));
            } else if !r.op.starts_with("is ") {
                ui.add(egui::DragValue::new(&mut r.value).speed(0.01));
                if r.op == "between" {
                    ui.label("and");
                    ui.add(egui::DragValue::new(&mut r.value2).speed(0.01));
                }
                ui.checkbox(&mut r.percent, "%").on_hover_text("Percent of the value range");
            }
            ui.label("then");
            match &mut r.output {
                Prop::Color(c) => {
                    widgets::color_edit(ui, c);
                }
                Prop::Text(s) => {
                    ui.add(egui::TextEdit::singleline(s).desired_width(80.0));
                }
                Prop::Num(n) => {
                    ui.add(egui::DragValue::new(n));
                }
                Prop::Bool(b) => {
                    ui.checkbox(b, "");
                }
                _ => {}
            }
            if ui.small_button("✕").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        d.rules.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button("+ New rule").clicked() {
            let output = match d.kind {
                PropKind::Color => Prop::Color(Color::rgb(0x2B, 0x6C, 0xE0)),
                PropKind::Bool => Prop::Bool(true),
                PropKind::Num { .. } => Prop::Num(1.0),
                _ => Prop::Text("text".into()),
            };
            d.rules.push(FxRule { op: ">".into(), value: 0.0, value2: 0.0, text: String::new(), percent: false, output });
        }
        ui.label("Otherwise");
        match &mut d.default {
            Prop::Color(c) => {
                widgets::color_edit(ui, c);
            }
            Prop::Text(s) => {
                ui.add(egui::TextEdit::singleline(s).desired_width(80.0));
            }
            Prop::Num(n) => {
                ui.add(egui::DragValue::new(n));
            }
            _ => {}
        }
    });
    if !matches!(d.kind, PropKind::Color) && matches!(d.default, Prop::Color(_)) {
        d.default = match d.kind {
            PropKind::Bool => Prop::Bool(false),
            PropKind::Num { .. } => Prop::Num(0.0),
            _ => Prop::Text(String::new()),
        };
    }
}

fn test(app: &VizApp, d: &FxDialog) -> String {
    let Some((pid, el)) = app.project.element(d.element) else { return String::new() };
    let Some(page) = app.project.page(pid) else { return String::new() };
    let cx = vizardcraft_core::state::context_for(&app.project, &app.built, &app.rt, page, el);
    let env = vizardcraft_core::fx::FxEnv::new(&app.project, &app.built, &cx, &page.name);
    match env.resolve_fx(&build(d)) {
        Prop::Color(c) => c.hex(),
        other => other.as_text(),
    }
}
