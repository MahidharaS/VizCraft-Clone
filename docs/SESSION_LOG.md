# VizCraft — build log and hand-over

This document records the whole first build session of VizCraft: what was asked, how it was
approached, what was built, what each agent did, what is unfinished, and exactly where the next
session (or another account) should continue.

- Session dates: 2026-11-03 → 2026-11-04
- Original working repo: `digital-maven/all`, branch `claude/zen-ritchie-37yfoc`, folder `vizcraft/`
- This repo (`Digital-Maven/vizcraft`) holds the same history, exported with `git subtree split`.

---

## 1. The prompts (translated from Kannada/English)

### Prompt 1 — the product
> Create a new project. Look at storytold's "craft" apps (ArtCraft, GridCraft, FigCraft… open-source,
> pure-Rust clean-room versions of paid software like Figma/Excel/Word) at https://github.com/storytold
> and build, the same way, **every feature of Power BI**, plus the editable developer features of
> **Tableau** dashboards, stories and pages. Combine both, because Tableau has no model view and no
> Power Query.
>
> Power BI: Get data; Home tab; Insert tab; Modeling; transformations; pages; filters and
> cross-filtering per page; drill down / drill up; ready-made charts; forecasting and analytics;
> edit features; filter pane; data/fields pane; **Report view**, **Table view**, **Model view**
> (database-style diagram, edit relationships), **TMDL view** (write global DAX as code), and
> **Transform** opening the **Power Query Editor**. Reuse what was done for `vizclone` (Power Query +
> Power Pivot) in another chat and integrate those directly, as a separate editor like in Power BI.
> **Publishing**: like Power BI's own service with mobile and desktop views, plus *new* options Power
> BI doesn't have: publish like a Claude artifact, or to our own domain via GitHub. Integrate
> **Fabric** features (step-by-step pipelines like KNIME). If open source is needed, borrow from the
> **Orange** data mining tool.
>
> From Tableau: fully dynamic formatting — in Power BI you can't drive conditional formatting or
> titles from your own numbers; in Tableau you can put numbers and conditional formats into a title.
> Also missing from both: **Figma-like elements** (shapes, separately styled elements) for building
> beautiful dashboards — take from FigCraft. When a chart is on the page, an **arrow icon** should
> open that chart alone on a **new sheet/page** (like Tableau sheets); dashboards stay pages; to put
> dashboards side by side use **stories**. Research all Tableau dashboard/story features. Custom
> visuals with **Python or R**. Many data sources, joins/merges on the data source page (which should
> go into Power Query). Tableau's multi-model data sources, **parameters** (switch columns), calculated
> parameters, **calculated fields**. Power BI **bookmarks** and panes. Excel-level **freedom to edit
> data**: a separate Excel-like editor (cleaning goes to Power Query, entering data goes here).
>
> Name: **VizCraft**. Do it very thoroughly; give me a package — clone, get a **.dmg**, install it.

### Prompt 2 — AI, data science, SQL
> I haven't checked yet. Add an **Orange engine** as a **separate interface** (like Model view) to build
> **machine-learning and statistical models**. This isn't just a Power BI + Tableau merge. Integrate
> **Microsoft Data Formulator** as another interface. Integrate **AI**: users' own API keys or **local
> models** (open-source models running in a terminal). The core differentiator: **every installation /
> account becomes its own learning model** — like a Meta Ads account learns from every campaign, track
> the user's habits (pipelines, visualization style…) and personalise. Use LangChain / LangSmith /
> AutoGen / CrewAI or anything; Hugging Face is available. Integrate **OpenJEV** (hosted on Hugging
> Face; a local decision-making model). Add a small **SQL engine** (MariaDB/DuckDB). Full analytics,
> including some Python. A packed tool for data analysts and data scientists, including **MLOps**
> (deploy and fine-tune models).
> Links: https://github.com/biolab/orange3 · https://huggingface.co/openjev/openjev ·
> https://zefan-cai.github.io/open-jev/story/#method · https://github.com/microsoft/data-formulator

### Prompt 3
> Pause for now; let's see this tomorrow.

### Prompt 4
> Push everything to the public repo **vizcraft** I gave you access to. Spend at most ~$10; use the
> rest to write detailed documentation of everything: my prompts, what you did, your responses, your
> thinking, what agents are running, where they stopped, where the next session must continue.

