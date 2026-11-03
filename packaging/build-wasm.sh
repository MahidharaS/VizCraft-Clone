#!/usr/bin/env bash
# Builds the VizCraft web viewer engine (crates/wasm) for wasm32-unknown-unknown.
# Rebuild vizcraft / vizcraft-server afterwards: vizcraft-publish embeds the .wasm when it exists,
# which makes published reports interactive (slicers, cross-filtering, drill, parameters).
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v rustup >/dev/null 2>&1; then
  rustup target add wasm32-unknown-unknown >/dev/null
fi
cargo build -p vizcraft-wasm --release --target wasm32-unknown-unknown

out=target/wasm32-unknown-unknown/release/vizcraft_wasm.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals "$out" -o "$out.opt" && mv "$out.opt" "$out"
fi
size=$(wc -c <"$out" | tr -d ' ')
echo "Built $out ($((size / 1024)) KiB)"
