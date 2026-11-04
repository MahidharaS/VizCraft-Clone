//! The preview grid: sticky headers with the column type, a filter dropdown, the quality bar and
//! value distribution; virtual rows; column selection and menus; the Column From Examples
//! column; the column profile panel.

use std::rc::Rc;

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use vizardcraft_query::examples::suggest;
use vizardcraft_query::profile::ColumnProfile;
use vizardcraft_query::steps::field;
use vizardcraft_query::value::quote_text;
use vizardcraft_query::{Evaluator, MType, NoHost, Table, Value};

use super::dialogs::{self, COND_OPS, Dialog};
use super::{Examples, add_step, ops};
use crate::app::VizApp;
use crate::theme::Tokens;

const ROW_H: f32 = 22.0;
const NUM_W: f32 = 44.0;
const TYPE_W: f32 = 30.0;

/// The open column filter menu (the header dropdown).
pub struct FilterMenu {
    pub col: String,
    pub values: Vec<(Value, bool)>,
    pub search: String,
    pub at: egui::Pos2,
    pub ty: MType,
    /// More values exist than the list shows.
    pub truncated: bool,
}

fn header_h(app: &VizApp) -> (f32, f32, f32) {
    let v = app.pq.view;
    (30.0, if v.quality { 36.0 } else { 0.0 }, if v.distribution { 50.0 } else { 0.0 })
}

fn col_width(t: &Table, c: usize) -> f32 {
    let head = t.columns.get(c).map(|n| n.chars().count() as f32 * 7.4 + TYPE_W + 42.0).unwrap_or(120.0);
    let mut chars = 0;
    for r in t.rows.iter().take(60) {
        chars = chars.max(r.get(c).map(|v| v.display().chars().count().min(42)).unwrap_or(0));
    }
    head.max(chars as f32 * 7.0 + 18.0).clamp(130.0, 340.0)
}

fn colors(t: &Tokens) -> (Color32, Color32, Color32) {
    (t.good, t.danger, t.muted.gamma_multiply(0.6))
}

pub fn show(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(p) = super::preview(app) else { return };
    if let Some(e) = &p.error {
        error_view(app, ui, t, e);
        return;
    }
    if let Some(v) = &p.scalar {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(v).size(26.0).color(t.text));
            ui.label(RichText::new("This step returns a single value").color(t.muted));
        });
        return;
    }
    if app.pq.examples.is_some() {
        examples_bar(app, ui, t, &p.table);
    }
    let profile_col = app.pq.selected.first().and_then(|c| p.table.column_index(c)).filter(|_| app.pq.view.profile);
    if let Some(c) = profile_col {
        egui::Panel::bottom("pq_profile").resizable(true).default_size(190.0).min_size(120.0).frame(egui::Frame::new().fill(t.panel).inner_margin(10).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
            profile_panel(app, ui, t, &p.table, c);
        });
    }
    table_view(app, ui, t, &p.table);
    filter_menu(app, ui.ctx(), t, &p.table);
}

fn error_view(app: &mut VizApp, ui: &mut Ui, t: &Tokens, e: &str) {
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        let warn = Color32::from_rgb(0xB4, 0x53, 0x09);
        egui::Frame::new().fill(warn.gamma_multiply(0.08)).stroke(Stroke::new(1.0, warn.gamma_multiply(0.5))).inner_margin(14).corner_radius(6.0).show(ui, |ui| {
            ui.set_max_width(720.0);
            ui.label(RichText::new("This step has an error").strong().color(warn));
            ui.add_space(4.0);
            ui.label(RichText::new(e).color(t.text));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Go to the previous step").clicked() {
                    let steps = super::steps(app);
                    let cur = app.pq.step.clone().or_else(|| steps.last().map(|s| s.name.clone()));
                    if let Some(i) = steps.iter().position(|s| Some(&s.name) == cur.as_ref())
                        && i > 0
                    {
                        app.pq.step = steps.get(i - 1).map(|s| s.name.clone());
                    }
                }
                if ui.button("Edit in Advanced Editor").clicked() {
                    app.pq.dialog = Some(Dialog::advanced(app));
                }
                if ui.button("Refresh Preview").clicked() {
                    app.pq.refresh_preview();
                }
            });
        });
    });
}