---

## 2. How the problem was approached (reasoning)

1. **Study the references first.** storytold's apps are Rust + egui with engine crates that have no UI
   dependency, one document format, code-drawn icons, and "everything is a command". `vizclone` in
   `digital-maven/all` already had two valuable engines: a **DAX engine** (`crates/datamodel`) and a
   **Power Query M engine** (`crates/query`, ~300 functions). Both only depended on serde/chrono, so
   they were copied into the new workspace rather than rewritten.
2. **Can't create a GitHub repo from the session** (tool restricted), so the project started as the
   `vizcraft/` folder in `digital-maven/all`, written as a standalone workspace so it could be moved
   later (which is what happened in prompt 4).
3. **Design the core contract myself, then fan out.** The risky part of a project this size is
   integration. So I first wrote the shared foundation that everything else plugs into:
   - `doc.rs`: one serializable `Project` with every concept (queries, sheets, model, pages of kinds
     Report/Worksheet/Dashboard/Tooltip/Drillthrough, elements = visuals/shapes/text/images/buttons/
     groups, `Prop` values that can be **fx-bound**, filters, bookmarks, stories, parameters, flows,
     publish targets).
   - `build.rs`: refresh = evaluate M queries + sheets → typed tables → DAX model (with auto date
     hierarchy columns, calculated tables/columns, relationships, measures, parameter tables).
   - `state.rs` + `vquery.rs`: filter context (report/page/visual filters, slicers incl. synced,
     cross-filter/highlight with *Edit interactions*, drill-through, RLS) → DAX `SUMMARIZECOLUMNS`
     query → `VisualData`.
   - `fx.rs`: the Tableau-style dynamic formatting the user wanted, generalised: **any** property can
     be a template (`{[Total Sales]:$#,0}`), a DAX expression, rules, a gradient or a field value.
   - `scene.rs`: charts draw into a renderer-independent `Scene`, painted by egui on desktop and
     written as SVG on the web — so published reports look identical and the browser can run the same
     Rust engine compiled to WebAssembly.
   - `format.rs`: the Format pane is generated from a schema, so every property automatically gets an
     fx button.
4. **Parallelise with isolated agents.** Each feature area went to a sub-agent in its own git worktree
   with a shared brief (`common.md`: layout, rules, egui 0.37 API notes, how to screenshot under Xvfb,
   commit rules) and explicit file ownership, so merges would be clean. I kept the report canvas and
   integration for myself.
5. **Verify by looking.** The app was run under `xvfb-run` with a `--screenshot` flag added to the app,
   plus a synthetic-input hook (`VIZCRAFT_EVENTS="10:click:700,464"`) to test clicks without a mouse.
   This caught real bugs (date hierarchy blank, gradient range wrong, ribbon layout).
6. **Honesty constraints.** No vendor code/icons/fonts. Orange is GPL → only its UX was studied;
   algorithms are original implementations. OpenJEV weights are 27B and CC BY-NC → integrated as a
   client to a locally served model, not bundled. A .dmg can't be built on Linux → GitHub Actions on a
   macOS runner builds it.

---

## 3. What exists now (merged, building, tested)

Build status at the last full check: `cargo build -p vizcraft` OK, `cargo test -p vizcraft-core` **46
passed**. The last merge (Power Query) was committed but its full build was interrupted by the pause —
**re-run the build first** (see §6).

