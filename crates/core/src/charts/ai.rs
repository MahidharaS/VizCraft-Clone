//! AI visuals: key influencers, decomposition tree and smart narrative. They need their own
//! queries, so [`run`] computes their data (called from `vquery::run`), and the renderers
//! below draw it.
//!
//! Data layouts:
//! - Key influencers: `[Factor (text), Value (category), Effect (ratio), <Analyze>, Average]`,
//!   sorted by effect (strongest first).
//! - Decomposition tree: `[Level, Field, Category, <Analyze>, Selected]`; level 0 is the root.
//! - Smart narrative: `[Sentence]`.

use super::RenderOpts;
use super::common::*;
use crate::build::{Built, ColType};
use crate::doc::*;
use crate::scalar::Scalar;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, text_width};
use crate::state::{Context, col_ref, field_column, resolve_use};
use crate::vquery::{DataCol, VisualData, format_value, query};

fn col(label: &str, role: &str, field: FieldRef, is_value: bool, format: &str, ty: ColType) -> DataCol {
    DataCol { label: label.into(), role: role.into(), field, is_value, format: format.into(), col_type: ty, show_as: ShowAs::Value }
}

fn eq_filter(p: &Project, built: &Built, f: &FieldRef, v: &Scalar) -> Option<String> {
    let (t, c) = field_column(p, built, f)?;
    Some(format!("FILTER(ALL({}), {} = {})", col_ref(&t, &c), col_ref(&t, &c), v.dax()))
}

/// Computes the data of AI visuals; None for other kinds.
pub fn run(p: &Project, built: &Built, spec: &VisualSpec, cx: &Context) -> Option<VisualData> {
    match spec.kind {
        VisualKind::KeyInfluencers => Some(influencers(p, built, spec, cx)),
        VisualKind::DecompositionTree => Some(decomposition_data(p, built, spec, cx)),
        VisualKind::SmartNarrative => Some(narrative_data(p, built, spec, cx)),
        _ => None,
    }
}

fn analyze(p: &Project, spec: &VisualSpec) -> Option<FieldUse> {
    let f = spec.well("values").first().map(|f| resolve_use(p, f))?;
    if f.is_value() {
        Some(f)
    } else {
        let mut f2 = f;
        f2.agg = Agg::Average;
        Some(f2)
    }
}

fn influencers(p: &Project, built: &Built, spec: &VisualSpec, cx: &Context) -> VisualData {
    let Some(an) = analyze(p, spec) else { return VisualData::error("Add a field to Analyze.") };
    let explain: Vec<FieldUse> = spec.well("details").iter().chain(spec.well("expand").iter()).map(|f| resolve_use(p, f)).filter(|f| !f.is_value()).collect();
    if explain.is_empty() {
        return VisualData::error("Add fields to Explain by.");
    }
    let vals = vec![("values".to_string(), an.clone())];
    let mut out = VisualData::default();
    let mut fmt = String::new();
    let mut rows: Vec<(f64, Vec<Scalar>)> = Vec::new();
    for f in &explain {
        let Ok(r) = query(p, built, &[("details".into(), FieldUse::new(f.field.clone()))], &vals, &cx.filters) else { continue };
        if let Some(c) = r.cols.get(1) {
            fmt = c.format.clone();
        }
        let pts: Vec<(Scalar, f64)> = r.rows.iter().filter_map(|row| Some((row.first()?.clone(), row.get(1)?.as_f64()?))).filter(|(_, v)| v.is_finite()).collect();
        if pts.len() < 2 {
            continue;
        }
        let avg = pts.iter().map(|x| x.1).sum::<f64>() / pts.len() as f64;
        for (cat, v) in pts {
            let eff = if avg.abs() > 1e-12 { v / avg } else { 1.0 };
            rows.push((eff, vec![Scalar::Text(f.label()), cat, Scalar::Num(eff), Scalar::Num(v), Scalar::Num(avg)]));
        }
    }
    let up = true;
    rows.sort_by(|a, b| if up { b.0.partial_cmp(&a.0) } else { a.0.partial_cmp(&b.0) }.unwrap_or(std::cmp::Ordering::Equal));
    out.cols = vec![
        col("Factor", "__factor", FieldRef::col("", "Factor"), false, "", ColType::Text),
        col("Value", "details", FieldRef::col("", "Value"), false, "", ColType::Any),
        col("Effect", "values", FieldRef::measure("", "Effect"), true, "0.00\"x\"", ColType::Number),
        col(&an.label(), "values", an.field.clone(), true, &fmt, ColType::Number),
        col("Average", "values", FieldRef::measure("", "Average"), true, &fmt, ColType::Number),
    ];
    out.rows = rows.into_iter().map(|r| r.1).collect();
    out
}

