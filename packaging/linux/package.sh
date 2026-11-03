#!/usr/bin/env bash
# Builds and packages VizCraft for Linux:
#
#   dist/VizCraft-<version>-linux-<arch>.tar.gz     bin/, share/applications, share/icons
#   dist/VizCraft-<version>-linux-<arch>.AppImage   when appimagetool is on PATH (or APPIMAGETOOL)
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
VERSION="${VIZCRAFT_VERSION:-$VERSION}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
WORK="$TARGET_DIR/linux-package"
DIST="$ROOT/dist"
NAME="VizCraft-$VERSION-linux-$ARCH"
STAGE="$WORK/$NAME"

echo "==> VizCraft $VERSION for Linux $ARCH"
if [[ "$SKIP_BUILD" = 0 ]]; then
  "$ROOT/packaging/build-wasm.sh"
  cargo build --release -p vizcraft -p vizcraft-server
fi

rm -rf "$WORK"
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$DIST"
install -m755 "$TARGET_DIR/release/vizcraft" "$STAGE/bin/vizcraft"
install -m755 "$TARGET_DIR/release/vizcraft-server" "$STAGE/bin/vizcraft-server"
strip "$STAGE/bin/vizcraft" "$STAGE/bin/vizcraft-server" 2>/dev/null || true
install -m644 "$HERE/vizcraft.desktop" "$STAGE/share/applications/com.vizcraft.app.desktop"
for s in 32 64 128 256 512; do
  mkdir -p "$STAGE/share/icons/hicolor/${s}x${s}/apps"
  python3 "$ROOT/packaging/icon/make_icon.py" "$s" "$STAGE/share/icons/hicolor/${s}x${s}/apps/com.vizcraft.app.png" >/dev/null
done
cat >"$STAGE/README.txt" <<EOF
VizCraft $VERSION for Linux ($ARCH)

Run:      ./bin/vizcraft
Install:  copy bin/ to ~/.local/bin and share/ to ~/.local/share (or /usr/local)
Service:  ./bin/vizcraft-server serve --port 8080 --token SECRET
Docs:     docs/publishing.md in the source repository
EOF
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$STAGE/share/applications/com.vizcraft.app.desktop" || true
fi

tar -C "$WORK" -czf "$DIST/$NAME.tar.gz" "$NAME"
echo "==> $DIST/$NAME.tar.gz"

# ---- AppImage (optional) -------------------------------------------------------------------------
APPIMAGETOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
if [[ -n "$APPIMAGETOOL" ]]; then
  APPDIR="$WORK/VizCraft.AppDir"
  mkdir -p "$APPDIR/usr"
  cp -R "$STAGE/bin" "$STAGE/share" "$APPDIR/usr/"
  cp "$STAGE/share/applications/com.vizcraft.app.desktop" "$APPDIR/com.vizcraft.app.desktop"
  cp "$STAGE/share/icons/hicolor/256x256/apps/com.vizcraft.app.png" "$APPDIR/com.vizcraft.app.png"
  cat >"$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
if [ "$1" = "server" ]; then shift; exec "$HERE/usr/bin/vizcraft-server" "$@"; fi
exec "$HERE/usr/bin/vizcraft" "$@"
EOF
  chmod +x "$APPDIR/AppRun"
  ARCH="$ARCH" "$APPIMAGETOOL" --no-appstream "$APPDIR" "$DIST/$NAME.AppImage"
  echo "==> $DIST/$NAME.AppImage"
else
  echo "(appimagetool not found: skipping the AppImage)"
fi
ls -lh "$DIST"