| Area | Where | Status |
| --- | --- | --- |
| Workspace, licences, README | `Cargo.toml`, `LICENSE-*`, `NOTICE`, `README.md` | done |
| DAX engine (+ many new functions: STDEV/VAR/PERCENTILE, set ops, time intelligence, FORMAT…) | `crates/datamodel` | done, tested |
| M engine (+ DB connector hook, `Web.Page`, `Python.Execute`, trig) | `crates/query` | done, tested |
| Core: document, refresh, filter context, visual queries, fx, scenes, format schema, sample | `crates/core/src` | done, tested |
| Charts: every visual kind (~45), axes, legends, labels, small multiples, maps, matrix, slicers, AI visuals | `crates/core/src/charts/` | done, tested (SVG gallery test) |
| Analytics: ref lines, trend, Holt-Winters forecast, k-means, anomalies, error bars | `crates/core/src/analytics.rs` | done |
| TMDL write/apply, Tableau calc → DAX translator | `crates/core/src/{tmdl,tableau}.rs` | done, tested |
| Data sheets formula engine; flows engine; ML library | `crates/core/src/{sheet,flow,ml}.rs` | done, tested |
| Connectors: files, folders, Excel, web/OData, SQLite, PostgreSQL, MySQL, XML, Parquet, Python | `crates/connectors` | done; PG/MySQL untested against servers; SQL Server/ODBC = "not yet" |
| Desktop shell: ribbon, rail, title bar, status bar, undo/redo, open/save `.vizc`, dark mode, screenshot + synthetic input hooks | `apps/vizcraft/src/{app,ribbon,widgets,theme,icons,paint}.rs` | done |
| Report view: canvas (move/resize/rotate, smart guides, marquee, groups), visual header (drill, focus, **↗ open as sheet**), panes (Visualizations Build/Format/Analytics, Data, Filters, Bookmarks, Selection, Sync slicers, Performance), fx dialog, Figma-style design props, worksheets (shelves, Marks, Show Me), dashboards (+ actions), stories, mobile layout designer, reading view, Python/R visuals | `apps/vizcraft/src/report/` | done |
| Power Query Editor, Get Data, data source canvas (joins/unions) | `apps/vizcraft/src/pq/` | done |
| Table / Model / DAX query / TMDL views + modeling dialogs | `apps/vizcraft/src/model/` | done |
| Data sheets view (Excel-like) and Flows view (+ notebooks) | `apps/vizcraft/src/{sheet,flows}/` | done |
| Publishing: web bundle, single-file HTML, WASM viewer, service publish, GitHub Pages, SVG | `crates/publish`, `crates/wasm`, `apps/vizcraft/src/publish/` | done; tested in headless Chromium |
| VizCraft Service portal + CLI (`serve`, `publish`, `export`, `render`) | `apps/vizcraft-server` | done |
| Packaging + CI (DMG, Linux tar.gz/AppImage, Windows zip/MSI) | `packaging/`, `.github/workflows/release.yml` | written; Linux package tested; **DMG never run** (no Mac) |
| Docs | `docs/publishing.md`, this file | done |

Known issues reported by agents (not yet fixed):
- A few font glyphs still render as boxes in places (title-bar search icon, status "●"); the workspace
  enables egui `monochrome_emoji_fonts` + eframe `system_fonts`, which should cover most — verify.
- Many-to-many relationships are stored/drawn but not used for filtering.
- Matrix "stepped off" (tabular) layout not implemented; no chart zoom slider.
- Sorting a data sheet doesn't rewrite formulas.
- PQ gear dialogs re-edit M rather than reopening the original dialog; fuzzy merge missing.
- Mouse drags (model view cards, relationship creation, sheet fill handle) weren't exercised headlessly.
- The sample's Targets make KPI/bullet goals look odd (data issue, not code).

---

## 4. Agents and where each stopped

| # | Agent (scope) | Result |
| --- | --- | --- |
| A | Charts & analytics | finished, merged |
| B | Power Query editor + connectors | finished, merged (last merge; build not re-verified) |
| C | Model/Table/DAX/TMDL views, DAX functions, Tableau calcs, TMDL | finished, merged |
| D | Data sheets, flows, ML library, notebooks | finished, merged |
| E | Publishing, WASM viewer, service, packaging, CI | finished, merged |
| F | **Data Science Studio** (Orange-style canvas), statistics, **MLOps** | **stopped at the user's request** (was writing the `crates/mlops` crate) → saved in `wip/studio-mlops.patch` |
| G | **AI**: providers (own key / local), Copilot, **Data Formulator** view, **OpenJEV** client, **personal brain** | **stopped** (was waiting on a build) → `wip/ai-formulator.patch` |
| H | **DuckDB SQL engine** + SQL view + Python cells | **stopped** (DuckDB was compiling) → `wip/sql-duckdb.patch` |

No agents are running now.