fn decomposition_data(p: &Project, built: &Built, spec: &VisualSpec, cx: &Context) -> VisualData {
    let Some(an) = analyze(p, spec) else { return VisualData::error("Add a field to Analyze.") };
    let levels: Vec<FieldUse> = spec.well("details").iter().map(|f| resolve_use(p, f)).filter(|f| !f.is_value()).collect();
    let vals = vec![("values".to_string(), an.clone())];
    let mut out = VisualData::default();
    let total = query(p, built, &[], &vals, &cx.filters).ok();
    let fmt = total.as_ref().and_then(|t| t.cols.first().map(|c| c.format.clone())).unwrap_or_default();
    let tv = total.as_ref().and_then(|t| t.rows.first().and_then(|r| r.first()).cloned()).unwrap_or_default();
    out.cols = vec![
        col("Level", "__level", FieldRef::col("", "Level"), false, "", ColType::Integer),
        col("Field", "__field", FieldRef::col("", "Field"), false, "", ColType::Text),
        col("Category", "details", FieldRef::col("", "Category"), false, "", ColType::Any),
        col(&an.label(), "values", an.field.clone(), true, &fmt, ColType::Number),
        col("Selected", "__sel", FieldRef::col("", "Selected"), false, "", ColType::Bool),
    ];
    out.rows.push(vec![Scalar::Num(0.0), Scalar::Text(an.label()), Scalar::Text(an.label()), tv, Scalar::Bool(true)]);
    let mut filters = cx.filters.clone();
    let max_levels = 8;
    for (li, f) in levels.iter().enumerate().take(max_levels) {
        let Ok(r) = query(p, built, &[("details".into(), FieldUse::new(f.field.clone()))], &vals, &filters) else { break };
        let mut rows: Vec<(Scalar, Scalar)> = r.rows.iter().filter_map(|row| Some((row.first()?.clone(), row.get(1)?.clone()))).collect();
        rows.sort_by(|a, b| b.1.as_f64().unwrap_or(f64::MIN).partial_cmp(&a.1.as_f64().unwrap_or(f64::MIN)).unwrap_or(std::cmp::Ordering::Equal));
        rows.truncate(12);
        if rows.is_empty() {
            break;
        }
        let chosen = spec.drill.path.get(li).filter(|v| rows.iter().any(|r| r.0.key() == v.key())).cloned().or_else(|| rows.first().map(|r| r.0.clone())).unwrap_or_default();
        for (cat, v) in rows {
            let sel = cat.key() == chosen.key();
            out.rows.push(vec![Scalar::Num((li + 1) as f64), Scalar::Text(f.label()), cat, v, Scalar::Bool(sel)]);
        }
        match eq_filter(p, built, &f.field, &chosen) {
            Some(fl) => filters.push(fl),
            None => break,
        }
    }
    out
}

