//! Running actions: buttons (bookmark, page navigation, drill-through, back, URL, Q&A, clear
//! slicers, set parameter), visual actions (slicer clicks, matrix expand), drilling, and
//! Tableau dashboard actions.

use vizcraft_core::doc::*;
use vizcraft_core::scalar::Scalar;

use crate::app::VizApp;

pub fn run(app: &mut VizApp, a: &Action) {
    match a {
        Action::Back => {
            if let Some(p) = app.rt.back_stack.pop() {
                app.page = p;
                app.selected.clear();
            }
        }
        Action::Bookmark { id } => {
            if let Some(b) = app.project.report.bookmarks.iter().find(|b| b.id == *id).cloned() {
                super::bookmarks::apply(app, &b);
            }
        }
        Action::PageNavigation { page } | Action::Drillthrough { page } => {
            app.go_to_page(*page);
        }
        Action::Url { url } => open_url(url),
        Action::QandA => super::insert_visual(app, VisualKind::QandA),
        Action::ClearAllSlicers => {
            for pg in app.project.report.pages.iter_mut() {
                for e in pg.elements.iter_mut() {
                    if let Some(v) = e.visual_mut()
                        && v.kind == VisualKind::Slicer
                    {
                        v.selection.clear();
                        v.range = None;
                    }
                }
            }
            app.rt.selection = None;
            app.report.render_cache.clear();
        }
        Action::SetParameter { name, value } => set_parameter(app, name, value.clone()),
        Action::ApplyAllSlicers => app.report.render_cache.clear(),
    }
}

pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(url).spawn();
}

pub fn set_parameter(app: &mut VizApp, name: &str, value: Scalar) {
    if let Some(p) = app.project.model.parameters.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) {
        p.current = value;
        app.mark_model_dirty();
    }
}

/// Slicer clicks, range changes, matrix expand/collapse.
pub fn visual_action(app: &mut VizApp, id: u64, action: &str, mods: egui::Modifiers) {
    let single = app.project.element(id).map(|(_, e)| e.props.get("slicer.single").and_then(|p| p.as_bool()).unwrap_or(false)).unwrap_or(false);
    let multi_ctrl = app.project.element(id).map(|(_, e)| e.props.get("slicer.multiCtrl").and_then(|p| p.as_bool()).unwrap_or(false)).unwrap_or(false);
    let sync_group = app.project.element(id).and_then(|(_, e)| e.sync_group.clone());
    let Some(el) = app.project.element_mut(id) else { return };
    let Some(v) = el.visual_mut() else { return };
    if let Some(rest) = action.strip_prefix("slicer:") {
        if rest == "all" {
            v.selection.clear();
        } else if let Ok(val) = serde_json::from_str::<Scalar>(rest) {
            let pos = v.selection.iter().position(|x| x.key() == val.key());
            if single || (multi_ctrl && !mods.command) {
                v.selection = if pos.is_some() && v.selection.len() == 1 { vec![] } else { vec![val] };
            } else if let Some(i) = pos {
                v.selection.remove(i);
            } else {
                v.selection.push(val);
            }
        }
        let sel = v.selection.clone();
        if let Some(g) = sync_group {
            sync_slicers(app, id, &g, sel);
        }
    } else if let Some(rest) = action.strip_prefix("range:") {
        let mut it = rest.split(':');
        let a = it.next().and_then(|s| s.parse().ok());
        let b = it.next().and_then(|s| s.parse().ok());
        if let (Some(a), Some(b)) = (a, b) {
            v.range = Some((a, b));
        }
    } else if let Some(k) = action.strip_prefix("expand:") {
        if !v.expanded.contains(&k.to_string()) {
            v.expanded.push(k.to_string());
        }
    } else if let Some(k) = action.strip_prefix("collapse:") {
        v.expanded.retain(|x| x != k);
    } else if let Some(k) = action.strip_prefix("sort:") {
        let desc = v.sort.as_ref().is_some_and(|s| s.by == k && !s.descending);
        v.sort = Some(SortSpec { by: k.to_string(), descending: desc || v.sort.as_ref().is_none_or(|s| s.by != k) });
    } else if let Some(n) = action.strip_prefix("scroll:") {
        let _ = n;
    } else if let Some(q) = action.strip_prefix("qna:") {
        v.question = q.to_string();
    }
    app.dirty = true;
    app.report.render_cache.clear();
}

