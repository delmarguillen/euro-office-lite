fn main() {
    // tauri_build tracks tauri.conf.json, capabilities and the bundled
    // resources, but not frontendDist. The frontend is embedded into the binary
    // by generate_context! at compile time, so without these lines cargo has no
    // dependency edge to the wrapper's own JS: editing it does not mark the
    // crate dirty, and `tauri dev` keeps serving whatever was embedded the last
    // time something else forced a rebuild.
    //
    // Only the wrapper's own files are listed. frontendDist points at ../src,
    // which also contains the sdkjs and web-apps submodules; walking those would
    // add tens of thousands of paths to every build's dependency check.
    for file in [
        "index.html",
        "bridge.js",
        "button-hint-patch.js",
        "font-patches.js",
        "editor-patches.js",
    ] {
        println!("cargo:rerun-if-changed=../src/{file}");
    }

    tauri_build::build()
}
