<h1 align="center">VizCraft</h1>

<p align="center"><b>An open-source business-intelligence and data-science studio, written from scratch in Rust.</b><br>
Power BI's data prep and semantic model, Tableau's worksheets, dashboards and stories,
Figma-style design elements, Excel-style data sheets, Orange/KNIME-style machine learning,
an embedded SQL engine, and a local AI that learns how you work.</p>

VizCraft follows the pattern of the ArtCraft "Craft" apps (GridCraft, PhotoCraft…): a native
egui desktop app over engine crates with no UI dependency, one JSON project file, and
everything drawn in code. It runs on macOS, Windows and Linux.

## What's inside

| Area | What you get |
| --- | --- |
| **Get data** | Text/CSV, Excel, JSON, XML, Parquet, folders, web/REST/OData, SQLite, PostgreSQL, MySQL, enter data, blank query; Tableau-style data source canvas for joins and unions (generates Power Query M) |
| **Power Query** | A full Power Query Editor over an M engine with ~300 functions: applied steps, formula bar, advanced editor, column profiling, merge/append, column from examples, parameters |
| **Data sheets** | Excel-like grids with formulas (`=SUM(B2:B9)`, `VLOOKUP`, `IF`…), clipboard, fill, sort, filter, find & replace — enter or edit data with full freedom; sheets load into the model |
| **Model** | Table view, Model view (diagram, relationships with cardinality and cross-filter direction), DAX query view, TMDL view (write + apply), measures, calculated columns and tables, quick measures, hierarchies and auto date/time, roles and *View as*, what-if / field / value parameters |
| **Report** | Pages, ~45 visual types (every Power BI built-in plus Tableau "Show Me" charts), Build / Format / Analytics panes, filters (basic, advanced, Top N, relative date), slicers with sync, cross-filter and cross-highlight with *Edit interactions*, drill down/up/expand, drill-through, tooltip pages, bookmarks, buttons and navigators, Q&A, smart narrative, key influencers, decomposition tree, Python and R visuals, focus mode, spotlight, mobile layout |
| **fx everywhere** | Every format property can be bound to data: templates like `Sales {[Total Sales]:$#,0} in {Selected:Region}`, DAX expressions, rules, gradients and field values — titles, text boxes, shapes, colours, sizes, anything |
| **Tableau** | Worksheets with Columns/Rows shelves, Marks card and Show Me; the ↗ arrow on any visual opens it as its own sheet; dashboards (tiled/floating, dashboard actions: filter, highlight, URL, go to sheet, change parameter); stories with story points and captions; calculated fields in Tableau syntax (IF/THEN/ELSE, LOD `{FIXED …}`) translated to DAX |
| **Design** | Figma-style shapes (rect, ellipse, star, arrow, path, frames with auto layout), gradients, strokes, shadows, rotation, groups, layers, smart guides, align and distribute, rich text with per-run styles and data-bound colours |
| **Analytics** | Constant/average/median/percentile lines, trend lines, Holt-Winters forecasting with confidence bands, clustering, anomalies, error bars |
| **Data Science Studio** | Orange-style widget canvas: preprocessing, models (trees, forests, boosting, SVM, neural nets, regression, naive Bayes, kNN, clustering, PCA/t-SNE), evaluation (Test & Score, ROC, confusion matrix), statistics (regression with p-values, ANOVA, t-tests, chi², ARIMA) |
| **MLOps** | Model registry with versions and stages, deploy as a REST endpoint (`vizcraft-mlserve`), batch scoring, drift monitoring, export |
| **Flows & notebooks** | Fabric/KNIME-style node pipelines and Python/DAX/M notebook cells whose outputs become model tables |
| **SQL** | Embedded DuckDB over every model table, plus your own warehouse tables; SQL editor, results grid, Python cells |
| **AI** | Bring your own key (OpenAI, Anthropic, Gemini, Hugging Face) or a local model (Ollama, LM Studio, llama.cpp, vLLM); Copilot pane that builds visuals and writes DAX/M; a Data Formulator-style exploration view; OpenJEV as a decision engine; a personal model that learns your habits on your machine |
| **Publish** | VizCraft Service (self-hosted portal with workspaces, desktop and phone views), a static web folder, one self-contained HTML file to share anywhere, GitHub Pages with your own domain, SVG export. Published reports stay interactive (the engine runs in the browser as WebAssembly) |

## Install

### Download (macOS, Windows, Linux)
Every push (to `main` or a `claude/*` branch) builds installers in GitHub Actions (`.github/workflows/release.yml`):
open the latest **VizCraft release** workflow run → **Artifacts** → `VizCraft-macOS` (a `.dmg`),
`VizCraft-Windows` (zip) or `VizCraft-Linux` (tar.gz). Tags named `vizcraft-v*` also publish a GitHub Release.

The macOS build is ad-hoc signed, not notarized: the first time, right-click **VizCraft.app → Open**
(or run `xattr -dr com.apple.quarantine /Applications/VizCraft.app`).

### Build from source
```bash
git clone https://github.com/MahidharaS/VizCraft-Clone.git
cd VizCraft-Clone
cargo run --release -p vizcraft -- --sample
```
Make a DMG on a Mac (needs Xcode command-line tools):
```bash
./packaging/macos/make-dmg.sh
```
The first build compiles DuckDB from source and takes a while.

## Using it
- `vizcraft --sample` opens the Retail Sales sample: 4 queries and a data sheet, a star schema, measures, a 3-page report, a worksheet and a story.
- The rail on the left switches views: Report, Table, Model, DAX query, TMDL, Data sheets, Flows, Stories, SQL, Data Science Studio, Formulator, and the Power Query Editor.
- Any text accepts live values: `{[Measure]}`, `{[Measure]:0.0%}`, `{SUM(Sales[Quantity])}`, `{Selected:Region}`, `{@Parameter}`, `{Page}`, `{Today:yyyy-mm-dd}`.
- Ctrl+click runs a button while editing; **Reading view** behaves like a published report.

## Architecture
```
crates/datamodel   DAX engine: columnar tables, relationships, filter context, DAX functions
crates/query       Power Query M engine: lexer, parser, lazy evaluator, ~300 functions, steps
crates/core        the .vizc document, refresh/model build, visual queries, fx, scenes, charts,
                   analytics, TMDL, Tableau calcs, sheets, flows, ML, statistics
crates/connectors  files, Excel, web, databases (the Power Query host)
crates/sql         embedded DuckDB
crates/ai          providers, agent loop with tools, OpenJEV client, the personal brain
crates/mlops       model registry and serving
crates/publish     web bundles, single-file HTML, service and GitHub Pages publishing
crates/wasm        the engine compiled to WebAssembly for interactive published reports
apps/vizcraft      the egui desktop app
apps/vizcraft-server   the VizCraft Service portal and CLI (publish, export, render)
apps/vizcraft-mlserve  ML model serving
```
Visuals are drawn once into a renderer-independent `Scene`; the desktop paints it with egui and the
web writes it as SVG, so published reports look exactly like the desktop.

Docs: [publishing](docs/publishing.md) · [SQL](docs/sql.md) · [AI](docs/ai.md) · [Data Science Studio](docs/studio.md) · [build log and hand-over](docs/SESSION_LOG.md).

## Licence
MIT OR Apache-2.0. See [NOTICE](NOTICE): the DAX and M engines started from GridCraft (the
`vizclone/` project's engines, copied into this repository). VizCraft is not affiliated with Microsoft, Salesforce/Tableau, Figma,
Orange or KNIME; no vendor code, icons or artwork are used. OpenJEV weights are CC BY-NC 4.0 and are
not bundled — you download and run them yourself.