The three WIP patches are `git format-patch` series against commit `005a830` of the original repo
(the export keeps the same tree, so they apply to this repo's matching history). They contain
partially working code that was **never merged or build-checked together**.

---

## 5. What the unfinished agents were asked to build (so a new session can resume)

### F — Data Science Studio + statistics + MLOps
- New rail view "studio": Orange-like widget canvas (Data, Transform, Model, Evaluate, Visualize,
  Unsupervised, Statistics, Deploy), widget windows (data table, scatter, distributions, box plot,
  heat map, confusion matrix, ROC, Test & Score, dendrogram, silhouette, PCA, feature stats,
  correlations, regression summaries, ARIMA forecast).
- New algorithms on top of `ml.rs`: gradient boosting, AdaBoost, linear SVM, MLP, ridge/lasso/elastic
  net, cross-validation, ROC/AUC/lift/calibration, MDS/t-SNE, feature ranking, permutation importance.
- `stats`: OLS with full inference, logistic with p-values, GLM Poisson, t-tests, ANOVA + Tukey,
  chi², correlation tests, normality, Mann-Whitney, Kruskal-Wallis, decomposition, ACF/PACF, ARIMA,
  ETS, special functions (gamma/beta/CDFs).
- `crates/mlops` + `apps/vizcraft-mlserve`: registry with versions/stages in `~/.vizcraft/models`,
  `POST /models/<name>/predict`, batch scoring, PSI/KS drift, export to JSON + Python scorer.
- Example workflows on the Retail sample.

### G — AI hub, Copilot, Formulator, OpenJEV, personal brain
- `crates/ai`: providers (OpenAI-compatible incl. Ollama/LM Studio/vLLM/llama.cpp/Groq/OpenRouter,
  Anthropic, Gemini, Hugging Face router); keys in `~/.vizcraft/ai.json` or env vars; local server
  auto-detection.
- Decision engine trait; **OpenJEV** client for `POST http://localhost:3000/v1/systemone`
  (`choice` / `noul` / `score` questions with probabilities). OpenJEV is 27B (GGUF Q4 ≈16.5 GB, MLX
  4-bit ≈15 GB), weights CC BY-NC 4.0 → user downloads/serves it; app degrades gracefully.
- Native Rust agent loop with tools (describe_model, run_dax, run_m, preview_table, create_visual,
  set_format, add_measure, add_filter, create_page, …) and a trace viewer (LangSmith-like). LangChain
  & co. are Python, so the equivalent was built natively.
- **Personal brain** (the differentiator): opt-in local event log `~/.vizcraft/brain/events.jsonl`;
  online learners: chart-type recommender (contextual logistic regression), formatting preferences,
  Power Query step n-gram model, BM25 memory of past questions, Thompson-sampling bandit on
  suggestion acceptance; "Forget everything"; export to instruction JSONL + `finetune_lora.py`
  (transformers + peft on a small model, then serve via Ollama/llama.cpp).
- UI: AI settings hub, Copilot pane (confirm before applying, undoable), **Formulator** view (data
  threads that branch, AI-derived fields, pin to report), Brain suggestion chips in the Build pane.
- `docs/ai.md`.

### H — SQL engine
- `crates/sql`: DuckDB (`bundled`) engine; register model tables (dates mapped from serials); files
  via read_csv/read_parquet/read_json; optional on-disk `<project>.duckdb` warehouse; worker thread,
  cancel; EXPLAIN.
- SQL view: object explorer, multi-tab editor with highlighting and autocomplete, Ctrl+Enter, results
  grid, export, history, "Create chart", "Load to model"; Python tab with `df`.
- M function `VizCraft.Sql("...")`; `docs/sql.md`. DuckDB from source takes 10–20 min to compile.

---

## 6. How the next session should continue (step by step)

1. Clone this repo; install Rust stable; on Linux also `libxkbcommon-x11-0` (and GTK dev libs for
   file dialogs) to run the app.
2. **Verify the current tree builds and tests pass:**
   ```bash
   cargo build -p vizcraft -p vizcraft-server
   cargo test -p vizcraft-core -p vizcraft-datamodel -p vizcraft-query -p vizcraft-connectors
   ```
   Fix anything from the last (Power Query) merge first.
3. Run and look: `cargo run -p vizcraft -- --sample` (views: `--view report|table-view|model|dax|tmdl|sheet|flow|story|pq`;
   headless: `xvfb-run -a ./target/debug/vizcraft --sample --screenshot out.png`).
4. Resume the three WIP areas **one at a time** (they all add a rail view in `app.rs`, so do them
   sequentially): `git am wip/sql-duckdb.patch` (or `git apply --3way`), build, fix, test, commit;
   then `wip/ai-formulator.patch`; then `wip/studio-mlops.patch`. The patches were cut from commit
   `005a830` (before the later merges), so expect small conflicts in `app.rs`, `main.rs`, `icons.rs`,
   `ribbon.rs`, `Cargo.toml` — keep both sides (they are additive). The studio patch already includes
   a merge of agent D's branch, so drop its duplicate `ml.rs`/`flow.rs` hunks if they conflict.
   Then complete the remaining items listed in §5 for each.
5. Fix the known issues in §3.
6. Push to `main` → GitHub Actions builds the macOS **.dmg**, Windows zip and Linux tar.gz
   (Actions → latest run → Artifacts). Tag `vizcraft-v0.1.0` for a Release. The DMG job has never run;
   expect to fix small script issues on the first run.

---

## 7. Responses given to the user during the session (summary)

- After prompt 1: explained the plan (Rust/egui like the Craft apps, reuse of vizclone's DAX + M
  engines, the fx system, scene-based rendering for identical desktop/web output), that the repo
  couldn't be created from the session so it lives in `vizcraft/`, and that 5 agents were building
  charts, Power Query, model views, sheets/flows and publishing/packaging in parallel while I built
  the report view.
- After prompt 2: described the three new builds (Studio + MLOps, AI layer with own-key/local models,
  Formulator, OpenJEV and the personal brain, DuckDB SQL) and the two caveats: OpenJEV is too large to
  bundle and is non-commercial; DuckDB makes the first build slower.
- During merges: reported each agent's completion, merge, build and test status.
- After prompt 3: stopped all agents, saved their unfinished work as patches, pushed.
- After prompt 4: exported the project to `Digital-Maven/vizcraft` with full history, adapted the CI
  workflow to the repo root, and wrote this document.

---

## 8. Session 2 (2026-11-04, continued from another account in `MahidharaS/VizCraft-Clone`)

- Imported this repo's full history into `MahidharaS/VizCraft-Clone` (branch
  `claude/affectionate-archimedes-03idy6`).
