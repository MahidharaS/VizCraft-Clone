//! The title bar and ribbon: File, Home, Insert, Modeling, View, Data, Publish, Help, plus the
//! contextual Format / Arrange tabs when something is selected on the canvas.

use egui::{Color32, RichText, Ui};
use vizcraft_core::doc::*;

use crate::app::{RibbonTab, View, VizApp};
use crate::theme::Tokens;
use crate::widgets::{big_button, column, group, small_button};

#[derive(Default)]
pub struct RibbonUi {
    pub collapsed: bool,
    pub search: String,
}

pub fn show(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    // Title bar.
    egui::Panel::top("titlebar").exact_size(34.0).frame(egui::Frame::new().fill(t.rail).inner_margin(egui::Margin::symmetric(10, 4))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), r, "report", Color32::from_rgb(0x5E, 0xEA, 0xD4));
            ui.label(RichText::new("VizCraft").strong().color(Color32::WHITE).size(14.0));
            ui.add_space(8.0);
            for (icon, tip, act) in [("save", "Save (Ctrl+S)", 0), ("undo", "Undo (Ctrl+Z)", 1), ("redo", "Redo (Ctrl+Y)", 2)] {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, Color32::from_white_alpha(20));
                }
                crate::icons::paint(ui.painter(), r.shrink(4.0), icon, t.rail_text);
                if resp.on_hover_text(tip).clicked() {
                    match act {
                        0 => app.save(false),
                        1 => app.undo(),
                        _ => app.redo(),
                    }
                }
            }
            ui.add_space(12.0);
            let name = app.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| app.project.name.clone());
            ui.label(RichText::new(name).color(t.rail_text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if app.dark { "☀" } else { "🌙" };
                if ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE)).frame(false)).on_hover_text("Toggle dark mode").clicked() {
                    app.dark = !app.dark;
                    crate::theme::apply(ui.ctx(), app.dark);
                }
                ui.add(egui::TextEdit::singleline(&mut app.ribbon.search).hint_text("🔍 Search commands").desired_width(220.0));
                if !app.ribbon.search.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let q = app.ribbon.search.to_lowercase();
                    run_search(app, &q);
                    app.ribbon.search.clear();
                }
            });
        });
    });
    // The Power Query Editor has its own ribbon.
    if app.pq_open {
        return;
    }
    // Tabs + ribbon body.
    let has_sel = !app.selected.is_empty() && app.view == View::Report && !app.pq_open;
    egui::Panel::top("ribbon").frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin { left: 8, right: 8, top: 2, bottom: 4 }).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| {
            let mut tabs = vec![(RibbonTab::File, "File"), (RibbonTab::Home, "Home"), (RibbonTab::Insert, "Insert"), (RibbonTab::Modeling, "Modeling"), (RibbonTab::View, "View"), (RibbonTab::Data, "Data"), (RibbonTab::Publish, "Publish"), (RibbonTab::Help, "Help")];
            if has_sel {
                tabs.push((RibbonTab::Format, "Format"));
                tabs.push((RibbonTab::Arrange, "Arrange"));
            }
            for (tab, label) in tabs {
                let active = app.ribbon_tab == tab;
                let contextual = matches!(tab, RibbonTab::Format | RibbonTab::Arrange);
                let mut txt = RichText::new(label).size(12.5);
                if active {
                    txt = txt.strong().color(t.accent);
                } else if contextual {
                    txt = txt.color(Color32::from_rgb(0xB4, 0x53, 0x09));
                }
                let r = ui.add(egui::Button::new(txt).frame(false));
                if active {
                    let rr = r.rect;
                    ui.painter().line_segment([rr.left_bottom() + egui::vec2(4.0, 1.0), rr.right_bottom() + egui::vec2(-4.0, 1.0)], egui::Stroke::new(2.5, t.accent));
                }
                if r.clicked() {
                    app.ribbon_tab = if active && tab != RibbonTab::File { app.ribbon_tab } else { tab };
                    if active && tab != RibbonTab::File {
                        app.ribbon.collapsed = !app.ribbon.collapsed;
                    } else {
                        app.ribbon.collapsed = false;
                    }
                }
            }
        });
        if !has_sel && matches!(app.ribbon_tab, RibbonTab::Format | RibbonTab::Arrange) {
            app.ribbon_tab = RibbonTab::Home;
        }
        if app.ribbon.collapsed {
            return;
        }
        egui::ScrollArea::horizontal().id_salt("ribbon-scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(82.0);
                match app.ribbon_tab {
                    RibbonTab::File => file_tab(app, ui, t),
                    RibbonTab::Home => home_tab(app, ui, t),
                    RibbonTab::Insert => insert_tab(app, ui, t),
                    RibbonTab::Modeling => modeling_tab(app, ui, t),
                    RibbonTab::View => view_tab(app, ui, t),
                    RibbonTab::Data => data_tab(app, ui, t),
                    RibbonTab::Publish => publish_tab(app, ui, t),
                    RibbonTab::Help => help_tab(app, ui, t),
                    RibbonTab::Format => crate::report::ribbon_format(app, ui, t),
                    RibbonTab::Arrange => crate::report::ribbon_arrange(app, ui, t),
                }
            });
        });
    });
}