fn draw_quality(painter: &egui::Painter, t: &Tokens, rect: Rect, p: &ColumnProfile) {
    let (g, r, e) = colors(t);
    let total = p.count.max(1) as f32;
    let bar = Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(rect.width() - 12.0, 3.0));
    let wv = bar.width() * p.valid() as f32 / total;
    let we = bar.width() * p.errors as f32 / total;
    painter.rect_filled(Rect::from_min_size(bar.min, vec2(wv, 3.0)), 0.0, g);
    painter.rect_filled(Rect::from_min_size(bar.min + vec2(wv, 0.0), vec2(we, 3.0)), 0.0, r);
    painter.rect_filled(Rect::from_min_size(bar.min + vec2(wv + we, 0.0), vec2(bar.width() - wv - we, 3.0)), 0.0, e);
    let pct = |n: usize| {
        let x = n as f32 * 100.0 / total;
        if n > 0 && x < 1.0 { "< 1%".to_string() } else { format!("{x:.0}%") }
    };
    let font = FontId::proportional(10.0);
    let y = rect.min.y + 15.0;
    let dot = |x: f32, y: f32, c: Color32| painter.circle_filled(pos2(x, y), 2.5, c);
    dot(rect.min.x + 9.0, y, g);
    painter.text(pos2(rect.min.x + 15.0, y), Align2::LEFT_CENTER, format!("Valid  {}", pct(p.valid())), font.clone(), t.text);
    dot(rect.min.x + 9.0, y + 11.0, r);
    painter.text(pos2(rect.min.x + 15.0, y + 11.0), Align2::LEFT_CENTER, format!("Error  {}", pct(p.errors)), font.clone(), t.muted);
    dot(rect.center().x + 6.0, y + 11.0, e);
    painter.text(pos2(rect.center().x + 12.0, y + 11.0), Align2::LEFT_CENTER, format!("Empty  {}", pct(p.empty)), font, t.muted);
}

fn draw_distribution(painter: &egui::Painter, t: &Tokens, rect: Rect, p: &ColumnProfile) {
    let bars: Vec<usize> = p.top.iter().take(24).map(|(_, n)| *n).collect();
    let max = bars.iter().copied().max().unwrap_or(1).max(1) as f32;
    let area = Rect::from_min_max(rect.min + vec2(6.0, 4.0), pos2(rect.max.x - 6.0, rect.max.y - 16.0));
    let n = bars.len().max(1) as f32;
    let w = (area.width() / n).min(12.0);
    for (i, b) in bars.iter().enumerate() {
        let h = (area.height() * *b as f32 / max).max(1.5);
        let r = Rect::from_min_size(pos2(area.min.x + i as f32 * w, area.max.y - h), vec2((w - 2.0).max(1.0), h));
        painter.rect_filled(r, 1.0, t.accent.gamma_multiply(0.8));
    }
    painter.text(pos2(rect.min.x + 6.0, rect.max.y - 7.0), Align2::LEFT_CENTER, format!("{} distinct, {} unique", p.distinct, p.unique), FontId::proportional(10.0), t.muted);
}

fn cell_text(v: &Value, ws: bool) -> String {
    let s = v.display();
    if ws { s.replace(' ', "·").replace('\n', "¶").replace('\t', "→") } else { s.replace('\n', " ") }
}

/// The header's type badge text.
fn type_badge(ty: MType) -> &'static str {
    match ty {
        MType::Any => "ABC\n123",
        MType::Number => "1.2",
        MType::Int64 => "123",
        MType::Currency => "$",
        MType::Percentage => "%",
        MType::Text => "ABC",
        MType::Logical => "T/F",
        MType::Date => "@date",
        MType::DateTime => "@datetime",
        MType::Time => "@time",
        MType::Duration => "@duration",
        MType::Table => "@query",
        MType::Record => "{ }",
        MType::List => "@to-list",
        MType::Binary => "01",
        _ => "?",
    }
}

#[derive(Default)]
struct Clicks {
    header: Option<(usize, bool)>,
    rename: Option<String>,
    filter: Option<(String, egui::Pos2)>,
    nav: Option<(usize, String)>,
    expand: Option<String>,
}