- Verified §6 step 2: `cargo build -p vizcraft -p vizcraft-server` OK; core/datamodel/query/connectors
  tests all pass. The Power Query merge is fine.
- **H — SQL engine: merged.** Applied `wip/sql-duckdb.patch` (use `git apply -p2 --3way`, the patches
  carry the old `vizcraft/` prefix). Conflicts in `Cargo.toml` and `query/src/funcs/source.rs` were
  additive. Fixes: DuckDB now builds with `parquet` + `json` features (it otherwise tried to download
  extensions at runtime); `SELECT TOP n` rewrite left a double space. `cargo test -p vizcraft-sql`:
  11 passed. SQL view checked headlessly: query runs, results grid shows typed columns.
- Fixed an egui 0.37 issue across the app: `TextEdit::frame(..)` overrides `.margin(..)`, so text
  drew under the SQL editor's line-number gutter (also DAX editor, sheet cell editor, Enter-data grid).
  Margins now live in the frame's `inner_margin`.
- Headless runs need `apt-get install libxkbcommon-x11-0`.
- **G — AI hub / Copilot / Formulator: merged.** Applied `wip/ai-formulator.patch` (conflicts in
  `Cargo.toml`, `apps/vizcraft/Cargo.toml`, `app.rs` were additive: View enum now has Sql, Formulator,
  Ai). Fixed 8 compile errors (name clash `channel`, egui 0.37 `frame(bool)`→`Frame`, two borrow
  errors). `cargo test -p vizcraft-ai`: 29 passed. Formulator fixes found by looking at it:
  - "Region" holding East/North/South/West was classed as geographic → auto Map with "No locations
    found". Geo-by-name now needs most values to resolve in the gazetteer (lat/lon always count).
  - Offline rules picked Store[Region] for x and Targets[Region] for colour → every bar split in 4.
    Same column name from another table is no longer used as colour; colour equal to the axis field
    is ignored in the spec (regression test added).
  - Thread cards pushed the resizable panel wider and the main area overflowed; fixed width cards.
  - Derived-data grid showed the same field twice.
