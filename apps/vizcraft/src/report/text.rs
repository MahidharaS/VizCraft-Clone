//! Rich text boxes: runs with sizes, weights, colours (fixed or fx) and live `{…}` values.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Painter, Rect, Ui, pos2, vec2};
use vizcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::{Tokens, c32};

#[allow(clippy::too_many_arguments)]
pub fn paint_rich(app: &mut VizApp, ui: &Ui, p: &Painter, page: &Page, el: &Element, text: &RichText, r: Rect, zoom: f32, opacity: f32, t: &Tokens) {
    let _ = ui;
    // A template override from the format pane replaces the runs.
    let override_text = el.props.get("text.override").map(|pr| super::render::resolve(app, page, el, pr).as_text()).filter(|s| !s.is_empty());
    let default_color = c32(app.project.report.theme.foreground);
    let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: r.width().max(1.0), ..Default::default() }, ..Default::default() };
    job.halign = match text.align.as_str() {
        "center" | "Center" => Align::Center,
        "right" | "Right" => Align::RIGHT,
        _ => Align::LEFT,
    };
    let mut bold_job = job.clone();
    let runs: Vec<TextRun> = match override_text {
        Some(s) => vec![TextRun { text: s, ..text.runs.first().cloned().unwrap_or_default() }],
        None => text.runs.clone(),
    };
    for run in &runs {
        let s = super::render::cached_template(app, page, el, &run.text);
        let color = run.color.as_ref().map(|c| super::render::resolve(app, page, el, c)).and_then(|c| c.as_color()).map(c32).unwrap_or(default_color).gamma_multiply(opacity);
        let fmt = TextFormat {
            font_id: FontId::proportional((run.size * zoom).max(1.0)),
            color,
            italics: run.italic,
            underline: if run.underline || run.link.is_some() { egui::Stroke::new(1.0, color) } else { egui::Stroke::NONE },
            line_height: if text.line_height > 0.0 { Some(run.size * zoom * text.line_height.max(0.8)) } else { None },
            ..Default::default()
        };
        job.append(&s, 0.0, fmt.clone());
        let mut bf = fmt;
        if !run.bold {
            bf.color = Color32::TRANSPARENT;
            bf.underline = egui::Stroke::NONE;
        }
        bold_job.append(&s, 0.0, bf);
    }
    let galley = p.ctx().fonts_mut(|f| f.layout_job(job));
    let h = galley.size().y;
    let y = match text.valign.as_str() {
        "middle" | "Middle" => r.center().y - h / 2.0,
        "bottom" | "Bottom" => r.bottom() - h,
        _ => r.top(),
    };
    let x = match text.align.as_str() {
        "center" | "Center" => r.center().x,
        "right" | "Right" => r.right(),
        _ => r.left(),
    };
    let cp = p.with_clip_rect(r.expand(4.0).intersect(p.clip_rect()));
    cp.galley(pos2(x, y), galley, default_color);
    if runs.iter().any(|r| r.bold) {
        let bg = p.ctx().fonts_mut(|f| f.layout_job(bold_job));
        cp.galley(pos2(x + 0.55 * zoom.max(0.6), y), bg, default_color);
    }
    if runs.is_empty() || runs.iter().all(|r| r.text.is_empty()) {
        cp.text(r.left_top() + vec2(2.0, 2.0), egui::Align2::LEFT_TOP, "Double-click to type. Use {[Measure]} for live values.", FontId::proportional(11.0 * zoom), t.muted);
    }
}

/// Inline editor overlay for a text box being edited.
pub fn inline_editor(app: &mut VizApp, ui: &mut Ui, r: Rect) {
    let Some(id) = app.report.editing_text else { return };
    let mut done = false;
    egui::Area::new(ui.scope_id().with(("edit", id))).fixed_pos(r.min).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(r.width().max(260.0));
            ui.label(egui::RichText::new("Edit text — use {[Measure]}, {[Measure]:$#,0}, {Selected:Column}, {@Parameter}, {Page}, {Today:yyyy-mm-dd}").size(10.5));
            let resp = ui.add(egui::TextEdit::multiline(&mut app.report.text_buffer).desired_width(r.width().max(260.0)).desired_rows(3));
            resp.request_focus();
            ui.horizontal(|ui| {
                if ui.button("Done").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command) {
                    done = true;
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    app.report.editing_text = None;
                }
            });
        });
    });
    if done {
        let txt = app.report.text_buffer.clone();
        app.checkpoint("Edit text");
        if let Some(el) = app.project.element_mut(id)
            && let ElementKind::Text(ts) = &mut el.kind
        {
            match ts.text.runs.len() {
                0 => ts.text.runs.push(TextRun { text: txt, ..TextRun::default() }),
                1 => {
                    if let Some(r0) = ts.text.runs.first_mut() {
                        r0.text = txt;
                    }
                }
                _ => {
                    // Editing a multi-run text as one string keeps the first run's style.
                    if ts.text.raw_text() != txt {
                        let first = ts.text.runs.first().cloned().unwrap_or_default();
                        ts.text.runs = vec![TextRun { text: txt, ..first }];
                    }
                }
            }
        }
        app.report.editing_text = None;
        app.report.render_cache.clear();
    }
}
