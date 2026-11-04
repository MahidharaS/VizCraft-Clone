//! Renders elements to scenes (with caching): visual data, resolved fx properties, analytics,
//! chrome and content.

use std::rc::Rc;

use vizardcraft_core::charts::{self, RenderOpts};
use vizardcraft_core::doc::*;
use vizardcraft_core::fx::{self, FxEnv, Props};
use vizardcraft_core::scene::{Rect, Scene};
use vizardcraft_core::state::{self, Context};
use vizardcraft_core::vquery::{self, VisualData};

use crate::app::VizApp;

/// Everything needed to draw one element.
#[derive(Clone)]
pub struct Rendered {
    pub chrome: Rc<Scene>,
    pub content: Rc<Scene>,
    /// Content box relative to the element.
    pub content_rect: Rect,
    pub data: Option<Rc<VisualData>>,
    pub props: Rc<Props>,
    pub cx: Rc<Context>,
    pub title: String,
    pub millis: f64,
}

/// Cached render of an element (keyed by its definition, context and size).
pub fn element(app: &mut VizApp, page_id: u64, el: &Element) -> Rendered {
    let Some(page) = app.project.page(page_id).cloned() else { return empty(el) };
    let cx = state::context_for(&app.project, &app.built, &app.rt, &page, el);
    let selected_points: Vec<Vec<(FieldRef, vizardcraft_core::scalar::Scalar)>> =
        app.rt.selection.as_ref().filter(|s| s.source == el.id).map(|s| s.points.clone()).unwrap_or_default();
    let key = format!(
        "{}|{}|{:?}|{:?}|{}|{}|{}",
        app.generation,
        serde_json::to_string(&el.kind).unwrap_or_default(),
        cx,
        selected_points,
        serde_json::to_string(&el.props).unwrap_or_default(),
        el.w.round(),
        el.h.round()
    ) + &format!("|{}|{}|{}", serde_json::to_string(&app.project.report.theme.name).unwrap_or_default(), app.report.slicer_search.get(&el.id).cloned().unwrap_or_default(), param_key(app));
    if let Some(r) = app.report.render_cache.get(&(el.id, page_id))
        && r.0 == key
    {
        return r.1.clone();
    }
    let t0 = std::time::Instant::now();
    let r = compute(app, &page, el, cx, &selected_points);
    let mut r = r;
    r.millis = t0.elapsed().as_secs_f64() * 1000.0;
    app.report.render_cache.insert((el.id, page_id), (key, r.clone()));
    r
}

fn param_key(app: &VizApp) -> String {
    app.project.model.parameters.iter().map(|p| p.current.key()).collect::<Vec<_>>().join(",")
}

fn empty(el: &Element) -> Rendered {
    Rendered {
        chrome: Rc::new(Scene::new(el.w, el.h)),
        content: Rc::new(Scene::new(el.w, el.h)),
        content_rect: Rect::new(0.0, 0.0, el.w, el.h),
        data: None,
        props: Rc::new(Props::default()),
        cx: Rc::new(Context::default()),
        title: String::new(),
        millis: 0.0,
    }
}

fn compute(app: &mut VizApp, page: &Page, el: &Element, cx: Context, selected: &[Vec<(FieldRef, vizardcraft_core::scalar::Scalar)>]) -> Rendered {
    let p = &app.project;
    let built = &app.built;
    let env = FxEnv::new(p, built, &cx, &page.name);
    let mut props = fx::resolve_props(&env, el);
    let theme = &p.report.theme;
    match &el.kind {
        ElementKind::Visual(v) => {
            let data = Rc::new(vquery::run(p, built, v, &cx));
            fx::resolve_point_colors(&env, el, &data, &mut props);
            fx::resolve_cell_colors(&env, el, &data, &mut props);
            let chrome = charts::chrome::chrome(el, &props, theme);
            let title = charts::chrome::title_text(el, &props);
            let analytics = vizardcraft_core::analytics::compute(&v.analytics, &data);
            let script_image = app.report.script_images.get(&el.id).map(|s| s.as_str());
            let search = app.report.slicer_search.get(&el.id).cloned().unwrap_or_default();
            let o = RenderOpts {
                theme,
                props: &props,
                width: chrome.content.w,
                height: chrome.content.h,
                selected,
                analytics: &analytics,
                script_image,
                search: &search,
            };
            let content = charts::render(v, &data, &o);
            Rendered { chrome: Rc::new(chrome.scene), content: Rc::new(content), content_rect: chrome.content, data: Some(data), props: Rc::new(props), cx: Rc::new(cx), title, millis: 0.0 }
        }
        _ => {
            let chrome = charts::chrome::chrome(el, &props, theme);
            Rendered { chrome: Rc::new(chrome.scene), content: Rc::new(Scene::new(el.w, el.h)), content_rect: chrome.content, data: None, props: Rc::new(props), cx: Rc::new(cx), title: String::new(), millis: 0.0 }
        }
    }
}

/// Expands templates in text (titles, text boxes, shapes, buttons) in the page context.
pub fn template(app: &VizApp, page: &Page, el: &Element, text: &str) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let cx = state::context_for(&app.project, &app.built, &app.rt, page, el);
    FxEnv::new(&app.project, &app.built, &cx, &page.name).template(text)
}

/// Resolves a single property (fixed or fx) in an element's context.
pub fn resolve(app: &VizApp, page: &Page, el: &Element, prop: &Prop) -> Prop {
    if !prop.is_fx() {
        return prop.clone();
    }
    let cx = state::context_for(&app.project, &app.built, &app.rt, page, el);
    FxEnv::new(&app.project, &app.built, &cx, &page.name).resolve(prop)
}

/// Template expansion cached per model generation and context.
pub fn cached_template(app: &mut VizApp, page: &Page, el: &Element, text: &str) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let cx = state::context_for(&app.project, &app.built, &app.rt, page, el);
    let key = format!("{}|{}|{:?}|{}", app.generation, text, cx.filters, param_key(app));
    if let Some(v) = app.report.text_cache.get(&key) {
        return v.clone();
    }
    let out = FxEnv::new(&app.project, &app.built, &cx, &page.name).template(text);
    if app.report.text_cache.len() > 2000 {
        app.report.text_cache.clear();
    }
    app.report.text_cache.insert(key, out.clone());
    out
}
