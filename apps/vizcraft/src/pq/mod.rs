//! Power Query: Get Data dialogs, the Tableau-style data source canvas, and the Power Query
//! Editor (queries, applied steps, transforms, M advanced editor, profiling).

use crate::app::VizApp;

#[derive(Default)]
pub struct PqState {}

/// Shows the Power Query Editor (fills the central area).
pub fn show(app: &mut VizApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Power Query Editor");
        if ui.button("Close & Apply").clicked() {
            app.pq_open = false;
            app.mark_data_dirty();
        }
    });
}

/// Floating dialogs (Get Data, connection settings, …).
pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let _ = (app, ctx);
}

/// Opens the Get Data flow for a source kind: "csv", "text", "excel", "json", "xml", "folder",
/// "web", "sqlite", "postgresql", "mysql", "sqlserver", "odbc", "parquet", "odata", "blank", "enter", "more".
pub fn get_data(app: &mut VizApp, kind: &str) {
    app.toast(format!("Get data: {kind}"));
}

/// Opens the editor on a query.
pub fn open_editor(app: &mut VizApp, query: Option<&str>) {
    let _ = query;
    app.pq_open = true;
}

/// The Tableau-style data source page: drag tables onto a canvas to join / union them.
pub fn data_source_canvas(app: &mut VizApp) {
    app.pq_open = true;
}
