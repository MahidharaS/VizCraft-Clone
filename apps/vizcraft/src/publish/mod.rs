//! Publishing: VizCraft Service, web folder, single-file HTML (artifact-style), GitHub Pages
//! with a custom domain, mobile layout, SVG export.
//!
//! The dialog builds the bundle on the UI thread (the model isn't `Send`) and sends it from a
//! worker thread (HTTP / git), reporting progress over a channel.

use std::collections::BTreeSet;
use std::sync::mpsc::{Receiver, channel};

use egui::{Color32, RichText, Ui, vec2};
use vizcraft_core::doc::{PageKind, PublishTarget};
use vizcraft_publish::BundleOptions;
use vizcraft_publish::net::{self, GitHubTarget};

use crate::app::VizApp;
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Service,
    Folder,
    Single,
    GitHub,
}

impl Tab {
    const ALL: [(Tab, &'static str, &'static str, &'static str); 4] = [
        (Tab::Service, "service", "VizCraft Service", "Your self-hosted report portal"),
        (Tab::Folder, "open", "Web folder", "A static site for any web server"),
        (Tab::Single, "export", "Single HTML file", "One file to share or attach"),
        (Tab::GitHub, "publish", "GitHub Pages", "Free hosting, your own domain"),
    ];
}

enum Msg {
    Progress(String),
    Done(Result<String, String>),
}

struct Job {
    rx: Receiver<Msg>,
    progress: String,
    tab: Tab,
}

struct Status {
    ok: bool,
    text: String,
    link: Option<String>,
    note: Option<String>,
}

pub struct PublishUi {
    pub open: bool,
    tab: Tab,
    loaded: bool,
    // Service.
    svc_url: String,
    svc_workspace: String,
    svc_name: String,
    svc_token: String,
    // Folder / single file.
    folder: String,
    single_path: String,
    // GitHub Pages.
    gh: GitHubTarget,
    // Options.
    interactive: bool,
    include_mobile: bool,
    excluded: BTreeSet<u64>,
    store_token: bool,
    job: Option<Job>,
    status: Option<Status>,
    env_checked: bool,
}

impl Default for PublishUi {
    fn default() -> Self {
        PublishUi {
            open: false,
            tab: Tab::Service,
            loaded: false,
            svc_url: "http://localhost:8080".into(),
            svc_workspace: "My workspace".into(),
            svc_name: String::new(),
            svc_token: String::new(),
            folder: String::new(),
            single_path: String::new(),
            gh: GitHubTarget { branch: "gh-pages".into(), ..GitHubTarget::default() },
            interactive: true,
            include_mobile: true,
            excluded: BTreeSet::new(),
            store_token: false,
            job: None,
            status: None,
            env_checked: false,
        }
    }
}

impl PublishUi {
    /// Fills the form from the project's saved targets.
    fn load(&mut self, app_project: &vizcraft_core::doc::Project) {
        self.interactive = app_project.publish.interactive || app_project.publish.targets.is_empty();
        self.include_mobile = app_project.publish.include_mobile || app_project.publish.targets.is_empty();
        self.svc_name = app_project.name.clone();
        for t in &app_project.publish.targets {
            match t {
                PublishTarget::Service { url, workspace, token } => {
                    self.svc_url = url.clone();
                    self.svc_workspace = workspace.clone();
                    if !token.is_empty() {
                        self.svc_token = token.clone();
                        self.store_token = true;
                    }
                }
                PublishTarget::Folder { path } => self.folder = path.clone(),
                PublishTarget::SingleFile { path } => self.single_path = path.clone(),
                PublishTarget::GitHubPages { repo, branch, token, domain, path } => {
                    self.gh = GitHubTarget { repo: repo.clone(), branch: branch.clone(), token: token.clone(), domain: domain.clone(), path: path.clone() };
                    if !token.is_empty() {
                        self.store_token = true;
                    }
                }
            }
        }
        self.loaded = true;
    }
}

/// Opens a publish flow: "service", "folder", "single", "github", "svg".
pub fn open(app: &mut VizApp, target: &str) {
    if target == "svg" {
        export_svg(app);
        return;
    }
    let ui = &mut app.publish_ui;
    ui.tab = match target {
        "folder" => Tab::Folder,
        "single" => Tab::Single,
        "github" => Tab::GitHub,
        _ => Tab::Service,
    };
    if !ui.loaded {
        let p = app.project.clone();
        app.publish_ui.load(&p);
    }
    app.publish_ui.status = None;
    app.publish_ui.open = true;
}

/// Saves the current page as an SVG image.
fn export_svg(app: &mut VizApp) {
    let Some(page) = app.current_page().cloned() else {
        app.error("There's no page to export.");
        return;
    };
    let name = format!("{} - {}.svg", app.project.name, page.name).replace(['/', '\\', ':'], "-");
    let Some(path) = rfd::FileDialog::new().add_filter("SVG image", &["svg"]).set_file_name(name).save_file() else { return };
    let mut rt = app.rt.clone();
    rt.page = page.id;
    let svg = vizcraft_publish::page_svg(&app.project, &app.built, &rt, &page, false);
    match std::fs::write(&path, svg) {
        Ok(()) => app.toast(format!("Saved {}", path.display())),
        Err(e) => app.error(format!("Couldn't save the SVG: {e}")),
    }
}

fn bundle_options(app: &VizApp) -> BundleOptions {
    let u = &app.publish_ui;
    let pages = app.project.report.pages.iter().filter(|p| p.kind != PageKind::Tooltip && !u.excluded.contains(&p.id)).map(|p| p.id).collect();
    BundleOptions { interactive: u.interactive, include_mobile: u.include_mobile, pages, title: None, wasm: None }
}

/// Remembers a target in the project (the token only when the user allows it).
fn remember(app: &mut VizApp, target: PublishTarget) {
    let same = |a: &PublishTarget, b: &PublishTarget| std::mem::discriminant(a) == std::mem::discriminant(b);
    let (interactive, mobile) = (app.publish_ui.interactive, app.publish_ui.include_mobile);
    let mut targets = app.project.publish.targets.clone();
    targets.retain(|t| !same(t, &target));
    targets.push(target);
    if targets != app.project.publish.targets || app.project.publish.interactive != interactive || app.project.publish.include_mobile != mobile {
        app.checkpoint("Publish settings");
        app.project.publish.targets = targets;
        app.project.publish.interactive = interactive;
        app.project.publish.include_mobile = mobile;
    }
}

fn now_label() -> String {
    vizcraft_publish::rfc3339(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

/// Runs the selected target.
fn start(app: &mut VizApp) {
    let tab = app.publish_ui.tab;
    let opts = bundle_options(app);
    if opts.pages.is_empty() {
        app.publish_ui.status = Some(Status { ok: false, text: "Choose at least one page to publish.".into(), link: None, note: None });
        return;
    }
    let store = app.publish_ui.store_token;
    match tab {
        Tab::Single => {
            let path = if app.publish_ui.single_path.trim().is_empty() {
                match rfd::FileDialog::new().add_filter("Web page", &["html"]).set_file_name(format!("{}.html", app.project.name)).save_file() {
                    Some(p) => p.display().to_string(),
                    None => return,
                }
            } else {
                app.publish_ui.single_path.clone()
            };
            let result = vizcraft_publish::single_file(&app.project, &app.built, &opts).and_then(|html| std::fs::write(&path, html).map_err(|e| e.to_string()));
            app.publish_ui.status = Some(match result {
                Ok(()) => {
                    app.publish_ui.single_path = path.clone();
                    remember(app, PublishTarget::SingleFile { path: path.clone() });
                    app.project.publish.last_published = now_label();
                    Status { ok: true, text: format!("Saved {path}"), link: Some(path), note: Some("Anyone can open this file in a browser — no server needed.".into()) }
                }
                Err(e) => Status { ok: false, text: format!("Couldn't save the file: {e}"), link: None, note: None },
            });
        }
        Tab::Folder => {
            let dir = if app.publish_ui.folder.trim().is_empty() {
                match rfd::FileDialog::new().pick_folder() {
                    Some(p) => p.display().to_string(),
                    None => return,
                }
            } else {
                app.publish_ui.folder.clone()
            };
            let result = vizcraft_publish::bundle(&app.project, &app.built, &opts).and_then(|files| net::write_folder(&files, std::path::Path::new(&dir)));
            app.publish_ui.status = Some(match result {
                Ok(()) => {
                    app.publish_ui.folder = dir.clone();
                    remember(app, PublishTarget::Folder { path: dir.clone() });
                    app.project.publish.last_published = now_label();
                    Status {
                        ok: true,
                        text: format!("Published to {dir}"),
                        link: Some(dir),
                        note: Some("Upload the folder to any web server (or run `python3 -m http.server` in it to try it locally).".into()),
                    }
                }
                Err(e) => Status { ok: false, text: e, link: None, note: None },
            });
        }
        Tab::Service | Tab::GitHub => {
            let files = match vizcraft_publish::bundle(&app.project, &app.built, &opts) {
                Ok(f) => f,
                Err(e) => {
                    app.publish_ui.status = Some(Status { ok: false, text: e, link: None, note: None });
                    return;
                }
            };
            let (tx, rx) = channel();
            let u = &app.publish_ui;
            if tab == Tab::Service {
                let (url, ws, token) = (u.svc_url.trim().to_string(), u.svc_workspace.trim().to_string(), u.svc_token.trim().to_string());
                let name = if u.svc_name.trim().is_empty() { app.project.name.clone() } else { u.svc_name.trim().to_string() };
                remember(app, PublishTarget::Service { url: url.clone(), workspace: ws.clone(), token: if store { token.clone() } else { String::new() } });
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::Progress(format!("Uploading to {url}…")));
                    let _ = tx.send(Msg::Done(net::publish_to_service(&url, &ws, &name, &token, &files)));
                });
            } else {
                let gh = u.gh.clone();
                if let Err(e) = gh.owner_repo() {
                    app.publish_ui.status = Some(Status { ok: false, text: e, link: None, note: None });
                    return;
                }
                remember(
                    app,
                    PublishTarget::GitHubPages { repo: gh.repo.clone(), branch: gh.branch.clone(), token: if store { gh.token.clone() } else { String::new() }, domain: gh.domain.clone(), path: gh.path.clone() },
                );
                let msg = format!("Publish {} with VizCraft", app.project.name);
                std::thread::spawn(move || {
                    let tx2 = tx.clone();
                    let progress = move |s: &str| {
                        let _ = tx2.send(Msg::Progress(s.to_string()));
                    };
                    let _ = tx.send(Msg::Done(net::publish_github_pages(&gh, &files, &msg, &progress)));
                });
            }
            app.publish_ui.status = None;
            app.publish_ui.job = Some(Job { rx, progress: "Building the report…".into(), tab });
        }
    }
}

