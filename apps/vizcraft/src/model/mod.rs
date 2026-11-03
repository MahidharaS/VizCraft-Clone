//! Table view, Model view (diagram), DAX query view, TMDL view, and the modeling dialogs
//! (relationships, measures, calculated columns/tables, parameters, roles).

use crate::app::VizApp;

#[derive(Default)]
pub struct ModelUi {}

pub fn table_view(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Table view");
        let _ = app;
    });
}

pub fn model_view(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Model view");
        let _ = app;
    });
}

pub fn dax_view(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("DAX query view");
        let _ = app;
    });
}

pub fn tmdl_view(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("TMDL view");
        let _ = app;
    });
}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let _ = (app, ctx);
}

/// New measure (in `table`, or the first table).
pub fn new_measure(app: &mut VizApp, table: Option<&str>) {
    let _ = table;
    app.toast("New measure");
}
pub fn quick_measure(app: &mut VizApp) {
    app.toast("Quick measure");
}
pub fn new_column(app: &mut VizApp, table: Option<&str>) {
    let _ = table;
    app.toast("New column");
}
pub fn new_table(app: &mut VizApp) {
    app.toast("New table");
}
/// A Tableau-style calculated field (Tableau syntax, translated to DAX).
pub fn calculated_field(app: &mut VizApp) {
    app.toast("Calculated field");
}
pub fn manage_relationships(app: &mut VizApp) {
    app.toast("Manage relationships");
}
/// New parameter: "whatif", "fields" or "value".
pub fn new_parameter(app: &mut VizApp, kind: &str) {
    app.toast(format!("New parameter: {kind}"));
}
pub fn manage_roles(app: &mut VizApp) {
    app.toast("Manage roles");
}
pub fn view_as(app: &mut VizApp) {
    app.toast("View as");
}
pub fn mark_date_table(app: &mut VizApp) {
    app.toast("Mark as date table");
}