- Not exercised: real LLM calls (no keys in the session), OpenJEV server, Copilot apply flow.
- **F — Data Science Studio + MLOps: merged and finished.** Patches 1–3 of `wip/studio-mlops.patch`
  were already merged (agent D), so only 4–5 were applied (statistics library, studio engine,
  `crates/mlops`). Missing pieces written this session:
  - `apps/vizcraft-mlserve/src/main.rs` (the binary had only a Cargo.toml).
  - The whole Studio UI (`apps/vizcraft/src/studio/`): palette, zoomable canvas with typed ports and
    wiring, settings inspector for every parameter kind, widget windows for every view (reports,
    scores, confusion matrix, ROC/lift/calibration, scatter, histogram, box plot, heat map,
    correlations, sieve/mosaic, line, dendrogram, silhouette, PCA, forecast, decomposition,
    ACF/PACF, distribution fits, drift), model registry panel (stages, export), Register / Save
    as Table actions, built-in model server. Rail icon + **Home → AI → Studio** button.
  - Verified headlessly on all 4 examples; registered a model from the UI, served it, and got a
    prediction back over HTTP from both the in-app server and `vizcraft-mlserve`.
  - `vizcraft-mlserve` is now included in the macOS, Windows and Linux packages. `docs/studio.md`.
- Repository independence: this repo is not a fork, has no submodules and only one remote
  (`MahidharaS/VizCraft-Clone`); README, Cargo metadata and docs now point here. Deleting the
  original repositories does not affect it.
- Still open from §3 (known issues): many-to-many filtering, matrix tabular layout, chart zoom
  slider, sheet sort not rewriting formulas, PQ gear dialogs, fuzzy merge; AI calls with real keys
  and OpenJEV were not exercised (no keys/servers here); the DMG has to come from the CI run.

## 9. Session 2, second request: polish, local models, Copilot everywhere, Git format

The user asked for: every kind of fine-tuning and UI/UX improvement (logos etc.); local AI models
installed through the terminal weren't detected; a Copilot that controls the whole app (like Claude
in Excel), not just one pane; a PBIP/PBIR-style Git format that agents can edit from the terminal or
VS Code; and a decision on how project files are stored.

- **File formats** (`docs/file-formats.md`): `.vizc` (single JSON file, like .pbix) and `.vizp`
  (pointer + folders, like .pbip/PBIR: measures as .dax, queries as .m, tables/pages/visuals as
  JSON by id, sheets as CSV). `crates/core/src/project_folder.rs`, lossless round-trip test. The
  app opens/saves both, drops, recent list, and reloads on outside changes.
- **Agents**: `vizcraft-server convert | validate | describe | dax | tools | tool | mcp`. The MCP
  server exposes the Copilot tools over stdio; saved folders get `.mcp.json`, `.vscode/` and
  `AGENTS.md`.
- **Copilot everywhere**: title-bar button + Ctrl/⌘+I in every view; 20 new tools
  (`get_project`/`edit_project` for any part of the document, TMDL, relationships, calculated
  columns, update query, delete, move, text, shapes, sheets/cells, Studio workflows/widgets,
  refresh, navigate, app commands). Tested in `whole_app_tools_edit_everything`.
- **Local models**: the probes used `localhost`, which on macOS can resolve to IPv6 while Ollama
  listens on IPv4 only, so running servers looked absent; now 127.0.0.1 (old settings migrate),
  `OLLAMA_HOST`, plus MLX, Jan, GPT4All. New `crates/ai/src/installed.rs` lists models on disk
  (Ollama store, LM Studio, GGUF files, Hugging Face/MLX cache), finds CLIs outside PATH (GUI apps
  on macOS don't get the shell PATH), and **Start & use** launches the server. A running local model
  is selected automatically at startup when nothing is configured. Verified with a fake Ollama.
- **UI**: welcome screen (quick starts, Copilot prompt, recent projects, blank report link), new
  logo (bars + spark) in the title bar, app icon updated to match, `.vizp` file association on macOS.
