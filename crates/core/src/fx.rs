//! Dynamic properties (fx): every formatting property can be a fixed value or bound to data.
//!
//! - Templates: `"Sales {[Total Sales]:$#,0} · {Selected:Region}"` put live numbers into any
//!   text (titles, text boxes, shape labels, buttons, tooltips).
//! - Expressions: a DAX expression whose result is the property (a number, text, or a colour).
//! - Rules and gradients: conditional formatting on a DAX value.
//!
//! Evaluation happens in the element's filter context, optionally narrowed to one data point.

use std::collections::BTreeMap;

use crate::build::Built;
use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use crate::state::{Context, col_ref, field_column};

/// Where a property is evaluated.
pub struct FxEnv<'a> {
    pub project: &'a Project,
    pub built: &'a Built,
    pub cx: &'a Context,
    /// Narrow to one data point (category values).
    pub point: Vec<(FieldRef, Scalar)>,
    pub page_name: &'a str,
}

impl<'a> FxEnv<'a> {
    pub fn new(project: &'a Project, built: &'a Built, cx: &'a Context, page_name: &'a str) -> FxEnv<'a> {
        FxEnv { project, built, cx, point: vec![], page_name }
    }

    fn filters(&self) -> Vec<String> {
        let mut f = self.cx.filters.clone();
        for (fr, v) in &self.point {
            if let Some((t, c)) = field_column(self.project, self.built, fr) {
                f.push(format!("FILTER(ALL({}), {} = {})", col_ref(&t, &c), col_ref(&t, &c), v.dax()));
            }
        }
        f
    }

    /// Evaluates a DAX scalar expression here.
    pub fn eval(&self, dax: &str) -> Result<Scalar, String> {
        let dax = crate::build::substitute_params(dax, self.project, &BTreeMap::new());
        let filters = self.filters();
        let refs: Vec<&str> = filters.iter().map(String::as_str).collect();
        self.built.model.evaluate_filtered(&dax, &refs).map(|v| Scalar::from_dm(&v)).map_err(|e| e.to_string())
    }

    fn measure_format(&self, dax: &str) -> String {
        let t = dax.trim();
        if let Some(name) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            return self.project.model.measures.iter().find(|m| m.name == name).map(|m| m.format.clone()).unwrap_or_default();
        }
        String::new()
    }

    /// Expands `{…}` placeholders in a template.
    pub fn template(&self, text: &str) -> String {
        if !text.contains('{') {
            return text.to_string();
        }
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            out.push_str(rest.get(..start).unwrap_or(""));
            let after = rest.get(start + 1..).unwrap_or("");
            // Find the matching close brace (allowing nested braces for DAX `{ }` lists).
            let mut depth = 1;
            let mut end = None;
            for (i, ch) in after.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(end) = end else {
                out.push_str(rest.get(start..).unwrap_or(""));
                return out;
            };
            let token = after.get(..end).unwrap_or("");
            out.push_str(&self.token(token));
            rest = after.get(end + 1..).unwrap_or("");
        }
        out.push_str(rest);
        out
    }

    fn token(&self, token: &str) -> String {
        let token = token.trim();
        if token.is_empty() {
            return String::new();
        }
        // {Page}, {Today:fmt}, {Selected:Field}, {Filters}, {@Param}
        if token.eq_ignore_ascii_case("page") {
            return self.page_name.to_string();
        }
        if token.eq_ignore_ascii_case("filters") {
            return if self.cx.description.is_empty() { "No filters".into() } else { self.cx.description.join("; ") };
        }
        if let Some(fmt) = token.strip_prefix("Today").map(|r| r.trim_start_matches(':')) {
            let f = if fmt.is_empty() { "yyyy-mm-dd" } else { fmt };
            return crate::numfmt::format_date(crate::state::today_serial(), f);
        }
        if let Some(field) = token.strip_prefix("Selected:") {
            return self.selected(field.trim());
        }
        if let Some(name) = token.strip_prefix('@') {
            let name = name.trim_start_matches('[').trim_end_matches(']');
            return self.project.parameter(name).map(|p| p.current.text()).unwrap_or_default();
        }
        // expression[:format] — the format is after the last ':' outside brackets/quotes.
        let (expr, fmt) = split_format(token);
        let fmt = if fmt.is_empty() { self.measure_format(expr) } else { fmt.to_string() };
        match self.eval(expr) {
            Ok(Scalar::Num(n)) => crate::vquery::format_value(n, &fmt, crate::build::ColType::Number),
            Ok(Scalar::Null) => "(Blank)".into(),
            Ok(v) => v.text(),
            Err(e) => format!("⚠ {e}"),
        }
    }

    /// The values selected for a field (from slicers/filters in the context description).
    fn selected(&self, field: &str) -> String {
        let p = self.project;
        for page in &p.report.pages {
            for e in &page.elements {
                if let Some(v) = e.visual()
                    && v.kind == VisualKind::Slicer
                    && v.well("field").first().is_some_and(|f| f.field.label().eq_ignore_ascii_case(field))
                {
                    if v.selection.is_empty() {
                        return "All".into();
                    }
                    return v.selection.iter().map(Scalar::text).collect::<Vec<_>>().join(", ");
                }
            }
        }
        "All".into()
    }

    /// Resolves a property to a concrete (non-fx) value.
    pub fn resolve(&self, prop: &Prop) -> Prop {
        match prop {
            Prop::Fx(fx) => self.resolve_fx(fx),
            other => other.clone(),
        }
    }

    pub fn resolve_fx(&self, fx: &Fx) -> Prop {
        match fx {
            Fx::Template { text } => Prop::Text(self.template(text)),
            Fx::Expression { dax } | Fx::FieldValue { dax } => match self.eval(dax) {
                Ok(Scalar::Num(n)) => Prop::Num(n),
                Ok(Scalar::Bool(b)) => Prop::Bool(b),
                Ok(Scalar::Text(t)) => match Color::parse(&t) {
                    Some(c) => Prop::Color(c),
                    None => Prop::Text(t),
                },
                Ok(Scalar::Null) => Prop::Text(String::new()),
                Err(e) => Prop::Text(format!("⚠ {e}")),
            },
            Fx::Rules { dax, rules, default } => {
                let v = self.eval(dax).unwrap_or(Scalar::Null);
                for r in rules {
                    if rule_holds(r, &v, None) {
                        return r.output.clone();
                    }
                }
                (**default).clone()
            }
            Fx::Gradient { dax, min, max, min_color, mid_color, max_color } => {
                let v = self.eval(dax).ok().and_then(|v| v.as_f64());
                match (v, min, max) {
                    (Some(x), Some(a), Some(b)) => Prop::Color(gradient(x, *a, *b, *min_color, *mid_color, *max_color)),
                    (Some(_), _, _) => Prop::Color(*max_color),
                    _ => Prop::Color(*min_color),
                }
            }
        }
    }
}