fn table_view(app: &mut VizApp, ui: &mut Ui, t: &Tokens, table: &Rc<Table>) {
    let (hh, qh, dh) = header_h(app);
    let head = hh + qh + dh;
    let widths: Vec<f32> = (0..table.columns.len()).map(|c| col_width(table, c)).collect();
    let ex_w = if app.pq.examples.is_some() { 210.0 } else { 0.0 };
    let total_w = NUM_W + widths.iter().sum::<f32>() + ex_w + 2.0;
    let n = table.rows.len();
    let total_h = head + n as f32 * ROW_H + 8.0;
    let mono = app.pq.view.monospace;
    let ws = app.pq.view.whitespace;
    let font = if mono { FontId::monospace(12.0) } else { FontId::proportional(12.5) };
    let selected = app.pq.selected.clone();
    let profiles: Vec<ColumnProfile> = if qh + dh > 0.0 { (0..table.columns.len()).map(|c| app.pq.profile(table, c)).collect() } else { vec![] };
    let predicted = app.pq.examples.as_ref().map(|e| predictions(e, table));
    let mut clicks = Clicks::default();
    let grid_bg = t.panel;
    egui::ScrollArea::both().id_salt(("pq_grid", app.pq.query.as_str())).auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let (whole, _) = ui.allocate_exact_size(vec2(total_w.max(viewport.width()), total_h.max(viewport.height())), Sense::hover());
        let origin = whole.min;
        let painter = ui.painter().clone();
        painter.rect_filled(Rect::from_min_size(origin, vec2(total_w, total_h)), 0.0, grid_bg);
        let first = ((viewport.min.y - head) / ROW_H).floor().max(0.0) as usize;
        let last = (((viewport.max.y - head) / ROW_H).ceil().max(0.0) as usize + 1).min(n);
        for r in first..last {
            let y = origin.y + head + r as f32 * ROW_H;
            if r % 2 == 1 {
                painter.rect_filled(Rect::from_min_size(pos2(origin.x, y), vec2(total_w, ROW_H)), 0.0, t.panel_alt);
            }
            let mut x = origin.x + NUM_W;
            for (c, w) in widths.iter().enumerate() {
                let cell = Rect::from_min_size(pos2(x, y), vec2(*w, ROW_H));
                if x + w >= origin.x + viewport.min.x && x <= origin.x + viewport.max.x {
                    let name = table.columns.get(c).cloned().unwrap_or_default();
                    if selected.contains(&name) {
                        painter.rect_filled(cell, 0.0, t.accent_soft.gamma_multiply(0.7));
                    }
                    let v = table.cell(r, c);
                    let (text, color, right, italic) = match v {
                        Value::Error(_) => ("Error".to_string(), t.danger, false, false),
                        Value::Null => ("null".to_string(), t.muted, false, true),
                        Value::Number(_) => (cell_text(v, ws), t.text, true, false),
                        Value::Table(_) => ("Table".to_string(), t.accent, false, false),
                        Value::List(_) => ("List".to_string(), t.accent, false, false),
                        Value::Record(_) => ("Record".to_string(), t.accent, false, false),
                        Value::Binary(_) => ("Binary".to_string(), t.accent, false, false),
                        Value::Date(_) | Value::DateTime(_) | Value::Time(_) | Value::Duration(_) => (cell_text(v, ws), t.text, true, false),
                        _ => (cell_text(v, ws), t.text, false, false),
                    };
                    let pos = if right { cell.right_center() - vec2(7.0, 0.0) } else { cell.left_center() + vec2(7.0, 0.0) };
                    let f = if italic { FontId::proportional(12.0) } else { font.clone() };
                    painter.with_clip_rect(cell.shrink2(vec2(3.0, 0.0)).intersect(ui.clip_rect())).text(pos, if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }, text, f, color);
                    if matches!(v, Value::Table(_) | Value::List(_) | Value::Record(_) | Value::Error(_) | Value::Binary(_)) {
                        let resp = ui.interact(cell, ui.scope_id().with(("pq_cell", r, c)), Sense::click());
                        let tip = match v {
                            Value::Error(e) => e.to_string(),
                            Value::Table(nt) => format!("Table: {} columns × {} rows. Click to drill in; use Expand to bring its columns in.", nt.columns.len(), nt.rows.len()),
                            _ => "Click to drill into this value".to_string(),
                        };
                        if resp.on_hover_text(tip).clicked() && !v.is_error() {
                            clicks.nav = Some((r, name));
                        }
                    }
                    painter.vline(cell.right(), cell.y_range(), Stroke::new(1.0, t.border.gamma_multiply(0.7)));
                }
                x += w;
            }
            if let Some(pred) = &predicted {
                example_cell(app, ui, t, Rect::from_min_size(pos2(x, y), vec2(ex_w, ROW_H)), r, pred.get(r).cloned().unwrap_or_default());
            }
            // Row numbers (sticky on the left).
            let nx = origin.x + (viewport.min.x).max(0.0);
            painter.rect_filled(Rect::from_min_size(pos2(nx, y), vec2(NUM_W, ROW_H)), 0.0, t.panel_alt);
            painter.text(pos2(nx + NUM_W - 8.0, y + ROW_H / 2.0), Align2::RIGHT_CENTER, (r + 1).to_string(), FontId::proportional(11.0), t.muted);
            painter.hline(origin.x..=origin.x + total_w, y + ROW_H, Stroke::new(1.0, t.border.gamma_multiply(0.6)));
        }
        // Sticky header.
        let hy = origin.y + viewport.min.y;
        painter.rect_filled(Rect::from_min_size(pos2(origin.x, hy), vec2(total_w.max(viewport.width()), head)), 0.0, t.panel);
        let mut x = origin.x + NUM_W;
        for (c, w) in widths.iter().enumerate() {
            let Some(name) = table.columns.get(c).cloned() else { continue };
            let rect = Rect::from_min_size(pos2(x, hy), vec2(*w, hh));
            let sel = selected.contains(&name);
            if sel {
                painter.rect_filled(Rect::from_min_size(rect.min, vec2(*w, head)), 0.0, t.accent_soft);
            }
            let ty = table.types.get(c).copied().unwrap_or_default();
            let badge = Rect::from_min_size(rect.min + vec2(5.0, 6.0), vec2(TYPE_W - 4.0, hh - 12.0));
            let badge_r = ui.interact(badge, ui.scope_id().with(("pq_type", c)), Sense::click()).on_hover_text(format!("{} (click to change the type)", ty.name()));
            if badge_r.hovered() {
                painter.rect_filled(badge.expand(1.0), 3.0, t.accent_soft);
            }
            let bt = type_badge(ty);
            if let Some(icon) = bt.strip_prefix('@') {
                super::icons::paint(&painter, badge.shrink(2.0), icon, t.muted, t.dark);
            } else if bt.contains('\n') {
                let (a, b) = bt.split_once('\n').unwrap_or((bt, ""));
                painter.text(badge.center() - vec2(0.0, 4.0), Align2::CENTER_CENTER, a, FontId::proportional(8.0), t.muted);
                painter.text(badge.center() + vec2(0.0, 4.5), Align2::CENTER_CENTER, b, FontId::proportional(8.0), t.muted);
            } else {
                painter.text(badge.center(), Align2::CENTER_CENTER, bt, FontId::proportional(if bt.chars().count() > 2 { 9.5 } else { 11.5 }), if ty.is_numeric() { t.accent } else { t.muted });
            }
            let drop = Rect::from_min_size(pos2(rect.max.x - 22.0, rect.min.y + 6.0), vec2(17.0, hh - 12.0));
            let drop_r = ui.interact(drop, ui.scope_id().with(("pq_drop", c)), Sense::click()).on_hover_text("Sort and filter");
            if drop_r.hovered() {
                painter.rect(drop, 3.0, t.accent_soft, Stroke::new(1.0, t.border), StrokeKind::Inside);
            }
            super::forms::caret(&painter, drop.center(), 3.5, if drop_r.hovered() { t.accent } else { t.muted }, 'd');
            let name_rect = Rect::from_min_max(pos2(badge.max.x + 6.0, rect.min.y), pos2(drop.min.x - 2.0, rect.max.y));
            let structured = matches!(ty, MType::Table | MType::Record) || table.rows.first().and_then(|r| r.get(c)).is_some_and(|v| matches!(v, Value::Table(_) | Value::Record(_)));
            let name_end = if structured { name_rect.max.x - 18.0 } else { name_rect.max.x };
            painter.with_clip_rect(Rect::from_min_max(name_rect.min, pos2(name_end, name_rect.max.y)).intersect(ui.clip_rect())).text(
                name_rect.left_center(),
                Align2::LEFT_CENTER,
                &name,
                FontId::proportional(12.5),
                if sel { t.accent } else { t.text },
            );
            if structured {
                let er = Rect::from_center_size(pos2(name_end + 9.0, rect.center().y), vec2(16.0, 16.0));
                let resp = ui.interact(er, ui.scope_id().with(("pq_expand", c)), Sense::click()).on_hover_text("Expand this column");
                super::icons::paint(&painter, er, "expand-column", if resp.hovered() { t.accent } else { t.muted }, t.dark);
                if resp.clicked() {
                    clicks.expand = Some(name.clone());
                }
            }
            let name_r = ui.interact(name_rect, ui.scope_id().with(("pq_head", c)), Sense::click());
            if name_r.clicked() {
                clicks.header = Some((c, ui.input(|i| i.modifiers.command || i.modifiers.shift)));
            }
            if name_r.double_clicked() {
                clicks.rename = Some(name.clone());
            }
            if name_r.secondary_clicked() && !sel {
                clicks.header = Some((c, false));
            }
            name_r.context_menu(|ui| {
                ops::column_menu(app, ui, &name);
            });
            if drop_r.clicked() {
                clicks.filter = Some((name.clone(), pos2(drop.max.x - 240.0, drop.max.y + 2.0)));
            }
            egui::Popup::menu(&badge_r).show(|ui| {
                for ty in MType::COLUMN_TYPES {
                    if ui.button(ty.name()).clicked() {
                        ops::change_type(app, std::slice::from_ref(&name), ty);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Using Locale…").on_hover_text("Detect the type from the data").clicked() {
                    app.pq.selected = vec![name.clone()];
                    ops::detect_types(app);
                    ui.close();
                }
            });
            if let Some(p) = profiles.get(c) {
                let clip = painter.with_clip_rect(Rect::from_min_size(pos2(x, hy), vec2(*w, head)).intersect(ui.clip_rect()));
                if qh > 0.0 {
                    draw_quality(&clip, t, Rect::from_min_size(pos2(x, hy + hh), vec2(*w, qh)), p);
                }
                if dh > 0.0 {
                    draw_distribution(&clip, t, Rect::from_min_size(pos2(x, hy + hh + qh), vec2(*w, dh)), p);
                }
            }
            painter.vline(x + w, hy..=hy + head, Stroke::new(1.0, t.border));
            x += w;
        }
        if let Some(ex) = &mut app.pq.examples {
            let rect = Rect::from_min_size(pos2(x, hy), vec2(ex_w, head));
            painter.rect_filled(rect, 0.0, t.accent_soft);
            ui.put(Rect::from_min_size(rect.min + vec2(6.0, 4.0), vec2(ex_w - 12.0, 22.0)), egui::TextEdit::singleline(&mut ex.name).hint_text("Column name"));
            painter.text(rect.min + vec2(8.0, 34.0), Align2::LEFT_TOP, "Type examples below", FontId::proportional(11.0), t.muted);
        }
        let nx = origin.x + viewport.min.x.max(0.0);
        painter.rect_filled(Rect::from_min_size(pos2(nx, hy), vec2(NUM_W, head)), 0.0, t.panel_alt);
        // The corner: a table menu.
        let corner = Rect::from_min_size(pos2(nx + 6.0, hy + 6.0), vec2(NUM_W - 12.0, hh - 12.0));
        let cr = ui.interact(corner, ui.scope_id().with("pq_corner"), Sense::click()).on_hover_text("Table actions");
        super::icons::paint(&painter, corner.shrink(2.0), "query", if cr.hovered() { t.accent } else { t.muted }, t.dark);
        egui::Popup::menu(&cr).show(|ui| corner_menu(app, ui));
        painter.hline(origin.x..=origin.x + total_w.max(viewport.width()), hy + head, Stroke::new(1.0, t.border));
        if qh + dh > 0.0 {
            painter.hline(origin.x..=origin.x + total_w, hy + hh, Stroke::new(1.0, t.border.gamma_multiply(0.5)));
        }
    });
    apply_clicks(app, clicks, table);
}

