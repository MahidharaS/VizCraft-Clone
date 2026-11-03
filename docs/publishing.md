# Publishing VizCraft reports

A published VizCraft report is a small static website: no database, no server-side code. You
can host it anywhere: the VizCraft Service, any web server, GitHub Pages, or as a single HTML
file.

| File | What it is |
|---|---|
| `index.html` | The viewer: header, page tabs, Desktop/Mobile toggle, filter pane, bookmarks, story player, tooltips, Back button. |
| `viewer.js`, `viewer.css` | The viewer's code and style (no frameworks). |
| `report.json` | The report: pages pre-rendered to SVG (desktop and phone layouts, bookmarks, story points) and, in interactive mode, the project and a snapshot of its data. |
| `vizcraft_viewer.wasm` | Interactive mode only: the VizCraft engine compiled to WebAssembly. |
| `thumbnail.svg` | The first page, for portals and link previews. |

## Interactive and static reports

**Interactive** (default when the engine is built in): the same engine as the desktop app runs in
the browser. Slicers, cross-filtering and cross-highlighting (click a bar; Ctrl/⌘-click adds),
drill up/down/expand (hover a visual for its header), focus mode, what-if and field parameters
(filter pane), bookmarks, story points and page navigation buttons all work, offline.

**Static**: every page is pre-rendered to SVG in its desktop and phone layouts. Page tabs,
bookmarks (as pre-rendered views), story points, tooltips and navigation buttons work; slicers
and cross-filtering don't. Static reports are smaller and need no WebAssembly.

The engine is embedded into VizCraft when it was built first:

```sh
packaging/build-wasm.sh          # builds target/wasm32-unknown-unknown/release/vizcraft_wasm.wasm
cargo build --release -p vizcraft -p vizcraft-server
```

(`VIZCRAFT_VIEWER_WASM=/path/to/file.wasm` points the build at another copy.) The publish dialog
shows "Interactive engine built in" when it is there. The release packages always include it.

**What's in `report.json`.** Interactive reports contain the model, measures and the loaded
data of every table, so anyone who can open the report can read that data. Queries (connection
strings), data sheets' formulas, flows and publish targets (tokens) are removed. Publish static
reports when the underlying data must not leave your hands.

## Desktop and phone layouts

Every page has a phone layout, 360 points wide:

- If you arranged the page in the mobile layout editor (`page.mobile.items`), that layout is used.
- Otherwise VizCraft generates one: elements in reading order (top to bottom, left to right) in a
  single column; cards and KPIs two per row; a banner shape keeps the title and text that sit on
  it; decorative shapes are left out; charts get a phone-friendly height.