fn get_data_menu(app: &mut VizApp, ui: &mut Ui) {
    let items: &[(&str, &str)] = &[
        ("csv", "Text/CSV"),
        ("excel", "Excel workbook"),
        ("json", "JSON"),
        ("xml", "XML"),
        ("parquet", "Parquet"),
        ("folder", "Folder"),
        ("web", "Web / REST API"),
        ("odata", "OData feed"),
        ("sqlite", "SQLite database"),
        ("postgresql", "PostgreSQL database"),
        ("mysql", "MySQL / MariaDB database"),
        ("sqlserver", "SQL Server database"),
        ("odbc", "ODBC"),
        ("duckdb", "VizCraft SQL (DuckDB)"),
        ("enter", "Enter data"),
        ("blank", "Blank query"),
        ("more", "More…"),
    ];
    for (k, label) in items {
        if ui.button(*label).clicked() {
            if *k == "duckdb" {
                crate::sqlview::get_data_sql(app);
            } else {
                crate::pq::get_data(app, k);
            }
            ui.close();
        }
    }
}

fn file_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Project", |ui| {
        if big_button(ui, t, "new", "New", "New blank project (Ctrl+N)").clicked() {
            app.new_project();
        }
        if big_button(ui, t, "open", "Open", "Open a .vizc project (Ctrl+O)").clicked() {
            app.open_dialog();
        }
        if big_button(ui, t, "save", "Save", "Save (Ctrl+S)").clicked() {
            app.save(false);
        }
        if big_button(ui, t, "save", "Save as", "Save as a new file").clicked() {
            app.save(true);
        }
    });
    group(ui, t, "Samples", |ui| {
        if big_button(ui, t, "report", "Retail sample", "Open the built-in Retail Sales sample").clicked() {
            app.load_project(vizcraft_core::sample::sales(), None);
        }
    });
    group(ui, t, "Export", |ui| {
        if big_button(ui, t, "export", "Export SVG", "Export the current page as SVG").clicked() {
            crate::publish::open(app, "svg");
        }
        if big_button(ui, t, "publish", "Web page", "Export a single self-contained HTML report").clicked() {
            crate::publish::open(app, "single");
        }
    });
    group(ui, t, "Project", |ui| {
        ui.vertical(|ui| {
            ui.label("Name");
            let mut name = app.project.name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                app.project.name = name;
                app.dirty = true;
            }
            let mut auto = app.project.model.auto_date_time;
            if ui.checkbox(&mut auto, "Auto date/time hierarchies").changed() {
                app.checkpoint("Auto date/time");
                app.project.model.auto_date_time = auto;
                app.mark_model_dirty();
            }
        });
    });
}