fn test_connection(app: &mut VizApp) {
    let (url, token) = (app.publish_ui.svc_url.trim().to_string(), app.publish_ui.svc_token.trim().to_string());
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(Msg::Done(net::service_ping(&url, &token).map(|m| format!("ping:{m}"))));
    });
    app.publish_ui.status = None;
    app.publish_ui.job = Some(Job { rx, progress: "Connecting…".into(), tab: Tab::Service });
}

fn preview(app: &mut VizApp) {
    let opts = bundle_options(app);
    let r = vizcraft_publish::single_file(&app.project, &app.built, &opts).and_then(|html| net::preview_file(&html, &app.project.name)).and_then(|p| net::open_in_browser(&p.display().to_string()));
    if let Err(e) = r {
        app.error(e);
    }
}

fn poll(app: &mut VizApp, ctx: &egui::Context) {
    let Some(job) = app.publish_ui.job.as_mut() else { return };
    let mut done = None;
    while let Ok(m) = job.rx.try_recv() {
        match m {
            Msg::Progress(p) => job.progress = p,
            Msg::Done(r) => done = Some(r),
        }
    }
    let tab = job.tab;
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let Some(r) = done else { return };
    app.publish_ui.job = None;
    app.publish_ui.status = Some(match r {
        Ok(m) if m.starts_with("ping:") => Status { ok: true, text: m.trim_start_matches("ping:").to_string(), link: None, note: None },
        Ok(url) => {
            app.project.publish.last_published = now_label();
            let note = if tab == Tab::GitHub {
                Some(app.publish_ui.gh.dns_help().map(|d| format!("{d} GitHub can take a minute to build the site.")).unwrap_or_else(|| "GitHub can take a minute to build the site the first time.".into()))
            } else {
                None
            };
            Status { ok: true, text: "Published.".into(), link: Some(url), note }
        }
        Err(e) => Status { ok: false, text: e, link: None, note: None },
    });
}