fn corner_menu(app: &mut VizApp, ui: &mut Ui) {
    if ui.button("Use First Row as Headers").clicked() {
        add_step(app, "Promoted Headers", "Table.PromoteHeaders({prev}, [PromoteAllScalars=true])");
        ui.close();
    }
    if ui.button("Add Custom Column…").clicked() {
        app.pq.dialog = Some(Dialog::custom_column(app));
        ui.close();
    }
    if ui.button("Add Conditional Column…").clicked() {
        app.pq.dialog = Some(Dialog::conditional(app));
        ui.close();
    }
    if ui.button("Add Index Column").clicked() {
        add_step(app, "Added Index", "Table.AddIndexColumn({prev}, \"Index\", 1, 1, Int64.Type)");
        ui.close();
    }
    if ui.button("Choose Columns…").clicked() {
        app.pq.dialog = Some(Dialog::choose_columns(app));
        ui.close();
    }
    ui.separator();
    if ui.button("Keep Top Rows…").clicked() {
        app.pq.dialog = Some(Dialog::rows(dialogs::KeepRemove::KeepTop));
        ui.close();
    }
    if ui.button("Remove Duplicates").clicked() {
        add_step(app, "Removed Duplicates", "Table.Distinct({prev})");
        ui.close();
    }
    if ui.button("Remove Blank Rows").clicked() {
        add_step(app, "Removed Blank Rows", "Table.SelectRows({prev}, each not List.IsEmpty(List.RemoveItems(Record.FieldValues(_), {\"\", null})))");
        ui.close();
    }
    ui.separator();
    if ui.button("Merge Queries…").clicked() {
        app.pq.dialog = Some(Dialog::merge(app, false));
        ui.close();
    }
    if ui.button("Append Queries…").clicked() {
        app.pq.dialog = Some(Dialog::append(false));
        ui.close();
    }
}

