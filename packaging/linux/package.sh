#!/usr/bin/env bash
# Builds and packages VizardCraft for Linux:
#
#   dist/VizardCraft-<version>-linux-<arch>.tar.gz     bin/, share/applications, share/icons
#   dist/VizardCraft-<version>-linux-<arch>.AppImage   when appimagetool is on PATH (or APPIMAGETOOL)
#
# Usage: packaging/linux/package.sh [--skip-build]
# Needs: cargo, python3 (icon). Build on an old distro so the binaries run on newer ones.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/packaging/linux"
cd "$ROOT"

SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,9p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
VERSION="${VIZARDCRAFT_VERSION:-$VERSION}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
WORK="$TARGET_DIR/linux-package"
DIST="$ROOT/dist"
NAME="VizardCraft-$VERSION-linux-$ARCH"
STAGE="$WORK/$NAME"

echo "==> VizardCraft $VERSION for Linux $ARCH"
if [[ "$SKIP_BUILD" = 0 ]]; then
  "$ROOT/packaging/build-wasm.sh"
  cargo build --release -p vizardcraft -p vizardcraft-server -p vizardcraft-mlserve
fi

rm -rf "$WORK"
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$DIST"
install -m755 "$TARGET_DIR/release/vizardcraft" "$STAGE/bin/vizardcraft"
install -m755 "$TARGET_DIR/release/vizardcraft-server" "$STAGE/bin/vizardcraft-server"
install -m755 "$TARGET_DIR/release/vizardcraft-mlserve" "$STAGE/bin/vizardcraft-mlserve"
strip "$STAGE/bin/vizardcraft" "$STAGE/bin/vizardcraft-server" "$STAGE/bin/vizardcraft-mlserve" 2>/dev/null || true
install -m644 "$HERE/vizardcraft.desktop" "$STAGE/share/applications/com.vizardcraft.app.desktop"
for s in 32 64 128 256 512; do
  mkdir -p "$STAGE/share/icons/hicolor/${s}x${s}/apps"
  cp "$ROOT/packaging/icon/vizardcraft-icon-${s}.png" "$STAGE/share/icons/hicolor/${s}x${s}/apps/com.vizardcraft.app.png"
done
cat >"$STAGE/README.txt" <<EOF
VizardCraft $VERSION for Linux ($ARCH)

Run:      ./bin/vizardcraft
Install:  copy bin/ to ~/.local/bin and share/ to ~/.local/share (or /usr/local)
Service:  ./bin/vizardcraft-server serve --port 8080 --token SECRET
Docs:     docs/publishing.md in the source repository
EOF
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$STAGE/share/applications/com.vizardcraft.app.desktop" || true
fi

tar -C "$WORK" -czf "$DIST/$NAME.tar.gz" "$NAME"
echo "==> $DIST/$NAME.tar.gz"

# ---- AppImage (optional) -------------------------------------------------------------------------
APPIMAGETOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
if [[ -n "$APPIMAGETOOL" ]]; then
  APPDIR="$WORK/VizardCraft.AppDir"
  mkdir -p "$APPDIR/usr"
  cp -R "$STAGE/bin" "$STAGE/share" "$APPDIR/usr/"
  cp "$STAGE/share/applications/com.vizardcraft.app.desktop" "$APPDIR/com.vizardcraft.app.desktop"
  cp "$STAGE/share/icons/hicolor/256x256/apps/com.vizardcraft.app.png" "$APPDIR/com.vizardcraft.app.png"
  cat >"$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
if [ "$1" = "server" ]; then shift; exec "$HERE/usr/bin/vizardcraft-server" "$@"; fi
exec "$HERE/usr/bin/vizardcraft" "$@"
EOF
  chmod +x "$APPDIR/AppRun"
  ARCH="$ARCH" "$APPIMAGETOOL" --no-appstream "$APPDIR" "$DIST/$NAME.AppImage"
  echo "==> $DIST/$NAME.AppImage"
else
  echo "(appimagetool not found: skipping the AppImage)"
fi
ls -lh "$DIST"