// ---------------------------------------------------------------------------------------------
// The dialog
// ---------------------------------------------------------------------------------------------

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    if !app.publish_ui.env_checked {
        app.publish_ui.env_checked = true;
        // Demo / screenshot hook: VIZCRAFT_OPEN=publish[:service|folder|single|github].
        if let Ok(v) = std::env::var("VIZCRAFT_OPEN")
            && let Some(rest) = v.strip_prefix("publish")
        {
            let (target, test) = match rest.trim_start_matches(':').split_once('+') {
                Some((t, a)) => (t.to_string(), a == "test"),
                None => (rest.trim_start_matches(':').to_string(), false),
            };
            open(app, &target);
            if test {
                test_connection(app);
            }
        }
    }
    poll(app, ctx);
    if !app.publish_ui.open {
        return;
    }
    let t = app.tokens();
    let mut open_flag = true;
    let mut action: Option<&'static str> = None;
    egui::Window::new(RichText::new("Publish report").strong().size(15.0))
        .id(egui::Id::unique("vizcraft-publish-dialog"))
        .open(&mut open_flag)
        .collapsible(false)
        .resizable(false)
        .fixed_size(vec2(780.0, 500.0))
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(0)))
        .show(ctx, |ui| {
            ui.set_min_size(vec2(780.0, 500.0));
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                sidebar(app, ui, &t);
                ui.vertical(|ui| {
                    ui.set_width(780.0 - 220.0);
                    egui::Frame::new().inner_margin(egui::Margin { left: 24, right: 24, top: 18, bottom: 8 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                        ui.set_min_height(500.0 - 18.0 - 8.0 - 58.0);
                        ui.set_width(780.0 - 220.0 - 48.0);
                        egui::ScrollArea::vertical().max_height(500.0 - 84.0).show(ui, |ui| {
                            form(app, ui, &t, &mut action);
                        });
                    });
                    footer(app, ui, &t, &mut action);
                });
            });
        });
    if !open_flag {
        app.publish_ui.open = false;
    }
    match action {
        Some("publish") => start(app),
        Some("preview") => preview(app),
        Some("test") => test_connection(app),
        Some("open-link") => {
            if let Some(l) = app.publish_ui.status.as_ref().and_then(|s| s.link.clone())
                && let Err(e) = net::open_in_browser(&l)
            {
                app.error(e);
            }
        }
        Some("copy-link") => {
            if let Some(l) = app.publish_ui.status.as_ref().and_then(|s| s.link.clone()) {
                ctx.copy_text(l);
                app.toast("Link copied");
            }
        }
        Some("dismiss") => app.publish_ui.status = None,
        Some("pick-folder") => {
            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                app.publish_ui.folder = p.display().to_string();
            }
        }
        Some("pick-file") => {
            if let Some(p) = rfd::FileDialog::new().add_filter("Web page", &["html"]).set_file_name(format!("{}.html", app.project.name)).save_file() {
                app.publish_ui.single_path = p.display().to_string();
            }
        }
        _ => {}
    }
}