fn home_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Clipboard", |ui| {
        column(ui, |ui| {
            if small_button(ui, t, "", "✂ Cut", "Cut (Ctrl+X)").clicked() {
                crate::report::clipboard(app, ui.ctx(), "cut");
            }
            if small_button(ui, t, "", "⎘ Copy", "Copy (Ctrl+C)").clicked() {
                crate::report::clipboard(app, ui.ctx(), "copy");
            }
            if small_button(ui, t, "", "📋 Paste", "Paste (Ctrl+V)").clicked() {
                crate::report::clipboard(app, ui.ctx(), "paste");
            }
        });
    });
    group(ui, t, "Data", |ui| {
        let r = big_button(ui, t, "get-data", "Get data ⌄", "Connect to files, databases and web sources");
        egui::Popup::menu(&r).show(|ui| get_data_menu(app, ui));
        column(ui, |ui| {
            if small_button(ui, t, "sheet", "Excel workbook", "Import an Excel workbook").clicked() {
                crate::pq::get_data(app, "excel");
            }
            if small_button(ui, t, "table", "Enter data", "Type or paste a table").clicked() {
                crate::pq::get_data(app, "enter");
            }
            if small_button(ui, t, "data", "SQL database", "Connect to a SQL database").clicked() {
                crate::pq::get_data(app, "sqlserver");
            }
        });
        column(ui, |ui| {
            if small_button(ui, t, "sheet", "New data sheet", "An Excel-like sheet you can type into").clicked() {
                crate::sheet::new_sheet(app);
            }
            if small_button(ui, t, "relationship", "Data source (joins)", "Tableau-style data source: drag tables to join or union").clicked() {
                crate::pq::data_source_canvas(app);
            }
            if small_button(ui, t, "data", "Recent sources", "Recently used sources").clicked() {
                crate::pq::get_data(app, "more");
            }
        });
    });
    group(ui, t, "Queries", |ui| {
        if big_button(ui, t, "transform", "Transform data", "Open the Power Query Editor").clicked() {
            crate::pq::open_editor(app, None);
        }
        if big_button(ui, t, "refresh", "Refresh", "Refresh all data (F5)").clicked() {
            app.mark_data_dirty();
        }
        if big_button(ui, t, "sql", "SQL", "SQL workbench (DuckDB) over the model, files and your warehouse").clicked() {
            crate::sqlview::open(app);
        }
    });
    group(ui, t, "Insert", |ui| {
        if big_button(ui, t, "clusteredColumn", "New visual", "Add a visual to the page").clicked() {
            crate::report::insert_visual(app, VisualKind::ClusteredColumn);
        }
        column(ui, |ui| {
            if small_button(ui, t, "text", "Text box", "Insert a text box").clicked() {
                crate::report::insert_text(app);
            }
            if small_button(ui, t, "shape", "Shapes", "Insert a rectangle").clicked() {
                crate::report::insert_shape(app, ShapeKind::RoundedRectangle);
            }
            if small_button(ui, t, "button", "Button", "Insert a button").clicked() {
                crate::report::insert_button(app, "Blank");
            }
        });
    });
    group(ui, t, "Calculations", |ui| {
        if big_button(ui, t, "measure", "New measure", "Create a DAX measure").clicked() {
            crate::model::new_measure(app, None);
        }
        column(ui, |ui| {
            if small_button(ui, t, "quick", "Quick measure", "Build a measure from a template").clicked() {
                crate::model::quick_measure(app);
            }
            if small_button(ui, t, "calc", "Calculated field", "Tableau-style calculated field").clicked() {
                crate::model::calculated_field(app);
            }
            if small_button(ui, t, "column", "New column", "Calculated column").clicked() {
                crate::model::new_column(app, None);
            }
        });
    });
    group(ui, t, "Share", |ui| {
        if big_button(ui, t, "publish", "Publish", "Publish to the VizCraft Service, the web or GitHub Pages").clicked() {
            crate::publish::open(app, "service");
        }
    });
    group(ui, t, "AI", |ui| {
        if big_button(ui, t, "ai", "Copilot", "Ask about your data; build visuals, measures and filters with AI").clicked() {
            app.ai.copilot_open = !app.ai.copilot_open;
            if app.ai.copilot_open && !matches!(app.view, View::Report | View::Table | View::Model | View::Dax) {
                app.view = View::Report;
            }
            app.pq_open = false;
        }
        column(ui, |ui| {
            if small_button(ui, t, "formulator", "Formulator", "Data Formulator-style data threads: concept encodings, derived fields, branching").clicked() {
                app.view = View::Formulator;
                app.pq_open = false;
            }
            if small_button(ui, t, "ai", "AI hub", "Models and keys, local models, OpenJEV, your personal brain, privacy").clicked() {
                app.view = View::Ai;
                app.pq_open = false;
            }
        });
        if big_button(ui, t, "studio", "Studio", "Data Science Studio: machine learning and statistics workflows, model registry and serving").clicked() {
            app.view = View::Studio;
            app.pq_open = false;
        }
    });
}