fn apply_clicks(app: &mut VizApp, c: Clicks, table: &Table) {
    if let Some((col, add)) = c.header {
        let name = table.columns.get(col).cloned().unwrap_or_default();
        if add {
            if let Some(i) = app.pq.selected.iter().position(|x| *x == name) {
                app.pq.selected.remove(i);
            } else {
                app.pq.selected.push(name);
            }
        } else {
            app.pq.selected = vec![name];
        }
    }
    if let Some(n) = c.rename {
        app.pq.dialog = Some(Dialog::rename_column(&n));
    }
    if let Some((n, at)) = c.filter {
        app.pq.selected = vec![n.clone()];
        app.pq.filter = Some(new_filter(table, &n, at));
    }
    if let Some(n) = c.expand {
        app.pq.dialog = Some(Dialog::expand(app, &n, false));
    }
    if let Some((r, col)) = c.nav {
        add_step(app, &col, &format!("{{prev}}{{{r}}}{}", field(&col)));
    }
}

pub fn new_filter(t: &Table, col: &str, at: egui::Pos2) -> FilterMenu {
    let mut vals: Vec<Value> = Vec::new();
    let mut truncated = false;
    if let Some(c) = t.column_index(col) {
        let mut seen = std::collections::HashSet::new();
        for r in &t.rows {
            let v = r.get(c).cloned().unwrap_or_default();
            if !v.is_error() && seen.insert(vizardcraft_query::funcs::table::key_of(&[&v])) {
                vals.push(v);
            }
            if vals.len() >= 1000 {
                truncated = true;
                break;
            }
        }
    }
    vals.sort_by(vizardcraft_query::value::compare);
    let ty = t.column_index(col).and_then(|c| t.types.get(c).copied()).unwrap_or_default();
    let ty = if ty == MType::Any { vizardcraft_query::funcs::convert::detect(&vals) } else { ty };
    FilterMenu { col: col.to_string(), values: vals.into_iter().map(|v| (v, true)).collect(), search: String::new(), at: pos2(at.x.max(4.0), at.y), ty, truncated }
}