/// Splits `expr:format` (the format part has no brackets or quotes).
pub fn split_format(token: &str) -> (&str, &str) {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut split = None;
    for (i, ch) in token.char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ':' if depth == 0 && !in_str => split = Some(i),
            _ => {}
        }
    }
    match split {
        Some(i) => (token.get(..i).unwrap_or(token).trim(), token.get(i + 1..).unwrap_or("").trim()),
        None => (token, ""),
    }
}

pub fn rule_holds(r: &FxRule, v: &Scalar, range: Option<(f64, f64)>) -> bool {
    let x = v.as_f64();
    let (a, b) = if r.percent {
        match (range, x) {
            (Some((lo, hi)), _) => (lo + (hi - lo) * r.value / 100.0, lo + (hi - lo) * r.value2 / 100.0),
            _ => (r.value, r.value2),
        }
    } else {
        (r.value, r.value2)
    };
    match (r.op.as_str(), x) {
        (">", Some(x)) => x > a,
        (">=", Some(x)) => x >= a,
        ("<", Some(x)) => x < a,
        ("<=", Some(x)) => x <= a,
        ("=", Some(x)) => (x - a).abs() < 1e-9,
        ("<>", Some(x)) => (x - a).abs() >= 1e-9,
        ("between", Some(x)) => x >= a && x <= b,
        ("contains", _) => v.text().to_lowercase().contains(&r.text.to_lowercase()),
        ("equals text", _) => v.text().eq_ignore_ascii_case(&r.text),
        ("is blank", _) => v.is_null(),
        ("is not blank", _) => !v.is_null(),
        _ => false,
    }
}