fn sidebar(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(220.0, 500.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, egui::CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 }, t.panel_alt);
    p.line_segment([rect.right_top(), rect.right_bottom()], egui::Stroke::new(1.0, t.border));
    p.text(rect.min + vec2(18.0, 20.0), egui::Align2::LEFT_TOP, "PUBLISH TO", egui::FontId::proportional(10.5), t.muted);
    let mut y = rect.top() + 44.0;
    for (tab, icon, label, desc) in Tab::ALL {
        let r = egui::Rect::from_min_size(egui::pos2(rect.left() + 10.0, y), vec2(200.0, 54.0));
        let resp = ui.interact(r, ui.scope_id().with(("publish-tab", label)), egui::Sense::click());
        let on = app.publish_ui.tab == tab;
        if on {
            p.rect(r, 7.0, t.panel, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            p.rect_filled(egui::Rect::from_min_size(r.min + vec2(0.0, 12.0), vec2(3.0, 30.0)), 2.0, t.accent);
        } else if resp.hovered() {
            p.rect_filled(r, 7.0, t.accent_soft.gamma_multiply(0.6));
        }
        let ic = egui::Rect::from_min_size(r.min + vec2(14.0, 15.0), vec2(24.0, 24.0));
        crate::icons::paint(&p, ic, icon, if on { t.accent } else { t.muted });
        p.text(r.min + vec2(48.0, 11.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(13.0), if on { t.text } else { t.text.gamma_multiply(0.85) });
        p.text(r.min + vec2(48.0, 30.0), egui::Align2::LEFT_TOP, desc, egui::FontId::proportional(10.5), t.muted);
        if resp.clicked() && app.publish_ui.job.is_none() {
            app.publish_ui.tab = tab;
            app.publish_ui.status = None;
        }
        y += 58.0;
    }
    // Engine status at the bottom.
    let (dot, text) = if vizcraft_publish::has_wasm() { (t.good, "Interactive engine built in") } else { (t.muted, "Static reports (no WASM engine)") };
    p.circle_filled(egui::pos2(rect.left() + 22.0, rect.bottom() - 24.0), 4.0, dot);
    p.text(egui::pos2(rect.left() + 32.0, rect.bottom() - 24.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(10.5), t.muted);
}

fn heading(ui: &mut Ui, t: &Tokens, title: &str, sub: &str) {
    ui.label(RichText::new(title).size(18.0).strong().color(t.text));
    ui.label(RichText::new(sub).size(12.0).color(t.muted));
    ui.add_space(8.0);
}

fn field(ui: &mut Ui, t: &Tokens, label: &str, value: &mut String, hint: &str, password: bool) -> egui::Response {
    ui.label(RichText::new(label).size(12.0).strong().color(t.text));
    let r = ui.add(egui::TextEdit::singleline(value).hint_text(hint).password(password).desired_width(f32::INFINITY).margin(vec2(8.0, 5.0)).background_color(t.panel_alt));
    ui.add_space(4.0);
    r
}

fn form(app: &mut VizApp, ui: &mut Ui, t: &Tokens, action: &mut Option<&'static str>) {
    let busy = app.publish_ui.job.is_some();
    ui.add_enabled_ui(!busy, |ui| {
        match app.publish_ui.tab {
            Tab::Service => {
                heading(ui, t, "VizCraft Service", "Publish to your team's portal. Run one with `vizcraft-server serve --token …`.");
                let u = &mut app.publish_ui;
                field(ui, t, "Service URL", &mut u.svc_url, "https://reports.example.com", false);
                ui.columns(2, |c| {
                    field(&mut c[0], t, "Workspace", &mut u.svc_workspace, "My workspace", false);
                    field(&mut c[1], t, "Report name", &mut u.svc_name, "Report", false);
                });
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() - 130.0);
                        field(ui, t, "Token", &mut u.svc_token, "The service's publish token", true);
                    });
                    ui.vertical(|ui| {
                        ui.add_space(19.0);
                        if ui.add_sized(vec2(122.0, 28.0), egui::Button::new("Test connection")).clicked() {
                            *action = Some("test");
                        }
                    });
                });
            }
            Tab::Folder => {
                heading(ui, t, "Web folder", "A static website: index.html, the viewer, the data and the engine.");
                ui.label(RichText::new("Folder").size(12.0).strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut app.publish_ui.folder).hint_text("Choose a folder…").desired_width(ui.available_width() - 90.0).margin(vec2(8.0, 5.0)).background_color(t.panel_alt));
                    if ui.add_sized(vec2(82.0, 28.0), egui::Button::new("Browse…")).clicked() {
                        *action = Some("pick-folder");
                    }
                });
                ui.add_space(4.0);
            }
            Tab::Single => {
                heading(ui, t, "Single HTML file", "Everything in one file — share it like an artifact; it works offline.");
                ui.label(RichText::new("File").size(12.0).strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut app.publish_ui.single_path).hint_text("Choose where to save…").desired_width(ui.available_width() - 90.0).margin(vec2(8.0, 5.0)).background_color(t.panel_alt));
                    if ui.add_sized(vec2(82.0, 28.0), egui::Button::new("Save as…")).clicked() {
                        *action = Some("pick-file");
                    }
                });
                ui.add_space(4.0);
            }
            Tab::GitHub => {
                heading(ui, t, "GitHub Pages", "Commits the report to a branch and turns on Pages. Free hosting with HTTPS.");
                let g = &mut app.publish_ui.gh;
                ui.columns(2, |c| {
                    field(&mut c[0], t, "Repository", &mut g.repo, "owner/repo", false);
                    field(&mut c[1], t, "Branch", &mut g.branch, "gh-pages", false);
                });
                ui.columns(2, |c| {
                    field(&mut c[0], t, "Subfolder (optional)", &mut g.path, "reports/sales", false);
                    field(&mut c[1], t, "Custom domain (optional)", &mut g.domain, "reports.example.com", false);
                });
                field(ui, t, "Personal access token", &mut g.token, "Contents: write (and Pages: write)", true);
                if g.owner_repo().is_ok() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Site:").size(11.5).color(t.muted));
                        ui.label(RichText::new(g.site_url()).size(11.5).color(t.accent).monospace());
                    });
                }
            }
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        options(app, ui, t);
    });
}