/// The header dropdown: sort, filters by condition, a checklist of values.
fn filter_menu(app: &mut VizApp, ctx: &egui::Context, t: &Tokens, table: &Table) {
    let Some(mut f) = app.pq.filter.take() else { return };
    let mut close = false;
    let mut apply = false;
    let col = f.col.clone();
    let area = egui::Area::new(egui::Id::unique("pq_filter_menu")).order(egui::Order::Foreground).fixed_pos(f.at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(250.0);
            let item = |ui: &mut Ui, icon: &str, label: &str| -> bool {
                let r = ui.add(egui::Button::new(label).frame(false).min_size(vec2(240.0, 22.0)));
                super::icons::paint(ui.painter(), Rect::from_min_size(r.rect.min + vec2(-2.0, 3.0), vec2(16.0, 16.0)), icon, t.muted, t.dark);
                r.clicked()
            };
            if item(ui, "sort-asc", "      Sort Ascending") {
                ops::sort(app, false);
                close = true;
            }
            if item(ui, "sort-desc", "      Sort Descending") {
                ops::sort(app, true);
                close = true;
            }
            ui.separator();
            if item(ui, "remove-rows", "      Remove Empty") {
                let fl = field(&col);
                add_step(app, "Filtered Rows", &format!("Table.SelectRows({{prev}}, each {fl} <> null and {fl} <> \"\")"));
                close = true;
            }
            let (label, ops_list): (&str, &[usize]) = if f.ty.is_numeric() {
                ("Number Filters", &[0, 1, 2, 3, 4, 5])
            } else if matches!(f.ty, MType::Date | MType::DateTime) {
                ("Date Filters", &[0, 1, 2, 3, 4, 5])
            } else {
                ("Text Filters", &[0, 1, 6, 7, 8, 9, 10, 11])
            };
            ui.menu_button(format!("      {label}"), |ui| {
                for i in ops_list {
                    let name = match (f.ty.is_numeric() || matches!(f.ty, MType::Date | MType::DateTime), *i) {
                        (true, 2) if matches!(f.ty, MType::Date | MType::DateTime) => "After…",
                        (true, 4) if matches!(f.ty, MType::Date | MType::DateTime) => "Before…",
                        _ => COND_OPS.get(*i).copied().unwrap_or_default(),
                    };
                    if ui.button(capitalize(name)).clicked() {
                        app.pq.dialog = Some(Dialog::filter_rows(table, &col, i + 1));
                        close = true;
                        ui.close();
                    }
                }
                if matches!(f.ty, MType::Date | MType::DateTime) {
                    ui.separator();
                    for (lbl, e) in [
                        ("Is Earliest", "List.Min"),
                        ("Is Latest", "List.Max"),
                    ] {
                        if ui.button(lbl).clicked() {
                            let fl = field(&col);
                            add_step(app, "Filtered Rows", &format!("Table.SelectRows({{prev}}, let latest = {e}({{prev}}{fl}) in each {fl} = latest)"));
                            close = true;
                            ui.close();
                        }
                    }
                    for (lbl, e) in [("In the Year…", "Date.Year"), ("In the Month…", "Date.Month")] {
                        if ui.button(lbl).clicked() {
                            app.pq.dialog = Some(Dialog::filter_rows(table, &col, 1));
                            let _ = e;
                            close = true;
                            ui.close();
                        }
                    }
                }
            });
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut f.search).hint_text("Search").desired_width(f32::INFINITY));
            let all = f.values.iter().all(|(_, k)| *k);
            let mut a = all;
            if ui.checkbox(&mut a, "(Select All)").changed() {
                for (_, k) in f.values.iter_mut() {
                    *k = a;
                }
            }
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                let q = f.search.to_lowercase();
                for (v, k) in f.values.iter_mut().filter(|(v, _)| q.is_empty() || v.display().to_lowercase().contains(&q)) {
                    let label = if v.is_null() {
                        "(null)".to_string()
                    } else if v.is_empty_cell() {
                        "(blank)".to_string()
                    } else {
                        v.display()
                    };
                    ui.checkbox(k, label);
                }
            });
            if f.truncated {
                ui.label(RichText::new("List may be incomplete (first 1,000 values).").size(10.5).color(t.muted));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if super::forms::primary(ui, t, "OK").clicked() {
                        apply = true;
                    }
                });
            });
        });
    });
    // A click outside (not on a submenu) closes it.
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && !egui::Popup::is_any_open(ctx);
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if apply {
        if let Some(cond) = dialogs::filter_condition(&col, &f.values, 0, "", f.truncated) {
            add_step(app, "Filtered Rows", &format!("Table.SelectRows({{prev}}, each {cond})"));
        }
        close = true;
    }
    if !close && !clicked_outside {
        app.pq.filter = Some(f);
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str() + if s.ends_with('…') { "" } else { "…" },
        None => String::new(),
    }
}