Viewers switch with the Desktop / Mobile toggle. On screens narrower than 640 px the phone layout
is chosen automatically. `?layout=mobile` / `?layout=desktop` in the URL forces one; `?embed=1`
hides the header and footer (used by the service's phone frame); `#page=<id>` opens a page.

Untick "Include the phone layout" to publish desktop only.

## Targets

Open **Publish** (ribbon: Publish tab, or Home → Publish). Each target remembers its settings in
the project (`publish.targets`). Tokens are stored only when you tick "Remember the token in the
project file" — they're saved in plain text, so leave it off for projects you share.
**Preview in browser** opens the report from a temporary file without uploading anything.

### VizCraft Service

A self-hosted portal like the Power BI service: workspaces, report cards with thumbnails, the
report view with an "Open in phone frame" mode, and an HTTP API.

In the dialog: the service URL, a workspace, the report name and the service's token. **Test
connection** checks the URL and whether the token can publish. Publishing again with the same
workspace and name replaces the report.

From the command line (CI, scheduled refreshes):

```sh
vizcraft-server publish sales.vizc --to https://reports.example.com --workspace Sales --token "$VIZCRAFT_TOKEN"
```

`publish` opens the project, refreshes every query (files, web, databases) and uploads the
bundle. Add `--static` for a static report, `--no-mobile` to skip the phone layout, `--name` to
rename it. `--sample` instead of a file publishes the built-in retail sample.

### Web folder

Writes the site into a folder. Upload it to any static host (Nginx, Apache, S3, Netlify,
Cloudflare Pages, an intranet share behind a web server). Browsers don't let pages read files from
a folder opened directly (`file://`), so serve it over HTTP — to try it locally:

```sh
cd my-report && python3 -m http.server 8000   # then open http://localhost:8000
```

Command line: `vizcraft-server export sales.vizc --out site/`.

### Single HTML file

Everything — viewer, data, engine — in one `.html` file. Email it, attach it, drop it in a chat
or a document library: it opens in any modern browser, offline, from disk. (Interactive files
carry the ~2 MB engine.) Command line: `vizcraft-server export sales.vizc --single --out report.html`.

### GitHub Pages

Free hosting with HTTPS on `https://<owner>.github.io/<repo>/`, or your own domain.

1. Create a repository on GitHub (public, or private on a plan that allows Pages).
2. Create a personal access token: GitHub → Settings → Developer settings → Fine-grained tokens,
   access to that repository with **Contents: Read and write** and **Pages: Read and write**.
3. In the dialog: repository `owner/repo`, branch (default `gh-pages`), an optional subfolder
   (several reports can share a repository: `reports/sales`, `reports/finance`), the token.
4. **Push to GitHub.** VizCraft runs `git` (it must be installed): it fetches the branch (or
   creates it), replaces the report in the subfolder, adds `.nojekyll`, commits and pushes over
   HTTPS with the token, then asks the GitHub API to serve Pages from the branch. The first build
   takes a minute or two.

If the token lacks the Pages permission, turn Pages on once by hand: repository → Settings →
Pages → Source: Deploy from a branch → `gh-pages` / root.

**Custom domain.** Enter it in the dialog (e.g. `reports.example.com`); VizCraft writes the
`CNAME` file. Then at your DNS provider:

- a subdomain (`reports.example.com`): a **CNAME** record `reports` → `<owner>.github.io`;
- an apex domain (`example.com`): **A** records to `185.199.108.153`, `185.199.109.153`,
  `185.199.110.153`, `185.199.111.153` (or ALIAS/ANAME to `<owner>.github.io`).

When DNS has propagated, tick **Enforce HTTPS** in Settings → Pages. The dialog shows the final URL
and these instructions after publishing.

### SVG image of a page

Publish → Export → Page as SVG saves the current page as a crisp vector image (for slides and
documents). Command line: `vizcraft-server render sales.vizc --page 2 --svg page2.svg [--mobile]`.

## Self-hosting the VizCraft Service

```sh
vizcraft-server serve --port 8080 --data /var/lib/vizcraft --token "$(openssl rand -hex 24)" --name "Acme Reports"
```

| Option | Default | |
|---|---|---|
| `--port` | 8080 | |
| `--host` | 0.0.0.0 | bind address (`127.0.0.1` behind a reverse proxy) |
| `--data` | `./vizcraft-data` | reports live in `<data>/<workspace>/<report>/` (`meta.json` + `files/`) |
| `--token` | `$VIZCRAFT_TOKEN` | needed to publish and delete. Without one anyone who can reach the port can publish: only for local use. |
| `--name` | VizCraft Service | shown in the portal |

Viewing reports needs no token. Put the service behind a reverse proxy for HTTPS (and for login,
if reports must be private: e.g. Nginx `auth_basic`, oauth2-proxy, Cloudflare Access).

**HTTP API**

| Request | |
|---|---|
| `GET /api/ping` | `{name, version, reports, canPublish}` (send the token to check it) |
| `GET /api/reports` | every report's metadata |
| `POST /api/publish?workspace=W&name=N` | body: the bundle as a zip (`Content-Type: application/zip`) or JSON `{"files": {"index.html": "<base64>", …}}`; header `Authorization: Bearer <token>`; returns `{url}` |
| `DELETE /api/reports/<workspace>/<report>` | with the token |
| `GET /r/<workspace>/<report>` | the report in the portal (`?view=phone` for the phone frame) |
| `GET /r/<workspace>/<report>/files/` | the bundle itself (embed it in an `<iframe>`) |

Publish with curl: `cd site && zip -r ../report.zip . && curl -H "Authorization: Bearer $T" --data-binary @../report.zip -H 'Content-Type: application/zip' "http://host:8080/api/publish?workspace=Sales&name=Q3"`.

### systemd

```ini
# /etc/systemd/system/vizcraft.service
[Unit]
Description=VizCraft Service
After=network-online.target

[Service]
User=vizcraft
EnvironmentFile=/etc/vizcraft.env          # VIZCRAFT_TOKEN=...
ExecStart=/usr/local/bin/vizcraft-server serve --host 127.0.0.1 --port 8080 --data /var/lib/vizcraft --name "Acme Reports"
Restart=on-failure
StateDirectory=vizcraft
NoNewPrivileges=yes
ProtectSystem=strict
ReadWritePaths=/var/lib/vizcraft

[Install]
WantedBy=multi-user.target
```

`sudo useradd --system vizcraft && sudo systemctl enable --now vizcraft`

### Docker

```dockerfile
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN rustup target add wasm32-unknown-unknown && packaging/build-wasm.sh \
 && cargo build --release -p vizcraft-server

FROM debian:bookworm-slim
COPY --from=build /src/target/release/vizcraft-server /usr/local/bin/
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["vizcraft-server", "serve", "--port", "8080", "--data", "/data"]
```

```sh
docker build -t vizcraft-server -f Dockerfile vizcraft/
docker run -d -p 8080:8080 -v vizcraft-data:/data -e VIZCRAFT_TOKEN=change-me vizcraft-server
```

(`vizcraft-server publish` needs the full image with connectors; the `serve` image only stores and
serves bundles.)

## Installing VizCraft

Release packages are built by `.github/workflows/vizcraft-release.yml` on every push to `main` /
`claude/**` (download them from the run's artifacts) and attached to a GitHub release for tags
`vizcraft-v*`.

**macOS** — `VizCraft-<version>-macos-universal.dmg` (Apple silicon and Intel). Open it and drag
VizCraft to Applications. The app is ad-hoc signed, not notarized, so the first launch is blocked
by Gatekeeper: right-click VizCraft.app → Open → Open (once), or
`xattr -dr com.apple.quarantine /Applications/VizCraft.app`. The service binary is inside the
app: `/Applications/VizCraft.app/Contents/MacOS/vizcraft-server`.

To build the DMG yourself on a Mac:

```sh
git clone https://github.com/digital-maven/all && cd all/vizcraft
packaging/macos/make-dmg.sh                    # host architecture
packaging/macos/make-dmg.sh --arch universal   # needs both rustup targets (it adds them)
open dist/
```

With `MACOS_SIGN_IDENTITY` set to a Developer ID identity the app is signed with the hardened
runtime instead of ad-hoc.

**Linux** — `packaging/linux/package.sh` builds `dist/VizCraft-<version>-linux-<arch>.tar.gz`
(bin/, a `.desktop` file and icons) and, when `appimagetool` is installed, an AppImage
(`./VizCraft-….AppImage`, or `… server serve` for the service).

**Windows** — `pwsh packaging/windows/package.ps1` builds a zip with `vizcraft.exe` and
`vizcraft-server.exe` (and an MSI when the WiX Toolset v3 is on PATH). SmartScreen may warn about
an unsigned app: More info → Run anyway.