fn insert_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Pages", |ui| {
        let r = big_button(ui, t, "new-page", "New page ⌄", "Add a page, worksheet or dashboard");
        egui::Popup::menu(&r).show(|ui| {
            for (k, label) in [(PageKind::Report, "Report page"), (PageKind::Worksheet, "Worksheet (Tableau)"), (PageKind::Dashboard, "Dashboard (Tableau)"), (PageKind::Tooltip, "Tooltip page"), (PageKind::Drillthrough, "Drill-through page")] {
                if ui.button(label).clicked() {
                    crate::report::new_page(app, k);
                    ui.close();
                }
            }
            if ui.button("Duplicate page").clicked() {
                crate::report::duplicate_page(app);
                ui.close();
            }
        });
        column(ui, |ui| {
            if small_button(ui, t, "worksheet", "New worksheet", "A Tableau worksheet: one visual built from shelves").clicked() {
                crate::report::new_page(app, PageKind::Worksheet);
            }
            if small_button(ui, t, "dashboard", "New dashboard", "A Tableau dashboard").clicked() {
                crate::report::new_page(app, PageKind::Dashboard);
            }
            if small_button(ui, t, "story", "New story", "A Tableau story").clicked() {
                crate::report::story::new_story(app);
            }
        });
    });
    group(ui, t, "Visuals", |ui| {
        ui.vertical(|ui| {
            for row in [&[VisualKind::StackedColumn, VisualKind::ClusteredColumn, VisualKind::StackedBar, VisualKind::Line, VisualKind::Area, VisualKind::LineClusteredColumn, VisualKind::Pie, VisualKind::Donut, VisualKind::Treemap, VisualKind::Scatter][..], &[VisualKind::Card, VisualKind::Kpi, VisualKind::Gauge, VisualKind::Table, VisualKind::Matrix, VisualKind::Slicer, VisualKind::Map, VisualKind::Funnel, VisualKind::Waterfall, VisualKind::Ribbon][..]] {
                ui.horizontal(|ui| {
                    for k in row {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, 3.0, t.accent_soft);
                        }
                        crate::icons::visual(ui.painter(), r.shrink(4.0), *k, t.accent);
                        if resp.on_hover_text(k.label()).clicked() {
                            crate::report::insert_visual(app, *k);
                        }
                    }
                });
            }
        });
        let r = big_button(ui, t, "ai", "More ⌄", "All visuals");
        egui::Popup::menu(&r).show(|ui| {
            let mut groups: Vec<&str> = VisualKind::ALL.iter().map(|k| k.group()).collect();
            groups.dedup();
            let mut seen = std::collections::BTreeSet::new();
            for g in groups {
                if !seen.insert(g) {
                    continue;
                }
                ui.menu_button(g, |ui| {
                    for k in VisualKind::ALL.iter().filter(|k| k.group() == g) {
                        if ui.button(k.label()).clicked() {
                            crate::report::insert_visual(app, *k);
                            ui.close();
                        }
                    }
                });
            }
        });
    });
    group(ui, t, "AI visuals", |ui| {
        column(ui, |ui| {
            if small_button(ui, t, "keyInfluencers", "Key influencers", "").clicked() {
                crate::report::insert_visual(app, VisualKind::KeyInfluencers);
            }
            if small_button(ui, t, "decompositionTree", "Decomposition tree", "").clicked() {
                crate::report::insert_visual(app, VisualKind::DecompositionTree);
            }
            if small_button(ui, t, "qandA", "Q&A", "Ask a question in plain language").clicked() {
                crate::report::insert_visual(app, VisualKind::QandA);
            }
        });
        column(ui, |ui| {
            if small_button(ui, t, "smartNarrative", "Narrative", "Automatic text summary").clicked() {
                crate::report::insert_visual(app, VisualKind::SmartNarrative);
            }
        });
    });
    group(ui, t, "Script visuals", |ui| {
        if big_button(ui, t, "python", "Python", "A visual drawn by a Python script (matplotlib)").clicked() {
            crate::report::insert_visual(app, VisualKind::PythonVisual);
        }
        if big_button(ui, t, "r", "R", "A visual drawn by an R script").clicked() {
            crate::report::insert_visual(app, VisualKind::RVisual);
        }
    });
    group(ui, t, "Elements", |ui| {
        if big_button(ui, t, "text", "Text box", "Rich text with live values: type {[Measure]}").clicked() {
            crate::report::insert_text(app);
        }
        let r = big_button(ui, t, "button", "Buttons ⌄", "Navigation and action buttons");
        egui::Popup::menu(&r).show(|ui| {
            for b in ["Blank", "Back", "Bookmark", "Information", "Help", "Q&A", "Arrow", "Reset", "Page navigator", "Bookmark navigator"] {
                if ui.button(b).clicked() {
                    crate::report::insert_button(app, b);
                    ui.close();
                }
            }
        });
        let r = big_button(ui, t, "shape", "Shapes ⌄", "Figma-style shapes");
        egui::Popup::menu(&r).show(|ui| {
            for s in ShapeKind::ALL {
                if ui.button(format!("{s:?}")).clicked() {
                    crate::report::insert_shape(app, s);
                    ui.close();
                }
            }
        });
        if big_button(ui, t, "image", "Image", "Insert a picture").clicked() {
            crate::report::insert_image(app);
        }
    });
}

