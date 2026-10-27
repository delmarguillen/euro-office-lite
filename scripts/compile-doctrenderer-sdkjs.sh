#!/usr/bin/env bash
# Compile the sdkjs bundles required by x2t/DoctRenderer for all editor types.
# The editor UI keeps using the repository submodule revision; this script
# temporarily checks out the sdkjs tag matching x2t and restores it on exit.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SDKJS="$PROJECT_ROOT/src/sdkjs"
BINARIES="$PROJECT_ROOT/src-tauri/binaries"
TAG="${DOCTRENDERER_SDKJS_TAG:-v9.4.0.129}"
# The Euro-Office sdkjs fork only carries tags up to 8.2; the tags that
# match each platform's x2t (9.4 and 10.0) exist only upstream.
REMOTE="${DOCTRENDERER_SDKJS_REMOTE:-https://github.com/ONLYOFFICE/sdkjs.git}"
STAGING="${DOCTRENDERER_STAGING:-${RUNNER_TEMP:-/tmp}/doctrenderer-sdkjs}"
ALLFONTS_SOURCE="${ALLFONTS_SOURCE:-$SDKJS/common/AllFonts.js}"
UI_REF="$(git -C "$SDKJS" rev-parse HEAD)"
PATCH_RECORD="$(mktemp)"

# The tag's patches come out before the checkout back to the UI revision,
# which would refuse to overwrite them.
restore_sdkjs() {
    node "$SCRIPT_DIR/apply-patches.mjs" revert --record "$PATCH_RECORD"
    git -C "$SDKJS" checkout --detach "$UI_REF" >/dev/null
}
trap restore_sdkjs EXIT

rm -rf "$STAGING"
mkdir -p "$STAGING"

echo "Compiling DoctRenderer sdkjs tag $TAG"
git -C "$SDKJS" fetch "$REMOTE" tag "$TAG" --no-tags
git -C "$SDKJS" checkout --detach "$TAG"

# A tag in a repository we do not control can be moved; the commit cannot.
[ -z "${CI:-}" ] || [ -n "${DOCTRENDERER_SDKJS_COMMIT:-}" ] || { echo "ERROR: DOCTRENDERER_SDKJS_COMMIT is required in CI"; exit 1; }
if [ -n "${DOCTRENDERER_SDKJS_COMMIT:-}" ]; then
    RESOLVED="$(git -C "$SDKJS" rev-parse "$TAG^{commit}")"
    if [ "$RESOLVED" != "$DOCTRENDERER_SDKJS_COMMIT" ]; then
        echo "ERROR: sdkjs tag $TAG resolves to $RESOLVED, expected $DOCTRENDERER_SDKJS_COMMIT"
        exit 1
    fi
    echo "sdkjs tag $TAG verified at commit $RESOLVED"
fi

# Upstream sdkjs dropped the grunt harness after 8.2 (it builds via build.py);
# the Euro-Office fork still carries it. Reuse the fork's build/ tooling
# against the tagged sources - the configs/*.json format is unchanged.
if [ ! -f "$SDKJS/build/Gruntfile.js" ]; then
    echo "Restoring grunt build harness from $UI_REF"
    git -C "$SDKJS" checkout "$UI_REF" -- build
fi

# patches/doctrenderer-sdkjs/ is written against this tag, not the UI revision.
node "$SCRIPT_DIR/apply-patches.mjs" apply --only doctrenderer-sdkjs --record "$PATCH_RECORD"

(
    cd "$SDKJS/build"
    # Upstream sdkjs tags ship no lockfile; the Euro-Office fork does.
    if [ -f package-lock.json ] || [ -f npm-shrinkwrap.json ]; then
        npm ci
    else
        npm install --no-audit --no-fund
    fi
    for module in word cell slide; do
        echo "Compiling $module SDK"
        npx grunt "compile-$module" --desktop=true --level=SIMPLE --no-color
    done
)

for module in word cell slide; do
    mkdir -p "$STAGING/$module"
    for bundle in sdk-all-min.js sdk-all.js; do
        source="$SDKJS/deploy/sdkjs/$module/$bundle"
        if [ ! -s "$source" ]; then
            echo "ERROR: Missing compiled DoctRenderer bundle: $source"
            exit 1
        fi
        cp "$source" "$STAGING/$module/$bundle"
        echo "$module/$bundle: $(wc -c < "$source") bytes"
    done
done

mkdir -p "$STAGING/common/Native"
mkdir -p "$STAGING/common/libfont/engine"
cp "$SDKJS/common/Native/native.js" "$STAGING/common/Native/"
cp "$SDKJS/common/Native/jquery_native.js" "$STAGING/common/Native/"
cp "$ALLFONTS_SOURCE" "$STAGING/common/AllFonts.js"
cp "$SDKJS/common/libfont/engine/fonts_native.js" "$STAGING/common/libfont/engine/"

# Preserve the legacy Word copies in binaries/ while all runtime resources
# migrate to the module-specific editors/sdkjs layout.
cp "$STAGING/word/sdk-all-min.js" "$BINARIES/sdk-all-min.js"
cp "$STAGING/word/sdk-all.js" "$BINARIES/sdk-all.js"

restore_sdkjs
trap - EXIT

echo "DoctRenderer sdkjs staging complete: $STAGING"
