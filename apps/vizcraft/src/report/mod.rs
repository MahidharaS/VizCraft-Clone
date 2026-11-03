//! Report view: pages, the canvas, visuals, design elements and the report panes.

pub mod story;

use egui::Ui;
use vizcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;

pub struct ReportUi {
    pub zoom: f32,
    pub fit: bool,
    pub fit_width: bool,
    pub mobile: bool,
    pub edit_interactions: bool,
}

impl Default for ReportUi {
    fn default() -> Self {
        ReportUi { zoom: 1.0, fit: true, fit_width: false, mobile: false, edit_interactions: false }
    }
}

impl ReportUi {
    pub fn on_rebuild(&mut self) {}
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Report");
        let _ = app;
    });
}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let _ = (app, ctx);
}

pub fn ribbon_format(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let _ = (app, ui, t);
}
pub fn ribbon_arrange(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let _ = (app, ui, t);
}
pub fn clipboard(app: &mut VizApp, ctx: &egui::Context, op: &str) {
    let _ = (app, ctx, op);
}
pub fn insert_visual(app: &mut VizApp, kind: VisualKind) {
    let _ = (app, kind);
}
pub fn insert_text(app: &mut VizApp) {
    let _ = app;
}
pub fn insert_shape(app: &mut VizApp, kind: ShapeKind) {
    let _ = (app, kind);
}
pub fn insert_button(app: &mut VizApp, kind: &str) {
    let _ = (app, kind);
}
pub fn insert_image(app: &mut VizApp) {
    let _ = app;
}
pub fn new_page(app: &mut VizApp, kind: PageKind) {
    let _ = (app, kind);
}
pub fn duplicate_page(app: &mut VizApp) {
    let _ = app;
}

/// Standard base64 decoding.
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.split(',').next_back().unwrap_or(s);
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