fn options(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let wasm = vizcraft_publish::has_wasm();
    let u = &mut app.publish_ui;
    ui.label(RichText::new("Options").size(12.0).strong());
    ui.add_enabled_ui(wasm, |ui| {
        ui.checkbox(&mut u.interactive, "Interactive (slicers, cross-filtering, drill, parameters)").on_disabled_hover_text("This build has no WASM engine: run packaging/build-wasm.sh and rebuild VizCraft.");
    });
    ui.checkbox(&mut u.include_mobile, "Include the phone layout");
    if matches!(u.tab, Tab::Service | Tab::GitHub) {
        ui.checkbox(&mut u.store_token, "Remember the token in the project file").on_hover_text("Tokens are saved in plain text in the .vizc file. Leave this off when you share the project.");
        if u.store_token {
            ui.label(RichText::new("The token will be saved in plain text in the project file.").size(11.0).color(t.danger));
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Pages").size(12.0).strong());
    let pages: Vec<(u64, String, PageKind)> = app.project.report.pages.iter().filter(|p| p.kind != PageKind::Tooltip).map(|p| (p.id, p.name.clone(), p.kind)).collect();
    ui.horizontal_wrapped(|ui| {
        for (id, name, kind) in pages {
            let mut on = !u.excluded.contains(&id);
            let label = if kind == PageKind::Report { name } else { format!("{name} ({})", kind.label().to_lowercase()) };
            if ui.checkbox(&mut on, label).changed() {
                if on {
                    u.excluded.remove(&id);
                } else {
                    u.excluded.insert(id);
                }
            }
        }
    });
}

fn footer(app: &mut VizApp, ui: &mut Ui, t: &Tokens, action: &mut Option<&'static str>) {
    let (rect, _) = ui.allocate_exact_size(vec2(780.0 - 220.0, 58.0), egui::Sense::hover());
    ui.painter().line_segment([rect.left_top(), rect.right_top()], egui::Stroke::new(1.0, t.border));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(24.0, 12.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let ui = &mut child;
    let busy = app.publish_ui.job.is_some();
    if ui.add_enabled(!busy, egui::Button::new("Preview in browser").min_size(vec2(0.0, 30.0))).on_hover_text("Opens the report in your browser (nothing is uploaded)").clicked() {
        *action = Some("preview");
    }
    ui.add_space(10.0);
    // Status / progress in the middle.
    let label = match app.publish_ui.tab {
        Tab::Service => "Publish",
        Tab::Folder => "Publish to folder",
        Tab::Single => "Save file",
        Tab::GitHub => "Push to GitHub",
    };
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let btn = egui::Button::new(RichText::new(label).strong().color(Color32::WHITE)).fill(t.accent).min_size(vec2(130.0, 32.0)).corner_radius(6.0);
        if ui.add_enabled(!busy, btn).clicked() {
            *action = Some("publish");
        }
        ui.add_space(8.0);
        if let Some(job) = &app.publish_ui.job {
            ui.label(RichText::new(&job.progress).size(12.0).color(t.muted));
            ui.add(egui::Spinner::new().size(16.0));
        }
    });
    status_overlay(app, ui, t, action);
}

/// The result of the last run, shown above the footer as a banner.
fn status_overlay(app: &mut VizApp, ui: &mut Ui, t: &Tokens, action: &mut Option<&'static str>) {
    let Some(st) = &app.publish_ui.status else { return };
    let footer_rect = ui.max_rect();
    let h = if st.note.is_some() { 76.0 } else { 46.0 };
    let r = egui::Rect::from_min_max(egui::pos2(footer_rect.left(), footer_rect.top() - h - 22.0), egui::pos2(footer_rect.right(), footer_rect.top() - 14.0));
    let (bg, fg) = if st.ok { (t.good.gamma_multiply(0.12), t.good) } else { (t.danger.gamma_multiply(0.12), t.danger) };
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("publish-status"));
    let painter = ui.ctx().layer_painter(layer);
    painter.rect(r, 8.0, if t.dark { t.panel_alt } else { Color32::WHITE }, egui::Stroke::new(1.0, fg.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    painter.rect_filled(r, 8.0, bg);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(12.0, 8.0))).layout(egui::Layout::top_down(egui::Align::Min)).layer_id(layer));
    child.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
        let p = ui.painter();
        p.circle_filled(ir.center(), 8.0, fg);
        let c = ir.center();
        let sw = egui::Stroke::new(1.8, Color32::WHITE);
        if st.ok {
            p.line_segment([c + vec2(-3.5, 0.0), c + vec2(-1.0, 2.8)], sw);
            p.line_segment([c + vec2(-1.0, 2.8), c + vec2(3.8, -2.6)], sw);
        } else {
            p.line_segment([c + vec2(-3.0, -3.0), c + vec2(3.0, 3.0)], sw);
            p.line_segment([c + vec2(3.0, -3.0), c + vec2(-3.0, 3.0)], sw);
        }
        ui.label(RichText::new(&st.text).size(12.5).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                *action = Some("dismiss");
            }
            if let Some(link) = &st.link {
                if ui.small_button("Copy link").clicked() {
                    *action = Some("copy-link");
                }
                if ui.small_button("Open").clicked() {
                    *action = Some("open-link");
                }
                ui.add(egui::Label::new(RichText::new(link).size(11.5).color(t.accent)).truncate());
            }
        });
    });
    if let Some(n) = &st.note {
        child.add(egui::Label::new(RichText::new(n).size(11.0).color(t.muted)).wrap());
    }
}