fn modeling_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Relationships", |ui| {
        if big_button(ui, t, "relationship", "Manage relationships", "Create and edit relationships").clicked() {
            crate::model::manage_relationships(app);
        }
    });
    group(ui, t, "Calculations", |ui| {
        if big_button(ui, t, "measure", "New measure", "").clicked() {
            crate::model::new_measure(app, None);
        }
        if big_button(ui, t, "column", "New column", "").clicked() {
            crate::model::new_column(app, None);
        }
        if big_button(ui, t, "table", "New table", "Calculated table (DAX)").clicked() {
            crate::model::new_table(app);
        }
        column(ui, |ui| {
            if small_button(ui, t, "quick", "Quick measure", "").clicked() {
                crate::model::quick_measure(app);
            }
            if small_button(ui, t, "calc", "Calculated field", "Tableau syntax").clicked() {
                crate::model::calculated_field(app);
            }
        });
    });
    group(ui, t, "Parameters", |ui| {
        let r = big_button(ui, t, "parameter", "New parameter ⌄", "What-if, field and value parameters");
        egui::Popup::menu(&r).show(|ui| {
            if ui.button("Numeric range (what-if)").clicked() {
                crate::model::new_parameter(app, "whatif");
                ui.close();
            }
            if ui.button("Fields (switch columns/measures)").clicked() {
                crate::model::new_parameter(app, "fields");
                ui.close();
            }
            if ui.button("Value parameter (Tableau)").clicked() {
                crate::model::new_parameter(app, "value");
                ui.close();
            }
        });
    });
    group(ui, t, "Security", |ui| {
        if big_button(ui, t, "security", "Manage roles", "Row-level security").clicked() {
            crate::model::manage_roles(app);
        }
        if big_button(ui, t, "security", "View as", "See the report as a role").clicked() {
            crate::model::view_as(app);
        }
    });
    group(ui, t, "Calendars", |ui| {
        if big_button(ui, t, "table", "Mark as date table", "").clicked() {
            crate::model::mark_date_table(app);
        }
    });
}

