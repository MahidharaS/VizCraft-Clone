//! Query Dependencies: sources on the left, queries (by depth) in the middle, where they load
//! on the right.

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use vizardcraft_query::steps::LoadTo;

use crate::app::VizApp;
use crate::theme::Tokens;

const W: f32 = 190.0;
const H: f32 = 46.0;

/// The outside sources a query reads: (label, icon).
fn sources(m: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (f, label, icon) in [
        ("File.Contents", "File", "file-csv"),
        ("Folder.Files", "Folder", "folder"),
        ("Web.Contents", "Web", "web"),
        ("OData.Feed", "OData", "odata"),
        ("Sqlite.Database", "SQLite", "sqlite"),
        ("PostgreSQL.Database", "PostgreSQL", "postgresql"),
        ("MySQL.Database", "MySQL", "mysql"),
        ("Sql.Database", "SQL Server", "sqlserver"),
        ("Python.Execute", "Python", "python"),
        ("#table", "Entered data", "enter-data"),
    ] {
        if m.contains(f) {
            let path = m.split(&format!("{f}(\"")).nth(1).and_then(|r| r.split('"').next()).map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string());
            out.push((
                match path {
                    Some(p) if !p.is_empty() => format!("{label}: {p}"),
                    _ => label.to_string(),
                },
                icon,
            ));
        }
    }
    out
}

