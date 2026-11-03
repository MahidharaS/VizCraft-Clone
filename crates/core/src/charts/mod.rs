//! Chart renderers: each visual kind draws its [`VisualData`] into a [`Scene`].
//!
//! `render` draws the plot content of a visual into a `width × height` box (the element's
//! content box, below the title). [`chrome`] draws the element container (background, border,
//! shadow, title) so the desktop and the web look the same.

pub mod chrome;
pub mod common;
pub mod qna;
pub mod svg;

use crate::color::Theme;
use crate::doc::*;
use crate::fx::Props;
use crate::scalar::Scalar;
use crate::scene::{Align, Scene};
use crate::vquery::VisualData;

pub struct RenderOpts<'a> {
    pub theme: &'a Theme,
    pub props: &'a Props,
    pub width: f32,
    pub height: f32,
    /// Points selected in this visual (it is the cross-filter source).
    pub selected: &'a [Vec<(FieldRef, Scalar)>],
    /// Analytics results computed for this visual (reference lines, forecasts, clusters).
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
    common::render_basic(spec, data, o)
}

/// Text helper: draws a centred message.
pub fn message(o: &RenderOpts, text: &str) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    s.text(o.width / 2.0, o.height / 2.0, text, 12.0, o.theme.foreground_muted, Align::Center, Align::Center);
    s
}
