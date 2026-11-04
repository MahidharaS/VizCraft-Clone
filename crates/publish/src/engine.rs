//! The report engine behind a published report: the project, its data, the model and the
//! interaction state, with the operations the web viewer calls (render a page, click a data point,
//! use a slicer, drill, change a parameter, apply a bookmark or a story point, navigate).
//!
//! The WASM viewer (`crates/wasm`) wraps this; [`crate::bundle`] uses it to pre-render the static
//! fallback.

use std::collections::BTreeMap;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use vizardcraft_core::build::{self, Built, TableData};
use vizardcraft_core::doc::*;
use vizardcraft_core::scalar::Scalar;
use vizardcraft_core::state::{self, Runtime};

use crate::render::{self, ElementOut, PHONE_WIDTH};

/// `report.json`: everything a published report needs, offline.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReportData {
    pub format: u32,
    pub name: String,
    /// When it was published (RFC 3339, UTC).
    pub published: String,
    pub generator: String,
    /// The WASM engine ships with the report (slicers, cross-filtering, drill, parameters).
    pub interactive: bool,
    pub include_mobile: bool,
    /// Pages shown as tabs, in order.
    pub pages: Vec<PageInfo>,
    pub bookmarks: Vec<BookmarkInfo>,
    pub stories: Vec<StoryInfo>,
    /// Parameters the viewer can change (interactive mode).
    pub parameters: Vec<ParamInfo>,
    /// The serial date the report was published on ("today" for relative date filters).
    pub today: f64,
    /// Pre-rendered views (static mode, and the first paint in interactive mode): key
    /// `page:<id>`, `bm:<id>`, `sp:<story>:<index>`.
    pub views: BTreeMap<String, View>,
    /// The project (interactive mode).
    pub project: Option<Project>,
    /// Loaded table data (interactive mode).
    pub tables: Vec<TableData>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PageInfo {
    pub id: u64,
    pub name: String,
    pub kind: String,
    /// Not a tab (hidden page; still reachable by buttons).
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BookmarkInfo {
    pub id: u64,
    pub name: String,
    pub group: String,
    pub page: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoryInfo {
    pub id: u64,
    pub name: String,
    pub title: String,
    pub points: Vec<StoryPointInfo>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoryPointInfo {
    pub caption: String,
    pub page: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParamInfo {
    pub name: String,
    pub description: String,
    /// "range" (min/max/step) or "list".
    pub kind: String,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub list: Vec<Scalar>,
    pub current: Scalar,
}

impl ParamInfo {
    pub fn of(p: &Parameter) -> Option<ParamInfo> {
        let base = ParamInfo { name: p.name.clone(), description: p.description.clone(), current: p.current.clone(), ..ParamInfo::default() };
        match &p.kind {
            ParameterKind::WhatIf { min, max, step, .. } => Some(ParamInfo { kind: "range".into(), min: *min, max: *max, step: *step, ..base }),
            ParameterKind::Value { list, min, max, step, .. } => {
                if !list.is_empty() {
                    Some(ParamInfo { kind: "list".into(), list: list.clone(), ..base })
                } else if let (Some(a), Some(b)) = (min, max) {
                    Some(ParamInfo { kind: "range".into(), min: *a, max: *b, step: step.unwrap_or((b - a) / 100.0), ..base })
                } else {
                    None
                }
            }
            ParameterKind::Fields { items } => Some(ParamInfo { kind: "list".into(), list: items.iter().map(|f| Scalar::Text(f.label())).collect(), ..base }),
        }
    }
}

/// A pre-rendered view: the page in its desktop layout and (optionally) the phone layout.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct View {
    pub desktop: PageOut,
    pub mobile: Option<PageOut>,
}

/// A rendered page.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PageOut {
    pub page: u64,
    pub name: String,
    pub width: f32,
    pub height: f32,
    pub mobile: bool,
    /// CSS background of the page canvas.
    pub background: String,
    /// CSS background around the page.
    pub wallpaper: String,
    /// "Fit to page", "Fit to width", "Actual size".
    pub view_mode: String,
    pub elements: Vec<ElementOut>,
    /// Active filters, readable ("Region is South").
    pub filters: Vec<String>,
    pub can_back: bool,
    pub has_selection: bool,
}

/// What an action asks the viewer to do.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Outcome {
    /// Show this page now.
    pub page: Option<u64>,
    /// Open this URL.
    pub url: Option<String>,
    /// Something changed: re-render.
    pub changed: bool,
    pub message: Option<String>,
}

pub struct Engine {
    pub original: Project,
    pub p: Project,
    pub tables: BTreeMap<String, Rc<TableData>>,
    pub built: Built,
    pub rt: Runtime,
}

impl Clone for Engine {
    fn clone(&self) -> Self {
        Engine { original: self.original.clone(), p: self.p.clone(), tables: self.tables.clone(), built: self.built.clone(), rt: self.rt.clone() }
    }
}

fn params(p: &Project) -> BTreeMap<String, Scalar> {
    p.model.parameters.iter().map(|x| (x.name.clone(), x.current.clone())).collect()
}

impl Engine {
    /// From a project and its refreshed data.
    pub fn new(p: Project, built: Built) -> Engine {
        let first = first_page(&p);
        Engine { original: p.clone(), tables: built.tables.clone(), p, built, rt: Runtime { page: first, ..Runtime::default() } }
    }

    /// From `report.json` data (rebuilds the model from the table snapshot).
    pub fn from_data(project: Project, tables: Vec<TableData>) -> Engine {
        let tables: BTreeMap<String, Rc<TableData>> = tables.into_iter().map(|t| (t.name.clone(), Rc::new(t))).collect();
        let (model, errors) = build::build_model(&project, &tables, &params(&project));
        let built = Built { model: Rc::new(model), tables, errors };
        Engine::new(project, built)
    }

    fn rebuild(&mut self) {
        let (model, errors) = build::build_model(&self.p, &self.tables, &params(&self.p));
        self.built = Built { model: Rc::new(model), tables: self.tables.clone(), errors };
    }

    pub fn page(&self, id: u64) -> Option<&Page> {
        self.p.page(id)
    }

    /// Renders a page in the desktop or the phone layout.
    pub fn render_page(&mut self, page_id: u64, mobile: bool) -> Option<PageOut> {
        let page = self.p.page(page_id)?.clone();
        if self.rt.page != page_id {
            self.rt.page = page_id;
        }
        let (placed, width, height) = if mobile {
            let (pl, h) = render::mobile_layout(&page);
            (pl, PHONE_WIDTH, h)
        } else {
            (render::desktop_layout(&page), page.width, page.height)
        };
        let mut elements = Vec::new();
        for pl in &placed {
            let Some(el) = page.element(pl.id) else { continue };
            elements.push(render::element_at(&self.p, &self.built, &self.rt, &page, el, pl, &format!("e{}{}", el.id, if mobile { "m" } else { "d" })));
        }
        let wallpaper = match &page.wallpaper {
            Fill::None => self.p.report.theme.background.css(),
            f => render::fill_css(f),
        };
        Some(PageOut {
            page: page.id,
            name: page.name.clone(),
            width,
            height,
            mobile,
            background: render::fill_css(&page.background),
            wallpaper,
            view_mode: if page.view_mode.is_empty() { "Fit to page".into() } else { page.view_mode.clone() },
            elements,
            filters: self.filter_descriptions(&page),
            can_back: !self.rt.back_stack.is_empty(),
            has_selection: self.rt.selection.as_ref().is_some_and(|s| page.element(s.source).is_some()),
        })
    }

    /// What filters the page: report and page filters, slicers, drill-through, the selection.
    pub fn filter_descriptions(&self, page: &Page) -> Vec<String> {
        let probe = Element { id: 0, kind: ElementKind::Group, ..Element::default() };
        let cx = state::context_for(&self.p, &self.built, &self.rt, page, &probe);
        let mut out = cx.description;
        if let Some(sel) = &self.rt.selection
            && let Some(src) = page.element(sel.source)
        {
            let vals: Vec<String> = sel.points.iter().map(|pt| pt.iter().map(|(_, v)| v.text()).collect::<Vec<_>>().join(" · ")).take(4).collect();
            let name = if src.name.is_empty() { src.kind_label() } else { src.name.clone() };
            out.push(format!("Selected in {name}: {}", vals.join(", ")));
        }
        out.dedup();
        out
    }

    /// A click on a data point of a visual (`point` = the `data-hit` identity; `None` = empty
    /// space, which clears the selection). In drill mode the click drills into the point.
    pub fn click(&mut self, element: u64, point: Option<Vec<(FieldRef, Scalar)>>, ctrl: bool) -> Outcome {
        let Some(point) = point.filter(|p| !p.is_empty()) else {
            let changed = self.rt.selection.take().is_some();
            return Outcome { changed, ..Outcome::default() };
        };
        let drill_mode = self.p.element(element).and_then(|(_, e)| e.visual()).is_some_and(|v| v.drill.drill_mode);
        if drill_mode && self.drill_into(element, &point) {
            return Outcome { changed: true, ..Outcome::default() };
        }
        state::toggle_selection(&mut self.rt, element, point, ctrl);
        Outcome { changed: true, ..Outcome::default() }
    }

    fn drill_into(&mut self, element: u64, point: &[(FieldRef, Scalar)]) -> bool {
        let Some(el) = self.p.element_mut(element) else { return false };
        let Some(v) = el.visual_mut() else { return false };
        let levels = ["axis", "rows", "legend", "location"].iter().map(|r| v.well(r).iter().filter(|f| !f.is_value()).count()).max().unwrap_or(0);
        if v.drill.level + 1 >= levels {
            return false;
        }
        let value = point.first().map(|(_, s)| s.clone()).unwrap_or_default();
        v.drill.path.truncate(v.drill.level);
        v.drill.path.push(value);
        v.drill.level += 1;
        v.drill.expand_to = 0;
        true
    }

    /// An action region of a visual (slicer items: `slicer:<value json>`; matrix rows:
    /// `expand:<key>`), or an [`Action`] as JSON (buttons, navigators).
    pub fn action(&mut self, element: u64, action: &str, ctrl: bool) -> Outcome {
        if action.trim_start().starts_with('{') {
            return match serde_json::from_str::<Action>(action) {
                Ok(a) => self.run_action(&a),
                Err(e) => Outcome { message: Some(format!("Unknown action: {e}")), ..Outcome::default() },
            };
        }
        if let Some(v) = action.strip_prefix("slicer:") {
            let value: Scalar = serde_json::from_str(v).unwrap_or(Scalar::Text(v.to_string()));
            return self.slicer_toggle(element, value, ctrl);
        }
        if let Some(key) = action.strip_prefix("expand:") {
            if let Some(v) = self.p.element_mut(element).and_then(Element::visual_mut) {
                if let Some(i) = v.expanded.iter().position(|k| k == key) {
                    v.expanded.remove(i);
                } else {
                    v.expanded.push(key.to_string());
                }
            }
            return Outcome { changed: true, ..Outcome::default() };
        }
        if action == "slicer:clear" || action == "clear" {
            return self.slicer_set(element, vec![], None);
        }
        Outcome::default()
    }

    fn slicer_toggle(&mut self, element: u64, value: Scalar, ctrl: bool) -> Outcome {
        let Some(v) = self.p.element(element).and_then(|(_, e)| e.visual()) else { return Outcome::default() };
        let mut sel = v.selection.clone();
        let single = v.slicer_mode == "Dropdown" && !ctrl;
        if let Some(i) = sel.iter().position(|x| x.key() == value.key()) {
            if ctrl || sel.len() == 1 {
                sel.remove(i);
            } else {
                sel = vec![value];
            }
        } else if ctrl && !single {
            sel.push(value);
        } else {
            sel = vec![value];
        }
        self.slicer_set(element, sel, None)
    }

    /// Sets a slicer's selection (and the slicers synced with it).
    pub fn slicer_set(&mut self, element: u64, values: Vec<Scalar>, range: Option<(f64, f64)>) -> Outcome {
        let group = self.p.element(element).and_then(|(_, e)| e.sync_group.clone());
        for page in &mut self.p.report.pages {
            for e in &mut page.elements {
                let same = e.id == element || (group.is_some() && e.sync_group == group);
                if !same {
                    continue;
                }
                if let Some(v) = e.visual_mut()
                    && v.kind == VisualKind::Slicer
                {
                    v.selection = values.clone();
                    v.range = range;
                }
            }
        }
        let first = values.first().cloned();
        if let Some(g) = group {
            self.p.report.sync_groups.insert(g, values);
        }
        // Slicers on a what-if parameter's table set its value.
        let param_slicer = self.p.element(element).and_then(|(_, e)| e.visual()).and_then(|v| v.well("field").first().cloned());
        if let Some(FieldUse { field: FieldRef::Column { table, .. }, .. }) = param_slicer
            && let Some(prm) = self.p.model.parameters.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&table))
        {
            if let Some(v) = first
                && matches!(prm.kind, ParameterKind::WhatIf { .. } | ParameterKind::Value { .. })
            {
                prm.current = v;
            }
            self.rebuild();
        }
        self.rt.selection = None;
        Outcome { changed: true, ..Outcome::default() }
    }

    /// Clears every slicer on every page.
    pub fn clear_slicers(&mut self) -> Outcome {
        for page in &mut self.p.report.pages {
            for e in &mut page.elements {
                if let Some(v) = e.visual_mut()
                    && v.kind == VisualKind::Slicer
                {
                    v.selection.clear();
                    v.range = None;
                }
            }
        }
        self.p.report.sync_groups.clear();
        self.rt.selection = None;
        Outcome { changed: true, ..Outcome::default() }
    }

    /// Sets a parameter (what-if, field or value parameter) and rebuilds the model.
    pub fn set_parameter(&mut self, name: &str, value: Scalar) -> Outcome {
        let Some(prm) = self.p.model.parameters.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)) else {
            return Outcome { message: Some(format!("No parameter named {name}.")), ..Outcome::default() };
        };
        prm.current = match (&prm.kind, &value) {
            // Field parameters hold the selected item's index.
            (ParameterKind::Fields { items }, Scalar::Text(t)) => items.iter().position(|f| &f.label() == t).map(|i| Scalar::Num(i as f64)).unwrap_or(Scalar::Num(0.0)),
            _ => value.clone(),
        };
        let pname = prm.name.clone();
        // Slicers on the parameter's table follow.
        for page in &mut self.p.report.pages {
            for e in &mut page.elements {
                if let Some(v) = e.visual_mut()
                    && v.kind == VisualKind::Slicer
                    && v.well("field").first().is_some_and(|f| f.field.table().eq_ignore_ascii_case(&pname))
                {
                    v.selection = vec![value.clone()];
                }
            }
        }
        self.rebuild();
        Outcome { changed: true, ..Outcome::default() }
    }

    /// Drill: "up", "down" (next level), "expand" (expand all down one level), "mode" (toggle
    /// drill mode: clicks drill into a point).
    pub fn drill(&mut self, element: u64, dir: &str) -> Outcome {
        let Some(v) = self.p.element_mut(element).and_then(Element::visual_mut) else { return Outcome::default() };
        let levels = ["axis", "rows", "legend", "location"].iter().map(|r| v.well(r).iter().filter(|f| !f.is_value()).count()).max().unwrap_or(0);
        let d = &mut v.drill;
        match dir {
            "up" => {
                if d.expand_to > 0 {
                    d.expand_to -= 1;
                } else if d.level > 0 {
                    d.level -= 1;
                    d.path.truncate(d.level);
                }
            }
            "down" | "next" => {
                if d.level + 1 < levels {
                    d.level += 1;
                    d.path.clear();
                    d.expand_to = 0;
                }
            }
            "expand" => {
                if d.expand_to + 1 < levels {
                    d.expand_to = d.expand_to.max(d.level) + 1;
                    d.path.clear();
                    d.level = 0;
                }
            }
            "mode" => d.drill_mode = !d.drill_mode,
            _ => return Outcome::default(),
        }
        if self.rt.selection.as_ref().is_some_and(|s| s.source == element) {
            self.rt.selection = None;
        }
        Outcome { changed: true, ..Outcome::default() }
    }

    /// Goes to a page (remembering where we were for Back).
    pub fn navigate(&mut self, page: u64) -> Outcome {
        self.go_to(page, true)
    }

    /// Shows a page; `remember` puts the current one on the Back stack (buttons do; page tabs
    /// don't).
    pub fn go_to(&mut self, page: u64, remember: bool) -> Outcome {
        if self.p.page(page).is_none() {
            return Outcome::default();
        }
        if remember && page != self.rt.page && self.rt.page != 0 {
            self.rt.back_stack.push(self.rt.page);
        }
        self.rt.page = page;
        self.rt.selection = None;
        Outcome { page: Some(page), changed: true, ..Outcome::default() }
    }

    pub fn back(&mut self) -> Outcome {
        match self.rt.back_stack.pop() {
            Some(pg) => {
                self.rt.page = pg;
                self.rt.selection = None;
                Outcome { page: Some(pg), changed: true, ..Outcome::default() }
            }
            None => Outcome::default(),
        }
    }

    pub fn run_action(&mut self, a: &Action) -> Outcome {
        match a {
            Action::Back => self.back(),
            Action::Bookmark { id } => self.apply_bookmark(*id),
            Action::PageNavigation { page } | Action::Drillthrough { page } => self.navigate(*page),
            Action::Url { url } => Outcome { url: Some(url.clone()), ..Outcome::default() },
            Action::ClearAllSlicers => self.clear_slicers(),
            Action::SetParameter { name, value } => self.set_parameter(name, value.clone()),
            Action::QandA | Action::ApplyAllSlicers => Outcome::default(),
        }
    }

    /// Applies captured view state (bookmarks, story points).
    pub fn apply_state(&mut self, st: &ViewState, only: Option<&[u64]>) {
        let applies = |id: u64| only.is_none_or(|o| o.contains(&id));
        let mut params_changed = false;
        for (fid, kind) in &st.filters {
            let set = |fs: &mut Vec<FilterDef>| {
                for f in fs.iter_mut().filter(|f| f.id == *fid) {
                    f.kind = kind.clone();
                }
            };
            set(&mut self.p.report.filters);
            for page in &mut self.p.report.pages {
                set(&mut page.filters);
                for e in &mut page.elements {
                    if let Some(v) = e.visual_mut() {
                        set(&mut v.filters);
                    }
                }
            }
        }
        for page in &mut self.p.report.pages {
            for e in &mut page.elements {
                if !applies(e.id) {
                    continue;
                }
                if let Some(h) = st.hidden.get(&e.id) {
                    e.hidden = *h;
                }
                let id = e.id;
                if let Some(v) = e.visual_mut() {
                    if let Some((vals, range)) = st.slicers.get(&id) {
                        v.selection = vals.clone();
                        v.range = *range;
                    }
                    if let Some(d) = st.drill.get(&id) {
                        v.drill = d.clone();
                    }
                }
            }
        }
        for (name, val) in &st.parameters {
            if let Some(prm) = self.p.model.parameters.iter_mut().find(|x| &x.name == name)
                && prm.current != *val
            {
                prm.current = val.clone();
                params_changed = true;
            }
        }
        self.rt.selection = st.selection.clone();
        self.rt.focus = st.focus;
        if params_changed {
            self.rebuild();
        }
    }

    pub fn apply_bookmark(&mut self, id: u64) -> Outcome {
        let Some(bm) = self.p.report.bookmarks.iter().find(|b| b.id == id).cloned() else { return Outcome::default() };
        let only = if bm.all_visuals || bm.selected_visuals.is_empty() { None } else { Some(bm.selected_visuals.as_slice()) };
        if bm.capture_data || !bm.capture_display {
            self.apply_state(&bm.state, only);
        } else {
            // Display only: visibility.
            let st = ViewState { hidden: bm.state.hidden.clone(), ..ViewState::default() };
            self.apply_state(&st, only);
        }
        let mut out = Outcome { changed: true, ..Outcome::default() };
        if (bm.capture_current_page || bm.page != 0) && self.p.page(bm.page).is_some() {
            self.rt.page = bm.page;
            out.page = Some(bm.page);
        }
        out
    }

    /// Shows a story point: its page with its captured state.
    pub fn story_point(&mut self, story: u64, index: usize) -> Outcome {
        let Some(pt) = self.p.report.stories.iter().find(|s| s.id == story).and_then(|s| s.points.get(index)).cloned() else { return Outcome::default() };
        self.reset_state();
        self.apply_state(&pt.state, None);
        self.rt.page = pt.page;
        Outcome { page: Some(pt.page), changed: true, ..Outcome::default() }
    }

    /// Back to the report as published (filters, slicers, drill, parameters).
    pub fn reset_state(&mut self) {
        let params_changed = self.p.model.parameters != self.original.model.parameters;
        let page = self.rt.page;
        self.p = self.original.clone();
        self.rt = Runtime { page, ..Runtime::default() };
        if params_changed {
            self.rebuild();
        }
    }

    pub fn reset(&mut self) -> Outcome {
        self.reset_state();
        Outcome { changed: true, ..Outcome::default() }
    }
}

/// The first page shown: the first visible report page.
pub fn first_page(p: &Project) -> u64 {
    p.report.pages.iter().find(|pg| !pg.hidden && tab_kind(pg.kind)).or(p.report.pages.first()).map(|pg| pg.id).unwrap_or(0)
}

/// Pages that appear as tabs (tooltip and drill-through pages don't).
pub fn tab_kind(k: PageKind) -> bool {
    matches!(k, PageKind::Report | PageKind::Dashboard | PageKind::Worksheet)
}
