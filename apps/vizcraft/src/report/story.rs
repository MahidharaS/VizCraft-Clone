//! Stories (Tableau): sequences of story points; each point shows a page or dashboard with the
//! filter state captured when it was made, and a caption.

use egui::{Color32, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use vizcraft_core::doc::*;

use crate::app::{View, VizApp};

pub fn new_story(app: &mut VizApp) {
    app.checkpoint("New story");
    let id = app.project.next_id();
    let pid = app.project.next_id();
    let n = app.project.report.stories.len() + 1;
    let state = super::bookmarks::capture(app);
    let page = app.page;
    app.project.report.stories.push(Story {
        id,
        name: format!("Story {n}"),
        title: format!("Story {n}"),
        points: vec![StoryPoint { id: pid, caption: "Add a caption".into(), page, state, notes: vec![] }],
        width: 1280.0,
        height: 720.0,
        navigator_style: "Caption boxes".into(),
        background: Fill::Solid(vizcraft_core::color::Color::WHITE),
    });
    app.report.story = Some((id, 0));
    app.view = View::Story;
    app.pq_open = false;
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    if app.project.report.stories.is_empty() {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(120.0);
                ui.heading("Stories");
                ui.label("A story is a sequence of story points that walk through your data, like slides.");
                if ui.button("New story").clicked() {
                    new_story(app);
                }
            });
        });
        return;
    }
    let (sid, mut idx) = app.report.story.unwrap_or((app.project.report.stories.first().map(|s| s.id).unwrap_or(0), 0));
    let Some(story) = app.project.report.stories.iter().find(|s| s.id == sid).cloned() else {
        app.report.story = None;
        return;
    };
    idx = idx.min(story.points.len().saturating_sub(1));
    egui::Panel::left("story-list").default_size(220.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(8))).show(ui, |ui| {
        ui.label(RichText::new("Stories").strong());
        for s in app.project.report.stories.clone() {
            if ui.selectable_label(s.id == sid, &s.name).clicked() {
                app.report.story = Some((s.id, 0));
            }
        }
        if ui.button("+ New story").clicked() {
            new_story(app);
        }
        ui.separator();
        ui.label(RichText::new("Story point").strong());
        let pages: Vec<(u64, String)> = app.project.report.pages.iter().filter(|p| p.kind != PageKind::Tooltip).map(|p| (p.id, p.name.clone())).collect();
        if let Some(pt) = story.points.get(idx).cloned() {
            let mut pt2 = pt.clone();
            let mut changed = false;
            ui.label("Caption");
            changed |= ui.add(egui::TextEdit::multiline(&mut pt2.caption).desired_rows(2)).changed();
            ui.label("Shows");
            egui::ComboBox::from_id_salt("sp-page").selected_text(pages.iter().find(|p| p.0 == pt2.page).map(|p| p.1.clone()).unwrap_or_default()).show_ui(ui, |ui| {
                for (pid, name) in &pages {
                    changed |= ui.selectable_value(&mut pt2.page, *pid, name).changed();
                }
            });
            if ui.button("Update point with current view").on_hover_text("Capture filters/slicers as they are now").clicked() {
                pt2.state = super::bookmarks::capture(app);
                changed = true;
            }
            if changed {
                app.checkpoint("Story point");
                if let Some(s) = app.project.report.stories.iter_mut().find(|s| s.id == sid)
                    && let Some(p) = s.points.get_mut(idx)
                {
                    *p = pt2;
                }
            }
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("+ Blank").clicked() {
                add_point(app, sid, idx, false);
            }
            if ui.button("Duplicate").clicked() {
                add_point(app, sid, idx, true);
            }
            if ui.button("Delete").clicked() && story.points.len() > 1 {
                app.checkpoint("Delete story point");
                if let Some(s) = app.project.report.stories.iter_mut().find(|s| s.id == sid) {
                    s.points.remove(idx);
                }
                app.report.story = Some((sid, idx.saturating_sub(1)));
            }
        });
        ui.separator();
        let mut st = story.clone();
        ui.label("Title");
        if ui.text_edit_singleline(&mut st.title).changed() {
            st.name = st.title.clone();
            if let Some(s) = app.project.report.stories.iter_mut().find(|s| s.id == sid) {
                s.title = st.title.clone();
                s.name = st.name.clone();
            }
        }
        ui.label("Navigator");
        egui::ComboBox::from_id_salt("navstyle").selected_text(&st.navigator_style).show_ui(ui, |ui| {
            for n in ["Caption boxes", "Numbers", "Dots", "Arrows only"] {
                if ui.selectable_label(st.navigator_style == n, n).clicked()
                    && let Some(s) = app.project.report.stories.iter_mut().find(|s| s.id == sid)
                {
                    s.navigator_style = n.into();
                }
            }
        });
    });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas)).show(ui, |ui| {
        let avail = ui.available_rect_before_wrap();
        // Title.
        ui.painter().text(avail.left_top() + vec2(24.0, 14.0), egui::Align2::LEFT_TOP, &story.title, egui::FontId::proportional(20.0), t.text);
        // Navigator.
        let nav_top = avail.top() + 48.0;
        let n = story.points.len();
        let mut x = avail.left() + 24.0;
        let arrow = |ui: &mut Ui, r: egui::Rect, label: &str| -> bool {
            let resp = ui.interact(r, ui.scope_id().with(label), Sense::click());
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(18.0), t.accent);
            resp.clicked()
        };
        if arrow(ui, egui::Rect::from_min_size(egui::pos2(x, nav_top), vec2(24.0, 52.0)), "‹") && idx > 0 {
            idx -= 1;
        }
        x += 28.0;
        for (i, p) in story.points.iter().enumerate() {
            let active = i == idx;
            let (w, label) = match story.navigator_style.as_str() {
                "Numbers" => (36.0, format!("{}", i + 1)),
                "Dots" => (18.0, "●".to_string()),
                "Arrows only" => (0.0, String::new()),
                _ => (180.0, vizcraft_core::scene::ellipsize(&p.caption, 12.0, 168.0)),
            };
            if w == 0.0 {
                continue;
            }
            let r = egui::Rect::from_min_size(egui::pos2(x, nav_top), vec2(w, 52.0));
            let resp = ui.interact(r, ui.scope_id().with(("sp", i)), Sense::click());
            ui.painter().rect(r, 6.0, if active { t.accent } else { t.panel }, Stroke::new(1.0, if active { t.accent } else { t.border }), StrokeKind::Inside);
            let g = ui.painter().layout(label, egui::FontId::proportional(12.0), if active { Color32::WHITE } else { t.text }, w - 12.0);
            ui.painter().galley(r.center() - g.size() / 2.0, g, t.text);
            if resp.clicked() {
                idx = i;
            }
            x += w + 6.0;
        }
        if arrow(ui, egui::Rect::from_min_size(egui::pos2(x, nav_top), vec2(24.0, 52.0)), "›") && idx + 1 < n {
            idx += 1;
        }
        // The point's page.
        let Some(pt) = story.points.get(idx).cloned() else { return };
        if app.report.story != Some((sid, idx)) || app.report.story_applied != Some(pt.id) {
            super::bookmarks::apply_state(app, &pt.state, true, true, None);
            app.report.story_applied = Some(pt.id);
        }
        app.report.story = Some((sid, idx));
        let Some(page) = app.project.page(pt.page).cloned() else { return };
        let area = egui::Rect::from_min_max(egui::pos2(avail.left() + 24.0, nav_top + 68.0), avail.right_bottom() - vec2(24.0, 16.0));
        let zoom = (area.width() / page.width).min(area.height() / page.height).max(0.05);
        let origin = egui::pos2(area.center().x - page.width * zoom / 2.0, area.top());
        let p = ui.painter_at(area);
        super::canvas::paint_page_static(app, ui, &p, &page, super::canvas::PageXf { origin, zoom }, &t);
    });
}

fn add_point(app: &mut VizApp, sid: u64, idx: usize, dup: bool) {
    app.checkpoint("Add story point");
    let id = app.project.next_id();
    let state = super::bookmarks::capture(app);
    let page = app.page;
    if let Some(s) = app.project.report.stories.iter_mut().find(|s| s.id == sid) {
        let mut p = if dup { s.points.get(idx).cloned().unwrap_or_default() } else { StoryPoint { caption: "New story point".into(), page, state, ..StoryPoint::default() } };
        p.id = id;
        s.points.insert((idx + 1).min(s.points.len()), p);
    }
    app.report.story = Some((sid, idx + 1));
}
