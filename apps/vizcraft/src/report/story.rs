//! Stories (Tableau): sequences of story points over pages and dashboards.

use crate::app::VizApp;

pub fn show(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Stories");
        let _ = app;
    });
}

pub fn new_story(app: &mut VizApp) {
    let _ = app;
}
