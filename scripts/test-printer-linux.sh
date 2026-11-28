#!/usr/bin/env bash
# Exercise the real CUPS backend with a fake scheduler. Requires Rust and
# libcups development files, but no webview, running CUPS server, or printer.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT
rustc --edition=2021 --test \
  "$ROOT/src-tauri/vendor/tauri-plugin-printer-v2/src/linux/cups.rs" \
  -o "$TEST_DIR/printer-tests"
"$TEST_DIR/printer-tests"
