//! Publishing: turns a project into a web report.
//!
//! [`bundle`] produces a static site — `index.html` (the viewer shell: page tabs, desktop/mobile
//! toggle, filter pane, bookmarks, story player, tooltips), `viewer.js`, `viewer.css`,
//! `report.json` (the report and a snapshot of its data, so it works offline), `thumbnail.svg`
//! and, in interactive mode, `vizardcraft_viewer.wasm` (the VizardCraft engine compiled to
//! WebAssembly: slicers, cross-filtering, drill, parameters all work in the browser).
//! Without the engine every page is pre-rendered to SVG in its desktop and phone layouts.
//!
//! [`single_file`] inlines all of it into one HTML file (share it like an artifact). The
//! [`net`] module sends bundles to a VizardCraft Service, GitHub Pages or a folder.
//!
//! [`render::render_element`] is the one function that draws an element (visual, shape, text,
//! image, button) the same way everywhere.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod engine;
pub mod render;

#[cfg(not(target_arch = "wasm32"))]
pub mod net;

use std::collections::BTreeMap;

use vizardcraft_core::build::{Built, TableData};
use vizardcraft_core::doc::*;

pub use engine::{Engine, ReportData};
pub use render::{element_svg, page_svg, render_element};

pub const VIEWER_JS: &str = include_str!("assets/viewer.js");
pub const VIEWER_CSS: &str = include_str!("assets/viewer.css");
const INDEX_HTML: &str = include_str!("assets/index.html");

/// The WASM viewer engine, when it was built before this crate (see `packaging/build-wasm.sh`).
#[cfg(has_viewer_wasm)]
pub const VIEWER_WASM: Option<&[u8]> = Some(include_bytes!(env!("VIZARDCRAFT_VIEWER_WASM_PATH")));
#[cfg(not(has_viewer_wasm))]
pub const VIEWER_WASM: Option<&[u8]> = None;

/// Whether this build can publish interactive reports.
pub fn has_wasm() -> bool {
    VIEWER_WASM.is_some()
}

#[derive(Clone, Debug)]
pub struct BundleOptions {
    /// Ship the WASM engine with the data (falls back to static when it isn't built in).
    pub interactive: bool,
    /// Pre-render the phone layout.
    pub include_mobile: bool,
    /// Pages to include (empty = every report page).
    pub pages: Vec<u64>,
    /// Title shown in the header (default: the project name).
    pub title: Option<String>,
    /// Use this engine instead of the built-in one.
    pub wasm: Option<Vec<u8>>,
}

impl Default for BundleOptions {
    fn default() -> Self {
        BundleOptions { interactive: true, include_mobile: true, pages: vec![], title: None, wasm: None }
    }
}

impl BundleOptions {
    pub fn from_project(p: &Project) -> BundleOptions {
        BundleOptions { interactive: p.publish.interactive, include_mobile: p.publish.include_mobile, ..BundleOptions::default() }
    }
}

/// The files of a bundle: (path, bytes).
pub type Files = Vec<(String, Vec<u8>)>;

/// What a published report may carry: no queries (connection strings), no sheets (the data is
/// in the table snapshot), no publish targets (tokens).
pub fn shareable_project(p: &Project) -> Project {
    let mut q = p.clone();
    q.queries = Default::default();
    q.sheets.clear();
    q.flows.clear();
    q.publish.targets.clear();
    q
}

fn now_rfc3339() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        rfc3339(secs)
    }
    #[cfg(target_arch = "wasm32")]
    {
        String::new()
    }
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

/// The report data (`report.json`) for a project.
pub fn report_data(p: &Project, built: &Built, opts: &BundleOptions) -> ReportData {
    let wasm_available = opts.wasm.is_some() || has_wasm();
    let interactive = opts.interactive && wasm_available;
    let included: Vec<&Page> = p.report.pages.iter().filter(|pg| opts.pages.is_empty() || opts.pages.contains(&pg.id)).filter(|pg| pg.kind != PageKind::Tooltip).collect();
    let mut engine = Engine::new(p.clone(), built.clone());
    let mut views = BTreeMap::new();
    for pg in &included {
        let mut e = engine.clone();
        e.rt.page = pg.id;
        if let Some(d) = e.render_page(pg.id, false) {
            let m = if opts.include_mobile { e.render_page(pg.id, true) } else { None };
            views.insert(format!("page:{}", pg.id), engine::View { desktop: d, mobile: m });
        }
    }
    for bm in &p.report.bookmarks {
        let mut e = engine.clone();
        let out = e.apply_bookmark(bm.id);
        let pid = out.page.unwrap_or(bm.page);
        if let Some(d) = e.render_page(pid, false) {
            let m = if opts.include_mobile { e.render_page(pid, true) } else { None };
            views.insert(format!("bm:{}", bm.id), engine::View { desktop: d, mobile: m });
        }
    }
    for st in &p.report.stories {
        for (i, _) in st.points.iter().enumerate() {
            let mut e = engine.clone();
            let out = e.story_point(st.id, i);
            let Some(pid) = out.page else { continue };
            if let Some(d) = e.render_page(pid, false) {
                let m = if opts.include_mobile { e.render_page(pid, true) } else { None };
                views.insert(format!("sp:{}:{i}", st.id), engine::View { desktop: d, mobile: m });
            }
        }
    }
    engine.rt = Default::default();
    ReportData {
        format: 1,
        name: opts.title.clone().unwrap_or_else(|| p.name.clone()),
        published: now_rfc3339(),
        generator: format!("VizardCraft {}", env!("CARGO_PKG_VERSION")),
        interactive,
        include_mobile: opts.include_mobile,
        pages: included
            .iter()
            .map(|pg| engine::PageInfo { id: pg.id, name: pg.name.clone(), kind: format!("{:?}", pg.kind), hidden: pg.hidden || !engine::tab_kind(pg.kind) })
            .collect(),
        bookmarks: p.report.bookmarks.iter().map(|b| engine::BookmarkInfo { id: b.id, name: b.name.clone(), group: b.group.clone(), page: b.page }).collect(),
        stories: p
            .report
            .stories
            .iter()
            .filter(|s| !s.points.is_empty())
            .map(|s| engine::StoryInfo {
                id: s.id,
                name: s.name.clone(),
                title: s.title.clone(),
                points: s.points.iter().map(|pt| engine::StoryPointInfo { caption: pt.caption.clone(), page: pt.page }).collect(),
            })
            .collect(),
        parameters: if interactive { p.model.parameters.iter().filter_map(engine::ParamInfo::of).collect() } else { vec![] },
        today: vizardcraft_core::state::today_serial(),
        views,
        project: if interactive { Some(shareable_project(p)) } else { None },
        tables: if interactive { built.tables.values().map(|t| TableData::clone(t)).collect() } else { vec![] },
    }
}