pub fn gradient(x: f64, min: f64, max: f64, lo: Color, mid: Option<Color>, hi: Color) -> Color {
    let t = if max > min { ((x - min) / (max - min)).clamp(0.0, 1.0) } else { 1.0 };
    match mid {
        Some(m) => {
            if t < 0.5 {
                lo.lerp(m, t * 2.0)
            } else {
                m.lerp(hi, (t - 0.5) * 2.0)
            }
        }
        None => lo.lerp(hi, t),
    }
}

impl Prop {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Prop::Bool(b) => Some(*b),
            Prop::Num(n) => Some(*n != 0.0),
            Prop::Text(t) | Prop::Choice(t) => match t.to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" => Some(true),
                "false" | "off" | "no" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Prop::Num(n) => Some(*n),
            Prop::Text(t) | Prop::Choice(t) => t.trim().parse().ok(),
            Prop::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
    pub fn as_text(&self) -> String {
        match self {
            Prop::Text(t) | Prop::Choice(t) => t.clone(),
            Prop::Num(n) => crate::numfmt::general(*n),
            Prop::Bool(b) => b.to_string(),
            Prop::Color(c) => c.hex(),
            Prop::Fx(_) => String::new(),
        }
    }
    pub fn as_color(&self) -> Option<Color> {
        match self {
            Prop::Color(c) => Some(*c),
            Prop::Text(t) | Prop::Choice(t) => Color::parse(t),
            _ => None,
        }
    }
    pub fn is_fx(&self) -> bool {
        matches!(self, Prop::Fx(_))
    }
}

/// Resolved properties of one element: fixed values only. Charts read from this.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props {
    pub map: BTreeMap<String, Prop>,
    /// Per-data-point colour overrides from fx data colours (identity key → colour).
    pub point_colors: BTreeMap<String, Color>,
    /// Per-cell conditional formatting for tables/matrices: (row key, column label, aspect) → colour.
    pub cell_colors: BTreeMap<(String, String, String), Color>,
}

impl Props {
    pub fn get(&self, key: &str) -> Option<&Prop> {
        self.map.get(key)
    }
    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.map.get(key).and_then(Prop::as_bool).unwrap_or(default)
    }
    pub fn num(&self, key: &str, default: f64) -> f64 {
        self.map.get(key).and_then(Prop::as_num).unwrap_or(default)
    }
    pub fn f32(&self, key: &str, default: f32) -> f32 {
        self.num(key, default as f64) as f32
    }
    pub fn text(&self, key: &str, default: &str) -> String {
        self.map.get(key).map(Prop::as_text).filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string())
    }
    pub fn color(&self, key: &str, default: Color) -> Color {
        self.map.get(key).and_then(Prop::as_color).unwrap_or(default)
    }
    pub fn color_opt(&self, key: &str) -> Option<Color> {
        self.map.get(key).and_then(Prop::as_color)
    }
}

/// Key for a data point identity (used by `Props::point_colors`).
pub fn point_key(point: &[(FieldRef, Scalar)]) -> String {
    point.iter().map(|(_, v)| v.key()).collect::<Vec<_>>().join("\u{1}")
}

/// Resolves all of an element's properties: defaults from the format schema, then the
/// element's own values, with fx bindings evaluated.
pub fn resolve_props(env: &FxEnv, el: &Element) -> Props {
    let mut out = Props::default();
    let kind_key = crate::format::schema_key(el);
    for card in crate::format::cards(&kind_key) {
        for pd in card.props {
            out.map.insert(pd.key.to_string(), pd.default.clone());
        }
    }
    for (k, v) in &el.props {
        if k.starts_with("cf.") || k.starts_with("dataColors.point.") {
            continue;
        }
        out.map.insert(k.clone(), env.resolve(v));
    }
    out
}

