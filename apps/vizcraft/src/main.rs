//! VizCraft desktop app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod app;
mod flows;
mod studio;
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

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VizCraft")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([960.0, 600.0])
            .with_icon(icons::app_icon()),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("VizCraft", options, Box::new(move |cc| Ok(Box::new(app::VizApp::new(cc, &args)))))
}