fn view_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Themes", |ui| {
        let r = big_button(ui, t, "theme", "Themes ⌄", "Report theme");
        egui::Popup::menu(&r).show(|ui| {
            for name in vizcraft_core::color::Theme::NAMES {
                if ui.button(*name).clicked() {
                    if let Some(th) = vizcraft_core::color::Theme::by_name(name) {
                        app.checkpoint("Theme");
                        app.project.report.theme = th;
                        crate::ai::track_theme(app, name);
                    }
                    ui.close();
                }
            }
        });
    });
    group(ui, t, "Scale to fit", |ui| {
        column(ui, |ui| {
            if small_button(ui, t, "", "Fit to page", "").clicked() {
                app.report.fit = true;
            }
            if small_button(ui, t, "", "Fit to width", "").clicked() {
                app.report.fit_width = true;
            }
            if small_button(ui, t, "", "Actual size", "").clicked() {
                app.report.zoom = 1.0;
            }
        });
    });
    group(ui, t, "Mobile", |ui| {
        let lbl = if app.report.mobile { "Desktop layout" } else { "Mobile layout" };
        if big_button(ui, t, "mobile", lbl, "Design the phone layout of this page").clicked() {
            app.report.mobile = !app.report.mobile;
        }
    });
    group(ui, t, "Page options", |ui| {
        ui.vertical(|ui| {
            let s = &mut app.project.report.settings;
            ui.checkbox(&mut s.show_gridlines, "Gridlines");
            ui.checkbox(&mut s.snap_to_grid, "Snap to grid");
            ui.checkbox(&mut s.lock_objects, "Lock objects");
        });
    });
    group(ui, t, "Show panes", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.toggle_value(&mut app.panes.filters, "Filters");
                ui.toggle_value(&mut app.panes.visualizations, "Visualizations");
                ui.toggle_value(&mut app.panes.data, "Data");
            });
            ui.horizontal(|ui| {
                ui.toggle_value(&mut app.panes.bookmarks, "Bookmarks");
                ui.toggle_value(&mut app.panes.selection, "Selection");
                ui.toggle_value(&mut app.panes.sync_slicers, "Sync slicers");
            });
            ui.horizontal(|ui| {
                ui.toggle_value(&mut app.panes.performance, "Performance analyzer");
            });
        });
    });
    group(ui, t, "Interactions", |ui| {
        ui.vertical(|ui| {
            let mut ch = app.project.report.settings.cross_highlight;
            if ui.checkbox(&mut ch, "Cross-highlight (off = cross-filter)").changed() {
                app.project.report.settings.cross_highlight = ch;
                app.cache.clear();
            }
            ui.toggle_value(&mut app.report.edit_interactions, "Edit interactions");
        });
    });
}

fn data_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Get data", |ui| {
        for (k, icon, label) in [("csv", "data", "Text/CSV"), ("excel", "sheet", "Excel"), ("json", "data", "JSON"), ("web", "service", "Web"), ("sqlite", "data", "SQLite"), ("postgresql", "data", "PostgreSQL")] {
            if big_button(ui, t, icon, label, "").clicked() {
                crate::pq::get_data(app, k);
            }
        }
    });
    group(ui, t, "Prepare", |ui| {
        if big_button(ui, t, "transform", "Power Query", "Clean and shape data").clicked() {
            crate::pq::open_editor(app, None);
        }
        if big_button(ui, t, "relationship", "Joins & unions", "Tableau-style data source canvas").clicked() {
            crate::pq::data_source_canvas(app);
        }
        if big_button(ui, t, "sheet", "Data sheets", "Edit data like Excel").clicked() {
            app.view = View::Sheets;
            app.pq_open = false;
        }
    });
    group(ui, t, "SQL analytics", |ui| {
        if big_button(ui, t, "sql", "SQL", "SQL workbench: DuckDB over the model tables, files and the project warehouse").clicked() {
            crate::sqlview::open(app);
        }
        column(ui, |ui| {
            if small_button(ui, t, "transform", "SQL query (Power Query)", "A Power Query query that starts from VizCraft.Sql(...)").clicked() {
                crate::sqlview::get_data_sql(app);
            }
            if small_button(ui, t, "python", "Python over SQL", "Analyze a SQL result with pandas").clicked() {
                crate::sqlview::open(app);
            }
        });
    });
    group(ui, t, "Fabric", |ui| {
        if big_button(ui, t, "flow", "New flow", "A pipeline of steps: prepare, ML, output tables").clicked() {
            crate::flows::new_flow(app);
            app.view = View::Flows;
            app.pq_open = false;
        }
        if big_button(ui, t, "python", "Notebook", "Python notebook cells over your tables").clicked() {
            app.view = View::Flows;
            app.flows_ui_open_notebook();
        }
    });
    group(ui, t, "Refresh", |ui| {
        if big_button(ui, t, "refresh", "Refresh all", "").clicked() {
            app.mark_data_dirty();
        }
    });
}