/// Data colours bound with fx (`dataColors.fx`) evaluated per data point of a visual.
pub fn resolve_point_colors(env: &FxEnv, el: &Element, data: &crate::vquery::VisualData, props: &mut Props) {
    // Fixed per-point colours set by the user ("dataColors.point.<key>").
    for (k, v) in &el.props {
        if let Some(pk) = k.strip_prefix("dataColors.point.")
            && let Some(c) = v.as_color()
        {
            props.point_colors.insert(pk.to_string(), c);
        }
    }
    let Some(Prop::Fx(fx)) = el.props.get("dataColors.fx") else { return };
    let roles: &[&str] = &["axis", "legend", "details", "rows", "location"];
    // Precompute the value range for gradients with automatic min/max.
    let mut range: Option<(f64, f64)> = None;
    if let Fx::Gradient { min: None, .. } | Fx::Gradient { max: None, .. } | Fx::Rules { .. } = fx {
        let vals: Vec<f64> = data.values().first().map(|vi| data.rows.iter().filter_map(|r| r.get(*vi)?.as_f64()).collect()).unwrap_or_default();
        if !vals.is_empty() {
            let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            range = Some((lo, hi));
        }
    }
    for r in 0..data.rows.len().min(2000) {
        let point = data.identity(r, roles);
        let key = point_key(&point);
        if props.point_colors.contains_key(&key) {
            continue;
        }
        let penv = FxEnv { project: env.project, built: env.built, cx: env.cx, point: point.clone(), page_name: env.page_name };
        let color = match fx {
            Fx::Gradient { dax, min, max, min_color, mid_color, max_color } => {
                let v = penv.eval(dax).ok().and_then(|v| v.as_f64());
                let (lo, hi) = (min.or(range.map(|r| r.0)).unwrap_or(0.0), max.or(range.map(|r| r.1)).unwrap_or(1.0));
                v.map(|x| gradient(x, lo, hi, *min_color, *mid_color, *max_color))
            }
            Fx::Rules { dax, rules, default } => {
                let v = penv.eval(dax).unwrap_or(Scalar::Null);
                rules.iter().find(|ru| rule_holds(ru, &v, range)).map(|ru| ru.output.clone()).unwrap_or((**default).clone()).as_color()
            }
            other => penv.resolve_fx(other).as_color(),
        };
        if let Some(c) = color {
            props.point_colors.insert(key, c);
        }
    }
}

/// Conditional formatting of table/matrix cells: `cf.<column label>.<aspect>` props hold fx
/// (aspect = "background", "font", "dataBar", "icon").
pub fn resolve_cell_colors(env: &FxEnv, el: &Element, data: &crate::vquery::VisualData, props: &mut Props) {
    for (k, v) in &el.props {
        let Some(rest) = k.strip_prefix("cf.") else { continue };
        let Some((col_label, aspect)) = rest.rsplit_once('.') else { continue };
        let Prop::Fx(fx) = v else { continue };
        let Some(ci) = data.cols.iter().position(|c| c.label == col_label) else { continue };
        let vals: Vec<f64> = data.rows.iter().filter_map(|r| r.get(ci)?.as_f64()).collect();
        let range = if vals.is_empty() {
            None
        } else {
            Some((vals.iter().cloned().fold(f64::INFINITY, f64::min), vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max)))
        };
        for r in 0..data.rows.len().min(5000) {
            let rk = point_key(&data.identity(r, &[]));
            let cell = data.cell(r, ci).clone();
            let color = match fx {
                // Rules / gradients on the cell's own value when `dax` is empty.
                Fx::Gradient { dax, min, max, min_color, mid_color, max_color } => {
                    let x = if dax.trim().is_empty() {
                        cell.as_f64()
                    } else {
                        FxEnv { project: env.project, built: env.built, cx: env.cx, point: data.identity(r, &[]), page_name: env.page_name }.eval(dax).ok().and_then(|v| v.as_f64())
                    };
                    let (lo, hi) = (min.or(range.map(|r| r.0)).unwrap_or(0.0), max.or(range.map(|r| r.1)).unwrap_or(1.0));
                    x.map(|x| gradient(x, lo, hi, *min_color, *mid_color, *max_color))
                }
                Fx::Rules { dax, rules, default } => {
                    let v = if dax.trim().is_empty() {
                        cell
                    } else {
                        FxEnv { project: env.project, built: env.built, cx: env.cx, point: data.identity(r, &[]), page_name: env.page_name }.eval(dax).unwrap_or_default()
                    };
                    rules.iter().find(|ru| rule_holds(ru, &v, range)).map(|ru| ru.output.clone()).unwrap_or((**default).clone()).as_color()
                }
                Fx::FieldValue { dax } | Fx::Expression { dax } => {
                    FxEnv { project: env.project, built: env.built, cx: env.cx, point: data.identity(r, &[]), page_name: env.page_name }.eval(dax).ok().and_then(|v| Color::parse(&v.text()))
                }
                Fx::Template { .. } => None,
            };
            if let Some(c) = color {
                props.cell_colors.insert((rk, col_label.to_string(), aspect.to_string()), c);
            }
        }
    }
}
