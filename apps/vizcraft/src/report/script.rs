//! Python and R script visuals: the script gets the visual's data as `dataset` and draws with
//! matplotlib / base R graphics; the PNG it saves is shown in the visual.

use std::sync::mpsc;

use egui::{RichText, Ui};
use vizcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;

pub const PY_TEMPLATE: &str = "# `dataset` is a pandas DataFrame with the fields you added (or a list of dicts without pandas).\nimport matplotlib\nmatplotlib.use('Agg')\nimport matplotlib.pyplot as plt\n\ncols = list(dataset.columns)\nfig, ax = plt.subplots(figsize=(6, 3.5))\nif len(cols) >= 2:\n    ax.bar(dataset[cols[0]].astype(str), dataset[cols[1]], color='#2B6CE0')\n    ax.set_xlabel(cols[0]); ax.set_ylabel(cols[1])\nplt.xticks(rotation=30, ha='right')\nplt.tight_layout()\n";
pub const R_TEMPLATE: &str = "# `dataset` is a data.frame with the fields you added.\nbarplot(dataset[[2]], names.arg = dataset[[1]], col = '#2B6CE0', las = 2)\n";

pub fn editor(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64, v: &VisualSpec) {
    ui.add_space(6.0);
    ui.label(RichText::new(if v.kind == VisualKind::RVisual { "R script editor" } else { "Python script editor" }).strong());
    let mut code = if v.script.is_empty() { if v.kind == VisualKind::RVisual { R_TEMPLATE.to_string() } else { PY_TEMPLATE.to_string() } } else { v.script.clone() };
    let changed = ui.add(egui::TextEdit::multiline(&mut code).code_editor().desired_rows(10).desired_width(f32::INFINITY)).changed();
    if changed && let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
        vv.script = code.clone();
        app.dirty = true;
    }
    ui.horizontal(|ui| {
        let running = app.report.script_jobs.iter().any(|(eid, _)| *eid == id);
        if ui.add_enabled(!running, egui::Button::new(if running { "Running…" } else { "▶ Run script" })).clicked() {
            run(app, id);
        }
        if let Some(e) = app.report.script_errors.get(&id) {
            ui.label(RichText::new(e).color(t.danger).size(10.5));
        }
    });
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

pub fn run(app: &mut VizApp, id: u64) {
    let page = app.page;
    let Some((_, el)) = app.project.element(id).map(|(p, e)| (p, e.clone())) else { return };
    let Some(v) = el.visual().cloned() else { return };
    let rd = super::render::element(app, page, &el);
    let Some(d) = rd.data.clone() else { return };
    let dir = std::env::temp_dir().join(format!("vizcraft-script-{id}"));
    let _ = std::fs::create_dir_all(&dir);
    let mut csv = d.cols.iter().map(|c| csv_escape(&c.label)).collect::<Vec<_>>().join(",") + "\n";
    for r in &d.rows {
        csv += &r.iter().map(|x| match x {
            vizcraft_core::scalar::Scalar::Num(n) => vizcraft_core::numfmt::general(*n),
            other => csv_escape(&other.text()),
        }).collect::<Vec<_>>().join(",");
        csv.push('\n');
    }
    let data_path = dir.join("dataset.csv");
    let out_path = dir.join("out.png");
    let _ = std::fs::remove_file(&out_path);
    if std::fs::write(&data_path, csv).is_err() {
        return;
    }
    let (w, h) = (rd.content_rect.w.max(100.0), rd.content_rect.h.max(80.0));
    let is_r = v.kind == VisualKind::RVisual;
    let user = if v.script.is_empty() { if is_r { R_TEMPLATE.to_string() } else { PY_TEMPLATE.to_string() } } else { v.script.clone() };
    let (prog, script_path, script) = if is_r {
        let s = format!(
            "dataset <- read.csv('{}', check.names = FALSE)\npng('{}', width = {}, height = {}, res = 110)\n{}\ninvisible(dev.off())\n",
            data_path.display(), out_path.display(), (w * 1.5) as i32, (h * 1.5) as i32, user
        );
        ("Rscript", dir.join("script.R"), s)
    } else {
        let s = format!(
            "import csv\ntry:\n    import pandas as pd\n    dataset = pd.read_csv(r'{}')\nexcept ImportError:\n    dataset = list(csv.DictReader(open(r'{}')))\n{}\nimport matplotlib.pyplot as _plt\n_f = _plt.gcf()\n_f.set_size_inches({:.2}, {:.2})\n_f.savefig(r'{}', dpi=110)\n",
            data_path.display(), data_path.display(), user, w / 72.0, h / 72.0, out_path.display()
        );
        ("python3", dir.join("script.py"), s)
    };
    if std::fs::write(&script_path, script).is_err() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = std::process::Command::new(prog).arg(&script_path).output();
        let msg = match res {
            Ok(o) if o.status.success() => match std::fs::read(&out_path) {
                Ok(bytes) => Ok(bytes),
                Err(e) => Err(format!("The script ran but saved no image: {e}")),
            },
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().rev().take(3).collect::<Vec<_>>().join(" | ")),
            Err(e) => Err(format!("Couldn't start {prog}: {e}. Install it and make sure it's on PATH.")),
        };
        let _ = tx.send(msg);
    });
    app.report.script_jobs.push((id, rx));
}

/// Collects finished script runs.
pub fn poll(app: &mut VizApp) {
    let mut done = Vec::new();
    for (i, (id, rx)) in app.report.script_jobs.iter().enumerate() {
        if let Ok(res) = rx.try_recv() {
            done.push((i, *id, res));
        }
    }
    for (i, id, res) in done.into_iter().rev() {
        app.report.script_jobs.remove(i);
        match res {
            Ok(bytes) => {
                app.report.script_images.insert(id, super::b64_encode(&bytes));
                app.report.script_errors.remove(&id);
            }
            Err(e) => {
                app.report.script_errors.insert(id, e);
            }
        }
        app.report.render_cache.clear();
    }
}