/// Text columns with few distinct values (good narrative dimensions).
fn narrative_dims(p: &Project, built: &Built) -> Vec<FieldRef> {
    let mut out = Vec::new();
    for (tname, t) in &built.tables {
        if p.parameter(tname).is_some() {
            continue;
        }
        for (ci, c) in t.columns.iter().enumerate() {
            if c.contains('.') || c.to_lowercase().ends_with("id") || !matches!(t.types.get(ci), Some(ColType::Text)) {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            for r in t.rows.iter().take(5000) {
                if let Some(v) = r.get(ci) {
                    seen.insert(v.key());
                }
                if seen.len() > 25 {
                    break;
                }
            }
            if (2..=25).contains(&seen.len()) {
                out.push(FieldRef::col(tname, c));
            }
        }
    }
    out
}

fn narrative_data(p: &Project, built: &Built, spec: &VisualSpec, cx: &Context) -> VisualData {
    let mut measures: Vec<FieldUse> = spec.all_fields().map(|f| resolve_use(p, f)).filter(|f| f.is_value()).collect();
    if measures.is_empty() {
        measures = p.model.measures.iter().take(2).map(|m| FieldUse::new(FieldRef::measure(&m.table, &m.name))).collect();
    }
    let mut dims: Vec<FieldRef> = spec.all_fields().map(|f| resolve_use(p, f)).filter(|f| !f.is_value()).map(|f| f.field).collect();
    if dims.is_empty() {
        dims = narrative_dims(p, built).into_iter().take(2).collect();
    }
    let date = built.tables.iter().find_map(|(tn, t)| t.columns.iter().zip(t.types.iter()).find(|(_, ty)| ty.is_temporal()).map(|(c, _)| (tn.clone(), c.clone())));
    let mut sentences: Vec<String> = Vec::new();
    for m in measures.iter().take(3) {
        let vals = vec![("values".to_string(), m.clone())];
        let Ok(tot) = query(p, built, &[], &vals, &cx.filters) else { continue };
        let fmt = tot.cols.first().map(|c| c.format.clone()).unwrap_or_default();
        let f = |v: f64| format_value(v, &fmt, ColType::Number);
        let Some(total) = tot.rows.first().and_then(|r| r.first()).and_then(Scalar::as_f64) else { continue };
        let name = m.label();
        sentences.push(format!("{name} is {}.", f(total)));
        let additive = !fmt.contains('%');
        for dfield in dims.iter().take(2) {
            let Ok(r) = query(p, built, &[("axis".into(), FieldUse::new(dfield.clone()))], &vals, &cx.filters) else { continue };
            let mut pts: Vec<(String, f64)> = r.rows.iter().filter_map(|row| Some((row.first()?.text(), row.get(1)?.as_f64()?))).collect();
            if pts.len() < 2 {
                continue;
            }
            pts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let (Some(top), Some(bot)) = (pts.first(), pts.last()) else { continue };
            let dl = dfield.label();
            if additive && total != 0.0 {
                sentences.push(format!("{} had the highest {name} at {} ({:.1}% of the total), while {} had the lowest at {}.", top.0, f(top.1), top.1 / total * 100.0, bot.0, f(bot.1)));
            } else {
                sentences.push(format!("Across {dl}, {name} ranged from {} for {} to {} for {}.", f(bot.1), bot.0, f(top.1), top.0));
            }
        }
        if let Some((tn, c)) = &date {
            let year = FieldRef::Level { table: tn.clone(), hierarchy: c.clone(), level: "Year".into() };
            if let Ok(r) = query(p, built, &[("axis".into(), FieldUse::new(year))], &vals, &cx.filters) {
                let ys: Vec<(String, f64)> = r.rows.iter().filter_map(|row| Some((row.first()?.text(), row.get(1)?.as_f64()?))).collect();
                if let (Some(a), Some(b)) = (ys.get(ys.len().saturating_sub(2)), ys.last())
                    && ys.len() >= 2
                    && a.1 != 0.0
                {
                    let g = (b.1 - a.1) / a.1.abs() * 100.0;
                    let verb = if g >= 0.0 { "grew" } else { "declined" };
                    sentences.push(format!("{name} {verb} {:.1}% from {} to {} ({} → {}).", g.abs(), a.0, b.0, f(a.1), f(b.1)));
                }
            }
            let month = FieldRef::Level { table: tn.clone(), hierarchy: c.clone(), level: "Month".into() };
            if let Ok(r) = query(p, built, &[("axis".into(), FieldUse::new(month))], &vals, &cx.filters) {
                let mut ms: Vec<(String, f64)> = r.rows.iter().filter_map(|row| Some((row.first()?.text(), row.get(1)?.as_f64()?))).collect();
                ms.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                if let (Some(hi), Some(lo)) = (ms.first(), ms.last())
                    && ms.len() >= 3
                {
                    sentences.push(format!("{name} peaks in {} ({}) and is lowest in {} ({}).", hi.0, f(hi.1), lo.0, f(lo.1)));
                }
            }
        }
    }
    if sentences.is_empty() {
        sentences.push("Add measures to summarize.".into());
    }
    VisualData { cols: vec![col("Sentence", "__text", FieldRef::col("", "Sentence"), false, "", ColType::Text)], rows: sentences.into_iter().map(|s| vec![Scalar::Text(s)]).collect(), ..VisualData::default() }
}

/// The FieldRef of an explain-by field from its label.
fn field_by_label(spec: &VisualSpec, label: &str) -> Option<FieldRef> {
    spec.all_fields().find(|f| f.label() == label || f.field.label() == label).map(|f| f.field.clone())
}

pub fn key_influencers(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    if d.rows.is_empty() {
        return empty(o, "Add fields to Analyze and Explain by");
    }
    let mut s = Scene::new(o.width, o.height);
    let an_label = d.cols.get(3).map(|c| c.label.clone()).unwrap_or_default();
    let fmt = ValueFmt::of_col(d, 3);
    let size = 10.0;
    s.text_bold(4.0, 4.0, format!("What influences {an_label} to increase?"), 11.0, o.theme.foreground, Align::Start, Align::Start);
    let split = o.width > 460.0;
    let lw = if split { o.width * 0.5 } else { o.width };
    let row_h = 40.0;
    let accent = o.props.color_opt("dataColors.default").unwrap_or(o.theme.accent);
    let top = 26.0;
    let n = (((o.height - top) / row_h).floor() as usize).max(1);
    for (i, r) in (0..d.rows.len()).take(n).enumerate() {
        let y = top + i as f32 * row_h;
        let eff = d.num(r, 2).unwrap_or(1.0);
        let factor = d.cell(r, 0).text();
        let value = cat_text(d, r, 1);
        let bub = Rect::new(4.0, y + 4.0, 44.0, row_h - 8.0);
        let c = if eff >= 1.0 { accent } else { o.theme.bad };
        s.push(Shape::Rect { rect: bub, fill: Some(c.with_alpha(28)), stroke: None, radius: bub.h / 2.0 });
        s.text_bold(bub.x + bub.w / 2.0, bub.y + bub.h / 2.0, format!("{eff:.2}x"), 9.5, c.shade(-0.2), Align::Center, Align::Center);
        s.text(56.0, y + 12.0, ell(&format!("When {factor} is {value}"), size, lw - 64.0), size, o.theme.foreground, Align::Start, Align::Center);
        let sub = format!("{an_label} is {} vs {} on average", fmt.short(d.num(r, 3).unwrap_or(0.0)), fmt.short(d.num(r, 4).unwrap_or(0.0)));
        s.text(56.0, y + 27.0, ell(&sub, size * 0.88, lw - 64.0), size * 0.88, o.theme.foreground_muted, Align::Start, Align::Center);
        s.line((4.0, y + row_h), (lw - 6.0, y + row_h), Line::solid(0.6, o.theme.visual_border));
        let point = field_by_label(spec, &factor).map(|f| vec![(f, d.cell(r, 1).clone())]).unwrap_or_default();
        let dimmed = dim(o, &point);
        s.push_hit(Shape::Rect { rect: Rect::new(0.0, y, lw, row_h), fill: None, stroke: None, radius: 0.0 }, Hit { point, tooltip: tooltip(d, r) }, dimmed);
    }
    // Right: the top factor's categories against the average.
    if split && let Some(factor) = d.rows.first().and_then(|r| r.first()).map(Scalar::text) {
        let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.cell(*r, 0).text() == factor).take(10).collect();
        let x0 = lw + 16.0;
        let w = o.width - x0 - 8.0;
        s.text_bold(x0, top + 2.0, ell(&format!("{an_label} by {factor}"), 10.0, w), 10.0, o.theme.foreground, Align::Start, Align::Start);
        let max = rows.iter().filter_map(|r| d.num(*r, 3)).fold(0.0f64, |a, b| a.max(b.abs())).max(1e-12);
        let avg = rows.first().and_then(|r| d.num(*r, 4)).unwrap_or(0.0);
        let label_w = rows.iter().map(|r| text_width(&cat_text(d, *r, 1), 9.0)).fold(0.0, f32::max).min(w * 0.35) + 8.0;
        let bh = ((o.height - top - 30.0) / rows.len().max(1) as f32).min(26.0);
        let bx = x0 + label_w;
        let bw = w - label_w - 40.0;
        for (i, r) in rows.iter().enumerate() {
            let y = top + 22.0 + i as f32 * bh;
            let v = d.num(*r, 3).unwrap_or(0.0);
            s.text(bx - 6.0, y + bh / 2.0, ell(&cat_text(d, *r, 1), 9.0, label_w - 6.0), 9.0, o.theme.foreground_muted, Align::End, Align::Center);
            let ww = bw * (v.abs() / max) as f32;
            let c = if v >= avg { accent } else { accent.with_alpha(120) };
            s.push(Shape::Rect { rect: Rect::new(bx, y + bh * 0.18, ww.max(1.0), bh * 0.64), fill: Some(c), stroke: None, radius: 2.0 });
            s.text(bx + ww + 4.0, y + bh / 2.0, fmt.short(v), 8.5, o.theme.foreground_muted, Align::Start, Align::Center);
        }
        let ax = bx + bw * (avg.abs() / max) as f32;
        let yb = top + 22.0 + rows.len() as f32 * bh;
        s.line((ax, top + 18.0), (ax, yb), Line::dashed(1.2, o.theme.foreground_muted));
        s.text(ax, yb + 2.0, format!("Average {}", fmt.short(avg)), 8.5, o.theme.foreground_muted, Align::Center, Align::Start);
    }
    s
}

