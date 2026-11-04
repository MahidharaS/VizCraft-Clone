//! VizardCraft desktop app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod app;
mod flows;
mod studio;
mod welcome;
mod icons;
mod model;
mod paint;
mod pq;
mod publish;
mod report;
mod ribbon;
mod sheet;
mod sqlview;
mod theme;
mod widgets;

/// VizCraft was renamed VizardCraft: carry over settings, keys, models and history once.
fn migrate_settings() {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from) else { return };
    let (old, new) = (home.join(".vizcraft"), home.join(".vizardcraft"));
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(&old, &new);
    }
}

fn main() -> eframe::Result {
    migrate_settings();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VizardCraft")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([960.0, 600.0])
            .with_icon(icons::app_icon()),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("VizardCraft", options, Box::new(move |cc| Ok(Box::new(app::VizApp::new(cc, &args)))))
}