// ---------------------------------------------------------------- Column From Examples

fn input_columns(ex: &Examples, t: &Table) -> Vec<usize> {
    ex.inputs.iter().filter_map(|c| t.column_index(c)).collect()
}

fn refresh_suggestions(ex: &mut Examples, t: &Table) {
    let mut cur: Vec<(usize, String)> = ex.values.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(r, v)| (*r, v.trim().to_string())).collect();
    cur.sort();
    if cur != ex.computed_for {
        ex.suggestions = if cur.is_empty() { vec![] } else { suggest(t, &input_columns(ex, t), &cur) };
        ex.chosen = 0;
        ex.computed_for = cur;
    }
}

fn predictions(ex: &Examples, t: &Table) -> Vec<String> {
    let Some(s) = ex.suggestions.get(ex.chosen) else { return vec![] };
    let host = NoHost;
    let ev = Evaluator::new(&host);
    let Ok(e) = vizardcraft_query::m::parse(&format!("each {}", s.expr)) else { return vec![] };
    let Ok(f) = ev.eval(&e, &Default::default()) else { return vec![] };
    (0..t.rows.len().min(1000)).map(|r| ev.apply1(&f, Value::record(t.row_record(r))).map(|v| v.display()).unwrap_or_else(|_| "Error".into())).collect()
}

fn example_cell(app: &mut VizApp, ui: &mut Ui, t: &Tokens, rect: Rect, r: usize, predicted: String) {
    let Some(ex) = app.pq.examples.as_mut() else { return };
    ui.painter().rect_filled(rect, 0.0, t.accent_soft.gamma_multiply(0.45));
    let buf = ex.values.entry(r).or_default();
    let typed = !buf.is_empty();
    let edit = egui::TextEdit::singleline(buf).hint_text(RichText::new(predicted).color(t.muted)).frame(egui::Frame::NONE).text_color(if typed { t.text } else { t.muted });
    let resp = ui.put(rect.shrink2(vec2(5.0, 1.0)), edit);
    if resp.changed() {
        ex.values.retain(|_, v| !v.is_empty());
    }
}