pub fn show(app: &mut VizApp, ctx: &egui::Context, t: &Tokens) {
    let doc = app.project.queries.clone();
    let sheets: Vec<String> = app.project.sheets.iter().map(|s| s.name.clone()).collect();
    let mut open = true;
    let screen = ctx.content_rect();
    egui::Window::new("Query Dependencies").open(&mut open).default_size(vec2((screen.width() - 120.0).min(1180.0), (screen.height() - 160.0).min(640.0))).show(ctx, |ui| {
        let mut depth: Vec<(String, usize)> = Vec::new();
        for q in &doc.queries {
            let mut d = 0;
            let mut stack = vec![(q.name.clone(), 0usize)];
            while let Some((n, k)) = stack.pop() {
                d = d.max(k);
                if k < 20 {
                    for dep in doc.dependencies(&n) {
                        if doc.query(&dep).is_some() {
                            stack.push((dep, k + 1));
                        }
                    }
                }
            }
            depth.push((q.name.clone(), d));
        }
        let max_depth = depth.iter().map(|x| x.1).max().unwrap_or(0);
        let mut srcs: Vec<(String, &str)> = Vec::new();
        for q in &doc.queries {
            for s in sources(&q.m) {
                if !srcs.iter().any(|x| x.0 == s.0) {
                    srcs.push(s);
                }
            }
            for s in &sheets {
                if steps_use(&q.m, s) && !srcs.iter().any(|x| x.0 == format!("Sheet: {s}")) {
                    srcs.push((format!("Sheet: {s}"), "sheet"));
                }
            }
        }
        let rows = srcs.len().max(depth.len()).max(3);
        let size = vec2((max_depth as f32 + 3.0) * (W + 70.0) + 40.0, rows as f32 * (H + 20.0) + 40.0);
        egui::ScrollArea::both().show(ui, |ui| {
            let (canvas, _) = ui.allocate_exact_size(size.max(ui.available_size()), Sense::hover());
            let painter = ui.painter_at(canvas);
            painter.rect_filled(canvas, 0.0, t.canvas);
            let col_x = |c: usize| canvas.min.x + 20.0 + c as f32 * (W + 70.0);
            let mut pos: Vec<(String, Pos2, &str, &str)> = Vec::new();
            for (i, (s, icon)) in srcs.iter().enumerate() {
                pos.push((format!("src:{s}"), pos2(col_x(0), canvas.min.y + 20.0 + i as f32 * (H + 20.0)), "source", icon));
            }
            for d in 0..=max_depth {
                for (i, (n, _)) in depth.iter().filter(|x| x.1 == d).enumerate() {
                    pos.push((n.clone(), pos2(col_x(1 + d), canvas.min.y + 20.0 + i as f32 * (H + 20.0)), "query", "query"));
                }
            }
            let dest_col = 2 + max_depth;
            pos.push(("dest:model".into(), pos2(col_x(dest_col), canvas.min.y + 20.0), "dest", "model"));
            pos.push(("dest:none".into(), pos2(col_x(dest_col), canvas.min.y + 20.0 + H + 20.0), "dest", "settings"));
            let at = |k: &str| pos.iter().find(|(n, ..)| n == k).map(|(_, p, ..)| *p);
            let mut edges: Vec<(String, String)> = Vec::new();
            for q in &doc.queries {
                for (s, _) in sources(&q.m) {
                    edges.push((format!("src:{s}"), q.name.clone()));
                }
                for s in &sheets {
                    if steps_use(&q.m, s) {
                        edges.push((format!("src:Sheet: {s}"), q.name.clone()));
                    }
                }
                for dep in doc.dependencies(&q.name) {
                    if doc.query(&dep).is_some() {
                        edges.push((dep, q.name.clone()));
                    }
                }
                edges.push((q.name.clone(), if q.load == LoadTo::None { "dest:none".into() } else { "dest:model".into() }));
            }
            let sel = app.pq.query.clone();
            for (a, b) in &edges {
                if let (Some(p), Some(q)) = (at(a), at(b)) {
                    let hot = *a == sel || *b == sel;
                    let from = p + vec2(W, H / 2.0);
                    let to = q + vec2(0.0, H / 2.0);
                    let mid = (from.x + to.x) / 2.0;
                    let pts = vec![from, pos2(mid, from.y), pos2(mid, to.y), to];
                    painter.add(egui::Shape::line(pts, Stroke::new(if hot { 2.0 } else { 1.2 }, if hot { t.accent } else { t.muted.gamma_multiply(0.7) })));
                    painter.add(egui::Shape::convex_polygon(vec![to, to + vec2(-6.0, -4.0), to + vec2(-6.0, 4.0)], if hot { t.accent } else { t.muted }, Stroke::NONE));
                }
            }
            for (k, p, kind, icon) in &pos {
                let r = Rect::from_min_size(*p, vec2(W, H));
                let (fill, label, sub) = match *kind {
                    "source" => (t.panel, k.trim_start_matches("src:").to_string(), "Source"),
                    "dest" => (t.accent_soft, if k == "dest:model" { "Data model".to_string() } else { "Connection only".to_string() }, "Load"),
                    _ => (if *k == sel { t.accent_soft } else { t.panel }, k.clone(), "Query"),
                };
                painter.rect(r, 6.0, fill, Stroke::new(if *k == sel { 2.0 } else { 1.0 }, if *k == sel { t.accent } else { t.border }), StrokeKind::Inside);
                super::icons::paint(&painter, Rect::from_min_size(r.min + vec2(10.0, 13.0), vec2(20.0, 20.0)), icon, t.accent, t.dark);
                let clip = painter.with_clip_rect(r.shrink(4.0));
                clip.text(r.min + vec2(38.0, 15.0), Align2::LEFT_CENTER, label, FontId::proportional(12.5), t.text);
                clip.text(r.min + vec2(38.0, 32.0), Align2::LEFT_CENTER, sub, FontId::proportional(10.5), t.muted);
                if *kind == "query" {
                    let resp = ui.interact(r, ui.scope_id().with(("pq_dep", k)), Sense::click());
                    if resp.on_hover_text("Open this query").clicked() {
                        app.pq.query = k.clone();
                        app.pq.step = None;
                    }
                }
            }
        });
    });
    if !open {
        app.pq.deps = false;
    }
}

fn steps_use(m: &str, name: &str) -> bool {
    vizardcraft_query::steps::identifiers(m).iter().any(|i| i == name)
}