pub fn decomposition(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    if d.rows.is_empty() {
        return empty(o, "Add fields to Analyze and Explain by");
    }
    let mut s = Scene::new(o.width, o.height);
    let nlev = d.rows.iter().filter_map(|r| r.first().and_then(Scalar::as_f64)).fold(0.0f64, f64::max) as usize + 1;
    let colw = (o.width / nlev as f32).clamp(90.0, 220.0);
    let fmt = ValueFmt::of_col(d, 3);
    let accent = o.props.color_opt("dataColors.default").unwrap_or(o.theme.accent);
    let track = o.theme.visual_border.with_alpha(160);
    let mut prev_sel: Option<(f32, f32)> = None;
    for lvl in 0..nlev {
        let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.num(*r, 0).map(|x| x as usize) == Some(lvl)).collect();
        if rows.is_empty() {
            break;
        }
        let x = lvl as f32 * colw + 6.0;
        let w = colw - 26.0;
        let head = rows.first().map(|r| d.cell(*r, 1).text()).unwrap_or_default();
        s.text_bold(x, 4.0, ell(&head, 10.0, w), 10.0, o.theme.foreground, Align::Start, Align::Start);
        let max = rows.iter().filter_map(|r| d.num(*r, 3)).fold(0.0f64, |a, b| a.max(b.abs())).max(1e-12);
        let item_h = ((o.height - 24.0) / rows.len().max(1) as f32).clamp(18.0, 40.0);
        let total_h = item_h * rows.len() as f32;
        let y0 = 22.0 + ((o.height - 24.0 - total_h) / 2.0).max(0.0);
        let mut sel_pos = None;
        for (i, r) in rows.iter().enumerate() {
            let y = y0 + i as f32 * item_h;
            if y + item_h > o.height + 1.0 {
                break;
            }
            let v = d.num(*r, 3).unwrap_or(0.0);
            let sel = matches!(d.cell(*r, 4), Scalar::Bool(true));
            let label = cat_text(d, *r, 2);
            let bar_y = y + item_h * 0.5;
            s.text(x, y + 2.0, ell(&label, 9.0, w * 0.62), 9.0, if sel { o.theme.foreground } else { o.theme.foreground_muted }, Align::Start, Align::Start);
            s.text(x + w, y + 2.0, fmt.short(v), 9.0, o.theme.foreground_muted, Align::End, Align::Start);
            let bh = (item_h * 0.3).clamp(4.0, 9.0);
            s.push(Shape::Rect { rect: Rect::new(x, bar_y + 1.0, w, bh), fill: Some(track), stroke: None, radius: bh / 2.0 });
            let bw = (w * (v.abs() / max) as f32).max(2.0);
            let c = if sel { accent } else { accent.with_alpha(130) };
            let point = if lvl == 0 { vec![] } else { field_by_label(spec, &d.cell(*r, 1).text()).map(|f| vec![(f, d.cell(*r, 2).clone())]).unwrap_or_default() };
            let dimmed = dim(o, &point);
            s.push_hit(Shape::Rect { rect: Rect::new(x, bar_y + 1.0, bw, bh), fill: Some(c), stroke: None, radius: bh / 2.0 }, Hit { point, tooltip: vec![(label.clone(), fmt.full(v))] }, dimmed);
            if let Some((px, py)) = prev_sel {
                s.push(Shape::Path { points: vec![(px, py), (px + 8.0, py), (x - 8.0, bar_y + 1.0 + bh / 2.0), (x - 2.0, bar_y + 1.0 + bh / 2.0)], closed: false, fill: None, stroke: Some(Line::solid(1.0, if sel { accent.with_alpha(180) } else { o.theme.foreground_muted.with_alpha(70) })) });
            }
            if sel {
                sel_pos = Some((x + w + 4.0, bar_y + 1.0 + bh / 2.0));
            }
            if lvl > 0 {
                s.actions.push((Rect::new(x, y, w, item_h), format!("decomp:{}:{}", lvl - 1, serde_json::to_string(d.cell(*r, 2)).unwrap_or_default())));
            }
        }
        prev_sel = sel_pos;
    }
    s
}

pub fn narrative(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let size = o.props.f32("values.size", 11.0).max(10.0);
    let line_h = size * 1.5;
    let accent = o.theme.accent;
    let mut y = 4.0;
    for r in 0..d.rows.len() {
        let text = d.cell(r, 0).text();
        let mut x = 16.0;
        s.push(Shape::Circle { cx: 7.0, cy: y + line_h / 2.0, r: 2.2, fill: Some(o.theme.foreground_muted), stroke: None });
        for word in text.split_whitespace() {
            let num = word.chars().any(|c| c.is_ascii_digit());
            let w = text_width(word, size) * if num { 1.15 } else { 1.0 };
            let sp = size * 0.32;
            if x + w > o.width - 4.0 && x > 16.0 {
                x = 16.0;
                y += line_h;
            }
            if y + line_h > o.height {
                return s;
            }
            s.push(Shape::Text { x, y: y + line_h / 2.0, text: word.into(), size, color: if num { accent } else { o.theme.foreground }, bold: num, italic: false, h: Align::Start, v: Align::Center, angle: 0.0 });
            x += w + sp;
        }
        y += line_h + size * 0.5;
    }
    s
}
