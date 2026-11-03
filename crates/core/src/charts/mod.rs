//! Chart renderers: each visual kind draws its [`VisualData`] into a [`Scene`].
//!
//! `render` draws the plot content of a visual into a `width × height` box (the element's
//! content box, below the title). [`chrome`] draws the element container (background, border,
//! shadow, title) so the desktop and the web look the same.
//!
//! # Scene actions
//!
//! Interactive regions are returned in `Scene::actions` as `(rect, action id)`:
//!
//! | Action id | Meaning |
//! |---|---|
//! | `slicer:<scalar json>` | toggle that value in a list/tile/dropdown slicer |
//! | `slicer:all` | "Select all" (clear the slicer selection) |
//! | `slicer:search` | the slicer's search box (focus a text field; its text comes back as `RenderOpts::search`) |
//! | `slicer:dropdown` | a dropdown slicer's header (toggle the `slicer.open` prop to show the list) |
//! | `range:<min>:<max>` | a between-slicer track: map the pointer's x inside the rect linearly to [min, max] and move the nearest handle (`VisualSpec::range`) |
//! | `expand:<row key>` / `collapse:<row key>` | matrix +/- buttons; the row key is the JSON array of the row path values (push/remove it in `VisualSpec::expanded`) |
//! | `decomp:<level>:<scalar json>` | decomposition tree: pick that node at `level` (set `drill.path` to the path up to `level` plus this value) |
//! | `scroll:<content height>` | table/matrix body: scrollable, with the total content height in points (scroll by setting the `scroll.offset` prop, in points) |

pub mod ai;
pub mod axis;
pub mod cards;
pub mod cartesian;
pub mod chrome;
pub mod common;
pub mod map;
pub mod misc;
pub mod pie;
pub mod qna;
pub mod scatter;
pub mod slicer;
pub mod svg;
pub mod table;
pub mod treemap;
pub mod waterfall;
pub mod world;

#[cfg(test)]
mod tests;

use crate::color::Theme;
use crate::doc::*;
use crate::fx::Props;
use crate::scalar::Scalar;
use crate::scene::{Align, Scene};
use crate::vquery::VisualData;

#[derive(Clone, Copy)]
pub struct RenderOpts<'a> {
    pub theme: &'a Theme,
    pub props: &'a Props,
    pub width: f32,
    pub height: f32,
    /// Points selected in this visual (it is the cross-filter source).
    pub selected: &'a [Vec<(FieldRef, Scalar)>],
    /// Analytics results computed for this visual (reference lines, forecasts, clusters).
    /// When empty and the spec has analytics lines, `render` computes them itself.
    pub analytics: &'a [crate::analytics::Computed],
    /// Image (base64 PNG) produced by a Python/R script visual.
    pub script_image: Option<&'a str>,
    /// Slicer search text typed in the UI.
    pub search: &'a str,
}

/// Draws a visual's content.
pub fn render(spec: &VisualSpec, data: &VisualData, o: &RenderOpts) -> Scene {
    if let Some(e) = &data.error {
        return Scene::message(o.width, o.height, e.clone());
    }
    if o.width < 4.0 || o.height < 4.0 {
        return Scene::new(o.width.max(0.0), o.height.max(0.0));
    }
    let computed = if o.analytics.is_empty() && !spec.analytics.is_empty() { crate::analytics::compute(&spec.analytics, data) } else { Vec::new() };
    let with_analytics = RenderOpts { analytics: if computed.is_empty() { o.analytics } else { &computed }, ..*o };
    let o = &with_analytics;
    use VisualKind::*;
    match spec.kind {
        StackedBar | StackedColumn | ClusteredBar | ClusteredColumn | StackedBar100 | StackedColumn100 | Line | Area | StackedArea | StackedArea100 | LineStackedColumn | LineClusteredColumn
        | Ribbon | DualAxis | Sparkline => cartesian::render(spec, data, o),
        Histogram => cartesian::histogram(spec, data, o),
        Waterfall => waterfall::waterfall(spec, data, o),
        Funnel => waterfall::funnel(spec, data, o),
        Scatter => scatter::render(spec, data, o),
        Pie | Donut => pie::render(spec, data, o),
        Treemap => treemap::render(spec, data, o),
        PackedBubbles => treemap::packed_bubbles(spec, data, o),
        WordCloud => treemap::word_cloud(spec, data, o),
        Map | FilledMap => map::render(spec, data, o),
        Gauge => cards::gauge(spec, data, o),
        Card => cards::card(spec, data, o),
        MultiRowCard => cards::multi_row(spec, data, o),
        Kpi => cards::kpi(spec, data, o),
        Slicer => slicer::render(spec, data, o),
        Table => table::table(spec, data, o),
        Matrix => table::matrix(spec, data, o),
        HeatMap | HighlightTable => table::heat(spec, data, o),
        KeyInfluencers => ai::key_influencers(spec, data, o),
        DecompositionTree => ai::decomposition(spec, data, o),
        SmartNarrative => ai::narrative(spec, data, o),
        QandA => misc::qna(spec, data, o),
        PythonVisual | RVisual => misc::script(spec, data, o),
        BoxPlot => misc::box_plot(spec, data, o),
        Bullet => misc::bullet(spec, data, o),
        Gantt => misc::gantt(spec, data, o),
        Radar => misc::radar(spec, data, o),
        Sankey => misc::sankey(spec, data, o),
    }
}

/// Text helper: draws a centred message.
pub fn message(o: &RenderOpts, text: &str) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    s.text(o.width / 2.0, o.height / 2.0, text, 12.0, o.theme.foreground_muted, Align::Center, Align::Center);
    s
}
