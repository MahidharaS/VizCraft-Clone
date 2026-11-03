//! Data sheets: Excel-like editing of data (enter, paste, edit, formulas, fill, sort, find).

use crate::app::VizApp;

#[derive(Default)]
pub struct SheetUi {}

pub fn show(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Data sheets");
        let _ = app;
    });
}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let _ = (app, ctx);
}

/// A new empty sheet.
pub fn new_sheet(app: &mut VizApp) {
    app.toast("New sheet");
}

/// Copies a query's output into an editable sheet.
pub fn sheet_from_query(app: &mut VizApp, query: &str) {
    app.toast(format!("Sheet from {query}"));
}