fn html_escape(s: &str) -> String {
    render::esc(s)
}

/// JSON safe to put inside `<script>`.
fn script_json(json: &str) -> String {
    json.replace("</", "<\\/").replace("<!--", "<\\!--")
}

fn index_html(data: &ReportData, inline: Option<(&str, Option<&str>)>) -> String {
    let title = html_escape(&data.name);
    let (head, tail) = match inline {
        None => ("<link rel=\"stylesheet\" href=\"viewer.css\">".to_string(), "<script src=\"viewer.js\"></script>".to_string()),
        Some((json, wasm_b64)) => {
            let mut t = format!("<script>window.VIZARDCRAFT_REPORT={};</script>", script_json(json));
            if let Some(w) = wasm_b64 {
                t += &format!("<script>window.VIZARDCRAFT_WASM_B64=\"{w}\";</script>");
            }
            t += &format!("<script>{}</script>", VIEWER_JS.replace("</script", "<\\/script"));
            (format!("<style>{VIEWER_CSS}</style>"), t)
        }
    };
    INDEX_HTML.replace("{{TITLE}}", &title).replace("{{HEAD}}", &head).replace("{{SCRIPTS}}", &tail)
}

/// Builds the static site of a report.
pub fn bundle(p: &Project, built: &Built, opts: &BundleOptions) -> Result<Files, String> {
    let data = report_data(p, built, opts);
    let json = serde_json::to_string(&data).map_err(|e| format!("Couldn't write the report data: {e}"))?;
    let mut files: Files = vec![
        ("index.html".into(), index_html(&data, None).into_bytes()),
        ("viewer.js".into(), VIEWER_JS.as_bytes().to_vec()),
        ("viewer.css".into(), VIEWER_CSS.as_bytes().to_vec()),
        ("report.json".into(), json.into_bytes()),
    ];
    if data.interactive
        && let Some(w) = opts.wasm.as_deref().or(VIEWER_WASM)
    {
        files.push(("vizardcraft_viewer.wasm".into(), w.to_vec()));
    }
    if let Some(pg) = data.pages.iter().find(|pg| !pg.hidden).or(data.pages.first()).and_then(|pi| p.page(pi.id)) {
        let rt = vizardcraft_core::state::Runtime { page: pg.id, ..Default::default() };
        files.push(("thumbnail.svg".into(), page_svg(p, built, &rt, pg, false).into_bytes()));
    }
    Ok(files)
}

/// One self-contained HTML file: the viewer, the data and (interactive) the engine inlined.
pub fn single_file(p: &Project, built: &Built, opts: &BundleOptions) -> Result<String, String> {
    let data = report_data(p, built, opts);
    let json = serde_json::to_string(&data).map_err(|e| format!("Couldn't write the report data: {e}"))?;
    let wasm = if data.interactive { opts.wasm.as_deref().or(VIEWER_WASM).map(b64_encode) } else { None };
    Ok(index_html(&data, Some((&json, wasm.as_deref()))))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, k) in idx.iter().enumerate() {
            if i <= chunk.len() {
                out.push(B64.get(*k as usize).copied().unwrap_or(b'A') as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for s in ["", "a", "ab", "abc", "abcd", "hello world!"] {
            let e = b64_encode(s.as_bytes());
            assert_eq!(b64_decode(&e).unwrap(), s.as_bytes());
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn sample_bundles() {
        let p = vizardcraft_core::sample::sales();
        let b = vizardcraft_core::build::build(&p, &vizardcraft_query::NoHost, &Default::default());
        let files = bundle(&p, &b, &BundleOptions { interactive: false, ..BundleOptions::default() }).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"index.html") && names.contains(&"report.json") && names.contains(&"thumbnail.svg"));
        let json = &files.iter().find(|(n, _)| n == "report.json").unwrap().1;
        let data: ReportData = serde_json::from_slice(json).unwrap();
        assert!(data.views.len() >= 3);
        let first = data.views.values().next().unwrap();
        assert!(!first.desktop.elements.is_empty());
        assert!(first.mobile.as_ref().is_some_and(|m| m.width == render::PHONE_WIDTH));
    }
}
