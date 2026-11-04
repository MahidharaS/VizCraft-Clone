# SQL analytics (DuckDB) in VizardCraft

VizardCraft embeds [DuckDB](https://duckdb.org), an in-process analytical SQL engine, so you can
query everything in a project with SQL, keep your own tables in a per-project warehouse, analyze
results with Python (pandas), and feed any of it back into the semantic model and the report.

Open it from the **SQL** rail icon, **Home → Queries → SQL**, or **Data → SQL analytics**.

## What you can query

| Object | Where it comes from | How to refer to it |
|---|---|---|
| **Model tables** | Every table loaded into the model (Power Query queries, data sheets, flows, parameters) is registered after each refresh, with real types: `DATE`, `TIMESTAMP`, `TIME`, `BIGINT`, `DOUBLE`, `BOOLEAN`, `VARCHAR`. | `Sales`, `"Unit Price"` |
| **Warehouse tables** | Tables you create with `CREATE TABLE … AS …`. They live in `<project>.duckdb` next to the `.vizc` file (in memory until the project is saved). | `top_products` or `warehouse.main.top_products` |
| **Files** | **+ File** in the explorer attaches CSV/TSV, Parquet, JSON (and `.xlsx` when DuckDB's `excel` extension is available) as views. You can also read files directly: `read_csv_auto('…')`, `read_parquet('…/*.parquet')`, `read_json_auto('…')`. | `people` (the file name) |
| **Other databases** | **+ Database** attaches SQLite, PostgreSQL, MySQL/MariaDB (read-only) or another DuckDB file through DuckDB extensions. Extensions install on first use, which needs internet access once; offline, VizardCraft says so instead of failing. | `alias.schema.table` |

Name resolution order is warehouse → model → files, so a warehouse table can shadow a model
table of the same name (use `memory.main."Sales"` for the model one).

Model tables are a copy of the data of the last refresh; refreshing (or editing a data sheet)
re-registers the tables that changed. Calculated tables (DAX) aren't registered.

## The SQL view

* **Object explorer** (left): model tables, warehouse tables, views, files and attached
  databases with their columns and types. Double-click a name to insert it; right-click for
  *SELECT TOP 100*, *COUNT rows*, *Describe*, *Summarize* (column profile), *Load to model*,
  *Export…*, *Drop…* (opens the `DROP` statement in a tab for you to run) and *Detach file*.
  The **History** tab lists past queries (click: open, double-click: run); **Snippets** holds
  reusable SQL (a starter set is included; *+ Save current* adds yours).
* **Editor tabs**: syntax highlighting (keywords, functions, strings, numbers, comments, quoted
  identifiers), line numbers, the current statement highlighted, and autocomplete for tables,
  columns (`Sales.` or an alias `s.` lists that table's columns), functions and keywords
  (Tab/Enter accepts, Esc closes).
  * **Ctrl/Cmd+Enter** runs the statement under the cursor, or the selection.
  * **F5** runs the whole script (statements in order, stopping at the first error).
  * **Cancel** interrupts a long-running query.
  * **Explain** / **Explain analyze** show the plan (with timings for *analyze*); `EXPLAIN …`
    statements in a script show up as *Plan* tabs.
  * `SELECT TOP n …` is accepted and rewritten to `LIMIT n`.
* **Results**: one tab per result set, plus *Messages* (per-statement timing, rows, errors).
  The grid is virtualized (millions of rows scroll smoothly), with sticky headers and row
  numbers; click a header to sort, drag column borders to resize (double-click to fit), drag or
  Shift+click to select a range, **Ctrl+C** copies it as TSV (with a header row). The display
  is capped (100 … 1,000,000 rows, *Show* menu); **Export…** writes the *full* result to CSV,
  Parquet or JSON with DuckDB's `COPY … TO`.

## Getting results into the report

* **Load to model** (result toolbar, or *Load to model* on a warehouse table) creates a Power
  Query query named as you choose:

  ```m
  let
      Source = VizardCraft.Sql("select region, sum(amount) from Sales group by 1")
  in
      Source
  ```

  It loads as a model table and re-runs on each refresh, so you can add relationships,
  measures and visuals as for any other table, and add more Power Query steps after `Source`.
* **Create chart** does the same and inserts a visual over the new table on the current report
  page: a line chart for a date first column, columns/bars for a category, a card for a single
  number, otherwise a table.
* **Save to warehouse** stores the result as a warehouse table (`CREATE OR REPLACE TABLE`), your
  own "lakehouse" layer that persists with the project.

## Power Query: `VizardCraft.Sql`

`VizardCraft.Sql(sql as text) as table` (alias `Sql.Native`) runs SQL on the embedded engine over
the model tables and the warehouse. Dates come back as M dates, timestamps as datetimes.
Model tables a query mentions are loaded on demand during a refresh, so a SQL query can build
on other queries. **Get data → VizardCraft SQL (DuckDB)** (and **Data → SQL query (Power Query)**)
creates such a query and opens it in the Power Query Editor; for the Power Query owner the entry
point is `sqlview::get_data_sql(app)` and the connector kind is `"duckdb"`.

Hosts without the engine (the service, tests) answer with "The SQL engine isn't available
here" — the M function is defined by `vizardcraft_query::m::eval::Host::sql`, whose default
implementation returns that error.

## Python

A **Python** tab (tab bar *+ Python*, or *Python* on a result) runs `python3` with the last SQL
result as a pandas DataFrame `df`. Whatever you leave in `out` (a DataFrame, a Series, or
anything `pd.DataFrame()` accepts) comes back as a grid; `print` output and errors show in the
console above it. **Save out to warehouse** stores `out` as a table (then *Load to model*).

```python
# df: the last SQL result
monthly = df.groupby(df["month"].dt.to_period("M"))["revenue"].sum()
print(monthly.describe())
out = monthly.reset_index()
```

Data is passed through temporary CSV/JSON files, so only `pandas` is needed (no DuckDB or
Arrow Python packages). Set `VIZARDCRAFT_PYTHON` to use a specific interpreter. If Python or pandas
is missing, the console says how to install them.

## Files on disk

* `<project>.duckdb` — the warehouse (next to the `.vizc`; created on first use).
* `~/.vizardcraft/sql_history.jsonl` — query history (newest last; trimmed automatically).
* `~/.vizardcraft/sql_snippets.json` — your snippets.
  (`VIZARDCRAFT_HOME` overrides `~/.vizardcraft`.)

## Building

The engine is the `vizardcraft-sql` crate. Its `duckdb` feature (on by default, and enabled by the
desktop app) compiles DuckDB from source through the `duckdb` crate's `bundled` feature: it
needs a C++ compiler and takes a while the first time (10–20 minutes on 4 cores), after which it
is cached like any other dependency. Without the feature the crate still builds and the SQL view
reports that the engine isn't available.

## Demo / test hooks

* `VIZARDCRAFT_SQL_DEMO="select …"` (with `--view sql`) puts SQL in the first tab and runs it.
* `VIZARDCRAFT_SQL_DEMO_PY="out = df.describe()"` then runs a Python tab over the result.