fn examples_bar(app: &mut VizApp, ui: &mut Ui, t: &Tokens, table: &Table) {
    if let Some(ex) = app.pq.examples.as_mut() {
        refresh_suggestions(ex, table);
    }
    let mut ok = false;
    let mut cancel = false;
    egui::Frame::new().fill(t.accent_soft).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        let Some(ex) = app.pq.examples.as_mut() else { return };
        ui.horizontal(|ui| {
            super::icons::paint(ui.painter(), Rect::from_min_size(ui.cursor().min, vec2(18.0, 18.0)), "from-examples", t.accent, t.dark);
            ui.add_space(22.0);
            ui.label(RichText::new("Add Column From Examples").strong());
            ui.label(RichText::new(if ex.inputs.is_empty() { "from all columns".to_string() } else { format!("from {}", ex.inputs.join(", ")) }).color(t.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.button("Cancel").clicked();
                ok = ui.add_enabled(!ex.suggestions.is_empty(), egui::Button::new(RichText::new("OK").strong())).clicked();
            });
        });
        ui.label(RichText::new("Enter sample values in the new column on the right; a formula that produces them appears here.").size(11.5));
        match ex.suggestions.get(ex.chosen) {
            Some(s) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Transform:").strong());
                    ui.label(RichText::new(&s.expr).monospace());
                });
                if ex.suggestions.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Other formulas:").small().color(t.muted));
                        for (i, alt) in ex.suggestions.iter().enumerate().take(6) {
                            if i != ex.chosen && ui.small_button(alt.expr.chars().take(40).collect::<String>()).on_hover_text(&alt.expr).clicked() {
                                ex.chosen = i;
                            }
                        }
                    });
                }
            }
            None if ex.values.values().any(|v| !v.trim().is_empty()) => {
                ui.colored_label(t.danger, "No formula reproduces these examples yet. Add another example, or check the values.");
            }
            None => {}
        }
    });
    if cancel {
        app.pq.examples = None;
    }
    if ok
        && let Some(ex) = app.pq.examples.take()
        && let Some(s) = ex.suggestions.get(ex.chosen)
    {
        let name = if ex.name.trim().is_empty() { "Custom".to_string() } else { ex.name.trim().to_string() };
        let ty = if s.ty.is_numeric() { "type number" } else { "type text" };
        add_step(app, &format!("Inserted {name}"), &format!("Table.AddColumn({{prev}}, {}, each {}, {ty})", quote_text(&name), s.expr));
    }
}

// ---------------------------------------------------------------- column profile

fn profile_panel(app: &mut VizApp, ui: &mut Ui, t: &Tokens, table: &Table, c: usize) {
    let p = app.pq.profile(table, c);
    let name = table.columns.get(c).cloned().unwrap_or_default();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(250.0);
            ui.label(RichText::new("Column statistics").strong().color(t.text));
            ui.label(RichText::new(&name).size(11.0).color(t.muted));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("pq_stats_scroll").show(ui, |ui| {
                egui::Grid::new("pq_stats").num_columns(2).spacing(vec2(18.0, 2.0)).striped(true).show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(RichText::new(k).color(t.muted).size(11.5));
                        ui.label(RichText::new(v).size(11.5));
                        ui.end_row();
                    };
                    row("Count", p.count.to_string());
                    row("Error", p.errors.to_string());
                    row("Empty", p.empty.to_string());
                    row("Distinct", p.distinct.to_string());
                    row("Unique", p.unique.to_string());
                    if let Some(m) = &p.min {
                        row("Min", m.display());
                    }
                    if let Some(m) = &p.max {
                        row("Max", m.display());
                    }
                    if let Some(a) = p.average {
                        row("Average", format!("{a:.4}"));
                    }
                    if let Some(s) = p.std_dev {
                        row("Standard deviation", format!("{s:.4}"));
                    }
                    if let Some(s) = p.sum {
                        row("Sum", vizardcraft_query::value::number_text(s));
                        row("Zero", p.zeros.to_string());
                    }
                    if let (Some(a), Some(b)) = (p.min_length, p.max_length) {
                        row("Min length", a.to_string());
                        row("Max length", b.to_string());
                    }
                });
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.label(RichText::new("Value distribution").strong().color(t.text));
            ui.add_space(4.0);
            let max = p.top.first().map(|x| x.1).unwrap_or(1).max(1) as f32;
            egui::ScrollArea::vertical().id_salt("pq_dist").show(ui, |ui| {
                for (v, n) in p.top.iter().take(30) {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(vec2(150.0, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_min_width(150.0);
                            ui.add(egui::Label::new(RichText::new(v).size(11.0)).truncate());
                        });
                        let (r, resp) = ui.allocate_exact_size(vec2((ui.available_width() - 50.0).clamp(80.0, 420.0), 14.0), Sense::click());
                        ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(r.width() * *n as f32 / max, r.height())), 2.0, if resp.hovered() { t.accent } else { t.accent.gamma_multiply(0.7) });
                        ui.label(RichText::new(n.to_string()).size(11.0).color(t.muted));
                        if resp.on_hover_text("Click to keep only this value").clicked() {
                            let lit = table.rows.iter().filter_map(|row| row.get(c)).find(|x| x.display() == *v).map(Value::m_literal).unwrap_or_else(|| quote_text(v));
                            add_step(app, "Filtered Rows", &format!("Table.SelectRows({{prev}}, each {} = {lit})", field(&name)));
                        }
                    });
                }
            });
        });
    });
}
