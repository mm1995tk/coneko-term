#!/bin/sh
# Rust を WebAssembly にして、wasm-bindgen の出力を src/wasm に置く（nix develop の中で回す）。
set -eu
cd "$(dirname "$0")/.."
cargo build --locked --release --target wasm32-unknown-unknown -p coneko-term-wasm
rm -rf src/wasm
wasm-bindgen --target web --out-dir src/wasm \
  target/wasm32-unknown-unknown/release/coneko_term_wasm.wasm
