# Project files, Git and agents

VizCraft saves a project in one of two formats. Both hold exactly the same project and convert
losslessly.

| Format | Like Power BI | Use it for |
| --- | --- | --- |
| **`.vizc`**: one JSON file | `.pbix` | sharing a single file, quick work |
| **`.vizp`**: a pointer file plus folders | `.pbip` + PBIR / TMDL | Git, code review, CI, AI agents, VS Code |

Save as either from **File → Save as** (pick the type), or **File → Save as folder**. Open either
with **File → Open**, by dropping it on the window, or from the welcome screen's recent list.

## The folder format (`.vizp`)

```text
Sales.vizp                          pointer: name, version, idSeed, the folders below
Sales.SemanticModel/
  model.json                        relationships, roles, parameters, culture, layouts
  tables/<Table>.json               one file per table (columns, hierarchies, source)
  measures.json                     measure metadata (table, name, format, folder, file)
  measures/<Table>/<Measure>.dax    the DAX of each measure
  columns.json, columns/<Table>/<Column>.dax   calculated columns
  queries.json, queries/<Query>.m   Power Query (M), one script per query
  parameters.json                   query parameters
Sales.Report/
  report.json                       theme, settings, report filters, bookmarks, stories, page order
  pages/page-<id>/page.json         page name, size, background, filters, visual order
  pages/page-<id>/visuals/<id>.json one file per visual, shape, text box, image or button
Sales.Data/
  sheets/<Sheet>.csv (+ .json)      entered data as CSV
  flows/, notebooks/                pipelines and notebooks
Sales.Science/
  workflows/<Workflow>.json         Data Science Studio workflows
  models/<name>-v<N>.json           registered model versions
Sales.Settings/publish.json         publishing targets
.gitignore  AGENTS.md  .mcp.json  .vscode/   written once, then yours to edit
```

- Pages and visuals are named by **id**, so renaming or reordering them changes one line, not file
  names. Pretty JSON with stable key order keeps `git diff` readable.
- Saving rewrites only changed files and deletes files the project no longer has (only inside the
  `Sales.*` folders, only `.json/.dax/.m/.csv`).
- **The app reloads the project when the files change on disk** (Git checkout, an agent, an
  editor), unless you have unsaved edits; then it tells you instead.

## Command line (for agents, scripts and CI)

`vizcraft-server` ships with the app (macOS: `/Applications/VizCraft.app/Contents/MacOS/`).

```bash
vizcraft-server convert Sales.vizc Sales.vizp        # or back: Sales.vizp Sales.vizc
vizcraft-server validate Sales.vizp                  # refresh, evaluate every measure; exit 1 on problems
vizcraft-server describe Sales.vizp                  # tables, columns, measures, pages as JSON
vizcraft-server dax Sales.vizp 'EVALUATE SUMMARIZECOLUMNS(Store[Region], "Sales", [Total Sales])'
vizcraft-server tools                                # every Copilot tool with its JSON schema
vizcraft-server tool Sales.vizp add_measure '{"name":"Margin %","dax":"DIVIDE([Total Profit],[Total Sales])","format":"0.0%"}'
```

`tool` runs exactly what the in-app Copilot runs (validated, then saved; `--dry-run` doesn't
save).

## MCP: Claude Code, VS Code, Cursor

```bash
vizcraft-server mcp Sales.vizp
```

This is a Model Context Protocol server on stdio exposing the Copilot tools (read the model, run
DAX/M, add measures, columns, relationships, queries, visuals, text, shapes, sheets, Studio
workflows, apply TMDL, and edit any part of the project). Each change is saved immediately, so an
open VizCraft window updates live. A saved folder project includes `.mcp.json`, so Claude Code
opened in that folder finds the server by itself.

## Copilot inside the app

Open it from any view with the **Copilot** button in the title bar or **Ctrl/⌘ + I**. It uses the
same tools, plus navigation (`navigate`) and app commands (save, undo, redo, publish). Mutating
steps ask before applying (unless auto-apply is on in AI hub → Privacy) and are undoable.
