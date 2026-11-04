#!/usr/bin/env bash
# Builds the VizardCraft web viewer engine (crates/wasm) for wasm32-unknown-unknown.
# Rebuild vizardcraft / vizardcraft-server afterwards: vizardcraft-publish embeds the .wasm when it exists,
# which makes published reports interactive (slicers, cross-filtering, drill, parameters).
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v rustup >/dev/null 2>&1; then
  rustup target add wasm32-unknown-unknown >/dev/null
fi
cargo build -p vizardcraft-wasm --release --target wasm32-unknown-unknown

out=target/wasm32-unknown-unknown/release/vizardcraft_wasm.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals "$out" -o "$out.opt" && mv "$out.opt" "$out"
fi
size=$(wc -c <"$out" | tr -d ' ')
echo "Built $out ($((size / 1024)) KiB)"
