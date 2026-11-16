#!/usr/bin/env bash
# Builds src/markdown/vendor/codemirror.min.js, the Markdown page's editor,
# from the versions pinned in tools/codemirror-vendor/package-lock.json.
# The bundle is committed; run this only to update it, then update the sizes
# and the sha256 in src/markdown/vendor/README.md.
#
# Run: scripts/vendor-codemirror.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
vendor="$root/tools/codemirror-vendor"
out="$root/src/markdown/vendor/codemirror.min.js"

cd "$vendor"
npm ci --no-audit --no-fund
npx --no-install esbuild entry.mjs \
  --bundle --minify --format=iife --target=es2020 \
  --legal-comments=none --charset=utf8 \
  --outfile="$out"

echo "bytes: $(wc -c < "$out")"
echo "gzip:  $(gzip -9 -c "$out" | wc -c)"
echo "sha256: $(sha256sum "$out" | cut -d' ' -f1)"
