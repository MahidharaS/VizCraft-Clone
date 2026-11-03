//! Flows: Fabric/KNIME/Orange-style node pipelines, notebooks and machine learning.

use crate::app::VizApp;

#[derive(Default)]
pub struct FlowsUi {}

pub fn show(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Flows");
        let _ = app;
    });
}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let _ = (app, ctx);
}

pub fn new_flow(app: &mut VizApp) {
    app.toast("New flow");
}
