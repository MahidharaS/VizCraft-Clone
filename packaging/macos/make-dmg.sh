#!/usr/bin/env bash
# Builds VizCraft.app and a drag-to-Applications DMG:
#
#   dist/VizCraft-<version>-macos-<arch>.dmg
#
# Usage: packaging/macos/make-dmg.sh [--arch host|universal|aarch64|x86_64] [--skip-build]
#
#   host       (default) the Mac's own architecture; "universal" when both Rust targets
#              (aarch64-apple-darwin, x86_64-apple-darwin) are installed and --arch universal
#   --skip-build  reuse the release binaries already in target/
#
# Signing: ad-hoc (`codesign -s -`) unless MACOS_SIGN_IDENTITY names a Developer ID identity.
# An ad-hoc signed app downloaded from the internet is quarantined by Gatekeeper: right-click
# VizCraft.app → Open (once), or run `xattr -dr com.apple.quarantine /Applications/VizCraft.app`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/packaging/macos"
cd "$ROOT"

ARCH=host
SKIP_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "make-dmg.sh runs on macOS (it needs iconutil, codesign and hdiutil)." >&2
  exit 1
fi

if [[ "$ARCH" = host ]]; then
  case "$(uname -m)" in
    arm64) ARCH=aarch64 ;;
    *) ARCH=x86_64 ;;
  esac
fi
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
VERSION="${VIZCRAFT_VERSION:-$VERSION}"
BUILD_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
export MACOSX_DEPLOYMENT_TARGET=11.0
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
WORK="$TARGET_DIR/macos-package"
DIST="$ROOT/dist"
APP="$WORK/VizCraft.app"
DMG="$DIST/VizCraft-$VERSION-macos-$ARCH.dmg"

echo "==> VizCraft $VERSION for macOS ($ARCH), signing identity: $IDENTITY"

# ---- build ---------------------------------------------------------------------------------------
if [[ "$SKIP_BUILD" = 0 ]]; then
  # The web viewer engine first: vizcraft-publish embeds it, so published reports are interactive.
  "$ROOT/packaging/build-wasm.sh"
  for t in "${TARGETS[@]}"; do rustup target add "$t" >/dev/null 2>&1 || true; done
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  cargo build --release -p vizcraft -p vizcraft-server "${args[@]}"
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin" "$DIST"
for bin in vizcraft vizcraft-server; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$TARGET_DIR/$t/release/$bin"); done
  if [[ ${#inputs[@]} -gt 1 ]]; then
    lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  else
    cp "${inputs[0]}" "$WORK/bin/$bin"
  fi
  lipo -info "$WORK/bin/$bin" || true
done

# ---- icon ----------------------------------------------------------------------------------------
echo "==> icon"
ICONSET="$WORK/VizCraft.iconset"
mkdir -p "$ICONSET"
python3 "$ROOT/packaging/icon/make_icon.py" 1024 "$WORK/icon_1024.png"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$WORK/icon_1024.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" "$WORK/icon_1024.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$WORK/VizCraft.icns"

# ---- VizCraft.app ------------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$WORK/bin/vizcraft" "$APP/Contents/MacOS/VizCraft"
# The service / command-line publisher ships inside the app:
#   /Applications/VizCraft.app/Contents/MacOS/vizcraft-server serve --port 8080
cp "$WORK/bin/vizcraft-server" "$APP/Contents/MacOS/vizcraft-server"
cp "$WORK/VizCraft.icns" "$APP/Contents/Resources/VizCraft.icns"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@BUILD_SHA@/$BUILD_SHA/g" "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

echo "==> signing"
sign_args=(--force --sign "$IDENTITY")
if [[ "$IDENTITY" != "-" ]]; then
  sign_args+=(--options runtime --timestamp)
fi
codesign "${sign_args[@]}" "$APP/Contents/MacOS/vizcraft-server"
codesign "${sign_args[@]}" --deep "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

# ---- DMG -------------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/VizCraft.app"
ln -s /Applications "$STAGE/Applications"
cp "$HERE/README-dmg.txt" "$STAGE/Read me.txt"
rm -f "$DMG"
# `hdiutil create -srcfolder` needs to attach a device and is flaky on CI runners ("Resource
# busy"); retry it, then fall back to makehybrid + convert.
ok=0
for attempt in 1 2 3; do
  if hdiutil create -volname "VizCraft" -srcfolder "$STAGE" -ov -fs HFS+ -format UDZO -imagekey zlib-level=9 "$DMG"; then
    ok=1
    break
  fi
  echo "hdiutil create failed (attempt $attempt); retrying" >&2
  sleep $((attempt * 5))
done
if [[ "$ok" = 0 ]]; then
  rm -f "$WORK/raw.dmg"
  hdiutil makehybrid -hfs -hfs-volume-name "VizCraft" -hfs-openfolder "$STAGE" -o "$WORK/raw.dmg" "$STAGE"
  hdiutil convert "$WORK/raw.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
  rm -f "$WORK/raw.dmg"
fi
codesign --force --sign "$IDENTITY" "$DMG"

echo "==> done"
ls -lh "$DMG"
