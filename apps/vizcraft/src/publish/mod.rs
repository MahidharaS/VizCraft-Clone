//! Publishing: VizCraft Service, web folder, single-file HTML (artifact-style), GitHub Pages
//! with a custom domain, mobile layout, SVG/PNG export.

use crate::app::VizApp;

#[derive(Default)]
pub struct PublishUi {}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let _ = (app, ctx);
}

/// Opens a publish flow: "service", "folder", "single", "github", "svg".
pub fn open(app: &mut VizApp, target: &str) {
    app.toast(format!("Publish: {target}"));
}