/// Slicers in the same sync group follow each other.
pub fn sync_slicers(app: &mut VizApp, source: u64, group: &str, sel: Vec<Scalar>) {
    for pg in app.project.report.pages.iter_mut() {
        for e in pg.elements.iter_mut() {
            if e.id != source
                && e.sync_group.as_deref() == Some(group)
                && let Some(v) = e.visual_mut()
            {
                v.selection = sel.clone();
            }
        }
    }
    app.project.report.sync_groups.insert(group.to_string(), sel);
}

pub fn drill(app: &mut VizApp, id: u64, op: &str) {
    let levels = app.report.render_cache.values().find_map(|(_, r)| r.data.as_ref().filter(|_| true).map(|d| d.drill_levels)).unwrap_or(1);
    let levels = app
        .project
        .element(id)
        .and_then(|(_, e)| e.visual())
        .map(|v| {
            let role = if v.kind == VisualKind::Matrix { "rows" } else if matches!(v.kind, VisualKind::Pie | VisualKind::Donut) { "legend" } else { "axis" };
            v.well(role).iter().filter(|f| !f.is_value()).count()
        })
        .unwrap_or(levels);
    let Some(el) = app.project.element_mut(id) else { return };
    let Some(v) = el.visual_mut() else { return };
    match op {
        "up" => {
            if v.drill.expand_to > 0 {
                v.drill.expand_to -= 1;
            } else if v.drill.level > 0 {
                v.drill.level -= 1;
                v.drill.path.truncate(v.drill.level);
            }
        }
        "mode" => v.drill.drill_mode = !v.drill.drill_mode,
        "next" => {
            if v.drill.level + 1 < levels {
                v.drill.level += 1;
                v.drill.path.clear();
                v.drill.expand_to = 0;
            }
        }
        "expand" => {
            if v.drill.expand_to + 1 < levels {
                v.drill.expand_to = v.drill.expand_to.max(v.drill.level) + 1;
            }
        }
        _ => {}
    }
    app.report.render_cache.clear();
    app.dirty = true;
}

pub fn drill_down_on(app: &mut VizApp, id: u64, point: &[(FieldRef, Scalar)]) {
    let levels = app
        .project
        .element(id)
        .and_then(|(_, e)| e.visual())
        .map(|v| {
            let role = if v.kind == VisualKind::Matrix { "rows" } else { "axis" };
            v.well(role).iter().filter(|f| !f.is_value()).count()
        })
        .unwrap_or(1);
    let Some(el) = app.project.element_mut(id) else { return };
    let Some(v) = el.visual_mut() else { return };
    if v.drill.level + 1 >= levels {
        return;
    }
    if let Some((_, val)) = point.first() {
        v.drill.path.truncate(v.drill.level);
        v.drill.path.push(val.clone());
        v.drill.level += 1;
    }
    app.report.render_cache.clear();
}

/// Tableau dashboard actions triggered by selecting marks in `source`.
pub fn dashboard_actions(app: &mut VizApp, page: &Page, source: u64, point: &[(FieldRef, Scalar)]) {
    for a in page.actions.iter().filter(|a| a.source.contains(&source) && a.run_on != "Hover") {
        match a.kind.as_str() {
            "URL" => {
                let mut url = a.url.clone();
                for (f, v) in point {
                    url = url.replace(&format!("<{}>", f.label()), &v.text());
                }
                open_url(&url);
            }
            "Go to Sheet" => {
                if let Some(pid) = a.target_page {
                    if let Some(sel) = app.rt.selection.clone() {
                        app.rt.drillthrough.insert(pid, sel.points.first().cloned().unwrap_or_default());
                    }
                    app.go_to_page(pid);
                }
            }
            "Change Parameter" => {
                if let (Some(name), Some((_, v))) = (&a.parameter, point.first()) {
                    set_parameter(app, name, v.clone());
                }
            }
            _ => {
                // Filter / Highlight: set the target interactions accordingly.
                let it = if a.kind == "Highlight" { Interaction::Highlight } else { Interaction::Filter };
                for t in &a.targets {
                    if let Some(e) = app.project.element_mut(*t) {
                        e.interactions.insert(source, it);
                    }
                }
            }
        }
    }
}
