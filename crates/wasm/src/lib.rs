//! The VizCraft engine for the browser: `vizcraft-core` + the publish engine compiled to
//! `wasm32-unknown-unknown`, with a tiny C-ABI JSON API the web viewer (`viewer.js`) calls.
//!
//! Every call takes one JSON argument (written into memory from [`alloc`]) and returns a pointer
//! to a JSON result whose length [`result_len`] gives (valid until the next call). Errors come
//! back as `{"error": "…"}`.
//!
//! | export | argument | result |
//! |---|---|---|
//! | `load` | `{project, tables, today}` | `{ok, pages, errors}` |
//! | `render_page` | `{page, mobile}` | a page: `{elements: [{id, x, y, w, h, svg, …}], filters, …}` |
//! | `click` | `{element, hit, ctrl}` (`hit` = a `data-hit` identity, or null) | outcome |
//! | `action` | `{element, action, ctrl}` (slicer items, matrix rows, button actions) | outcome |
//! | `slicer` | `{element, values?, range?, action?}` | outcome |
//! | `set_parameter` | `{name, value}` | outcome |
//! | `drill` | `{element, dir}` (`up`, `down`, `expand`, `mode`) | outcome |
//! | `apply_bookmark` | `{id}` | outcome (`page`) |
//! | `story_point` | `{story, index}` | outcome (`page`) |
//! | `navigate` / `back` / `reset` | `{page, remember}` / `{}` / `{}` | outcome |
//!
//! Build: `packaging/build-wasm.sh` (or `cargo build -p vizcraft-wasm --release --target
//! wasm32-unknown-unknown`).

use std::cell::RefCell;

use serde::Deserialize;
use serde::Serialize;
use vizcraft_core::build::TableData;
use vizcraft_core::doc::{FieldRef, Project};
use vizcraft_core::scalar::Scalar;
use vizcraft_publish::engine::{Engine, Outcome};

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Allocates `len` bytes for an argument.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Frees memory from [`alloc`].
///
/// # Safety
/// `ptr` must come from `alloc(len)` and not be freed twice.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: allocated by `alloc` with this capacity.
    unsafe { drop(Vec::from_raw_parts(ptr, 0, len.max(1))) };
}

/// The length of the last result.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn result_len() -> usize {
    RESULT.with(|r| r.borrow().len())
}

fn arg<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        return &[];
    }
    // SAFETY: the caller wrote `len` bytes at `ptr` (from `alloc`).
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

fn respond<T: Serialize>(v: &T) -> *const u8 {
    let bytes = serde_json::to_vec(v).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}").into_bytes());
    RESULT.with(|r| {
        *r.borrow_mut() = bytes;
        r.borrow().as_ptr()
    })
}

fn error(msg: impl Into<String>) -> *const u8 {
    respond(&serde_json::json!({ "error": msg.into() }))
}

fn with_engine<T: Serialize>(f: impl FnOnce(&mut Engine) -> T) -> *const u8 {
    ENGINE.with(|e| match e.borrow_mut().as_mut() {
        Some(eng) => respond(&f(eng)),
        None => error("No report is loaded."),
    })
}

fn parse<'a, T: Deserialize<'a>>(ptr: *const u8, len: usize) -> Result<T, *const u8> {
    serde_json::from_slice(arg(ptr, len)).map_err(|e| error(format!("Bad argument: {e}")))
}

/// Pure-Rust entry points (also used by the tests).
pub mod api {
    use super::*;

    #[derive(Deserialize)]
    pub struct Load {
        pub project: Project,
        #[serde(default)]
        pub tables: Vec<TableData>,
        #[serde(default)]
        pub today: Option<f64>,
    }