fn publish_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Publish", |ui| {
        if big_button(ui, t, "service", "VizCraft Service", "Publish to a VizCraft Service workspace (self-hosted portal)").clicked() {
            crate::publish::open(app, "service");
        }
        if big_button(ui, t, "publish", "GitHub Pages", "Publish to GitHub Pages with your own domain").clicked() {
            crate::publish::open(app, "github");
        }
        if big_button(ui, t, "export", "Single file", "One self-contained HTML file you can share anywhere").clicked() {
            crate::publish::open(app, "single");
        }
        if big_button(ui, t, "open", "Web folder", "A static web site folder (any host: Netlify, S3, nginx…)").clicked() {
            crate::publish::open(app, "folder");
        }
    });
    group(ui, t, "Layouts", |ui| {
        if big_button(ui, t, "mobile", "Mobile layout", "Arrange visuals for phones").clicked() {
            app.report.mobile = true;
            app.view = View::Report;
        }
    });
    group(ui, t, "Export", |ui| {
        if big_button(ui, t, "export", "SVG", "Export the page as SVG").clicked() {
            crate::publish::open(app, "svg");
        }
    });
}

fn help_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Learn", |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("VizCraft").strong());
            ui.label("Power BI-style modeling + Tableau-style worksheets, dashboards and stories,");
            ui.label("Figma-style design elements, Excel-style data sheets and Fabric-style flows.");
            ui.label("Tip: any text can show live numbers: type {[Total Sales]:$#,0} in a title.");
        });
    });
    group(ui, t, "Shortcuts", |ui| {
        ui.vertical(|ui| {
            ui.label("Ctrl+Z / Ctrl+Y  undo / redo · Ctrl+S save · F5 refresh");
            ui.label("Ctrl+C / Ctrl+V / Ctrl+D copy, paste, duplicate · Del delete");
            ui.label("Arrows nudge · Shift+arrows nudge 10 · Ctrl+G group · Ctrl+Shift+G ungroup");
        });
    });
    let _ = app;
}

fn run_search(app: &mut VizApp, q: &str) {
    for k in VisualKind::ALL {
        if k.label().to_lowercase().contains(q) {
            crate::report::insert_visual(app, *k);
            return;
        }
    }
    let actions: &[(&str, fn(&mut VizApp))] = &[
        ("new measure", |a| crate::model::new_measure(a, None)),
        ("relationship", crate::model::manage_relationships),
        ("power query", |a| crate::pq::open_editor(a, None)),
        ("transform", |a| crate::pq::open_editor(a, None)),
        ("refresh", |a| a.mark_data_dirty()),
        ("publish", |a| crate::publish::open(a, "service")),
        ("parameter", |a| crate::model::new_parameter(a, "whatif")),
        ("text box", crate::report::insert_text),
        ("new page", |a| crate::report::new_page(a, PageKind::Report)),
        ("bookmark", |a| a.panes.bookmarks = true),
        ("sheet", crate::sheet::new_sheet),
        ("flow", crate::flows::new_flow),
        ("sql", crate::sqlview::open),
        ("duckdb", crate::sqlview::open),
    ];
    for (k, f) in actions {
        if k.contains(q) || q.contains(k) {
            f(app);
            return;
        }
    }
    app.toast(format!("No command matches “{q}”."));
}

impl VizApp {
    pub fn flows_ui_open_notebook(&mut self) {
        self.pq_open = false;
        self.view = View::Flows;
    }
}