    pub fn load(a: Load) -> serde_json::Value {
        vizcraft_core::state::set_today_serial(a.today.filter(|t| *t > 0.0));
        let eng = Engine::from_data(a.project, a.tables);
        let errors: Vec<String> = eng.built.errors.iter().map(|(o, e)| format!("{o}: {e}")).collect();
        let pages = eng.p.report.pages.len();
        ENGINE.with(|e| *e.borrow_mut() = Some(eng));
        serde_json::json!({ "ok": true, "pages": pages, "errors": errors })
    }
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn load(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<api::Load>(ptr, len) {
        Ok(a) => respond(&api::load(a)),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct PageArg {
    page: u64,
    #[serde(default)]
    mobile: bool,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn render_page(ptr: *const u8, len: usize) -> *const u8 {
    let a: PageArg = match parse(ptr, len) {
        Ok(a) => a,
        Err(e) => return e,
    };
    ENGINE.with(|e| match e.borrow_mut().as_mut() {
        Some(eng) => match eng.render_page(a.page, a.mobile) {
            Some(p) => respond(&p),
            None => error(format!("There's no page {}.", a.page)),
        },
        None => error("No report is loaded."),
    })
}

#[derive(Deserialize)]
struct ClickArg {
    element: u64,
    #[serde(default)]
    hit: Option<Vec<(FieldRef, Scalar)>>,
    #[serde(default)]
    ctrl: bool,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn click(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<ClickArg>(ptr, len) {
        Ok(a) => with_engine(|e| e.click(a.element, a.hit, a.ctrl)),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct ActionArg {
    #[serde(default)]
    element: u64,
    action: String,
    #[serde(default)]
    ctrl: bool,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn action(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<ActionArg>(ptr, len) {
        Ok(a) => with_engine(|e| e.action(a.element, &a.action, a.ctrl)),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct SlicerArg {
    element: u64,
    #[serde(default)]
    values: Option<Vec<Scalar>>,
    #[serde(default)]
    range: Option<(f64, f64)>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    ctrl: bool,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn slicer(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<SlicerArg>(ptr, len) {
        Ok(a) => with_engine(|e| match (a.values, a.action) {
            (Some(v), _) => e.slicer_set(a.element, v, a.range),
            (None, Some(act)) => e.action(a.element, &act, a.ctrl),
            (None, None) => e.slicer_set(a.element, vec![], a.range),
        }),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct ParamArg {
    name: String,
    value: Scalar,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn set_parameter(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<ParamArg>(ptr, len) {
        Ok(a) => with_engine(|e| e.set_parameter(&a.name, a.value)),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct DrillArg {
    element: u64,
    dir: String,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn drill(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<DrillArg>(ptr, len) {
        Ok(a) => with_engine(|e| e.drill(a.element, &a.dir)),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct IdArg {
    id: u64,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn apply_bookmark(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<IdArg>(ptr, len) {
        Ok(a) => with_engine(|e| e.apply_bookmark(a.id)),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct StoryArg {
    story: u64,
    #[serde(default)]
    index: usize,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn story_point(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<StoryArg>(ptr, len) {
        Ok(a) => with_engine(|e| e.story_point(a.story, a.index)),
        Err(e) => e,
    }
}

#[derive(Deserialize)]
struct NavArg {
    page: u64,
    #[serde(default)]
    remember: bool,
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn navigate(ptr: *const u8, len: usize) -> *const u8 {
    match parse::<NavArg>(ptr, len) {
        Ok(a) => with_engine(|e| e.go_to(a.page, a.remember)),
        Err(e) => e,
    }
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn back(_ptr: *const u8, _len: usize) -> *const u8 {
    with_engine(Engine::back)
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn reset(_ptr: *const u8, _len: usize) -> *const u8 {
    with_engine(Engine::reset)
}

/// Keeps the outcome type in the public API docs.
pub type ActionOutcome = Outcome;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn call(f: extern "C" fn(*const u8, usize) -> *const u8, json: &str) -> serde_json::Value {
        let p = f(json.as_ptr(), json.len());
        let n = result_len();
        let out = unsafe { std::slice::from_raw_parts(p, n) }.to_vec();
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn engine_api_cross_filters() {
        let p = vizcraft_core::sample::sales();
        let b = vizcraft_core::build::build(&p, &vizcraft_query::NoHost, &Default::default());
        let tables: Vec<TableData> = b.tables.values().map(|t| (**t).clone()).collect();
        let load_arg = serde_json::json!({"project": vizcraft_publish::shareable_project(&p), "tables": tables, "today": 45500.0}).to_string();
        let r = call(load, &load_arg);
        assert_eq!(r["ok"], true, "{r}");
        let page = p.report.pages[0].id;
        let r = call(render_page, &format!("{{\"page\":{page}}}"));
        let els = r["elements"].as_array().unwrap();
        assert!(els.len() > 5);
        // Click the first bar of the product bar chart.
        let svg = els.iter().find(|e| e["visual"] == "clusteredBar").unwrap()["svg"].as_str().unwrap();
        let start = svg.find("data-hit=\"").unwrap() + 10;
        let end = start + svg[start..].find('"').unwrap();
        let hit = svg[start..end].replace("&quot;", "\"").replace("&amp;", "&");
        let bar_id = els.iter().find(|e| e["visual"] == "clusteredBar").unwrap()["id"].as_u64().unwrap();
        let r = call(click, &format!("{{\"element\":{bar_id},\"hit\":{hit}}}"));
        assert_eq!(r["changed"], true);
        let r = call(render_page, &format!("{{\"page\":{page}}}"));
        assert!(r["hasSelection"].as_bool().unwrap());
        assert!(r["filters"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().starts_with("Selected in")));
        let r = call(render_page, &format!("{{\"page\":{page},\"mobile\":true}}"));
        assert_eq!(r["width"], 360.0);
    }

}
