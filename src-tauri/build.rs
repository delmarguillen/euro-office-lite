// Commands the webview may invoke. Each one also needs its allow-* permission
// in a capability.
const APP_COMMANDS: &[&str] = &[
    "open_file",
    "save_file",
    "save_file_as",
    "write_editor_bin",
    "print_document",
    "create_new",
    "get_current_path",
    "pick_image_files",
    "take_pending_open_file",
    "open_pdf_viewer",
    "print_to_printer",
    "convert_for_insert",
    "get_system_fonts",
    "set_window_title",
    "set_document_modified",
    "js_log",
    "force_close",
    "remove_note_separator",
    "read_clipboard_image",
    "read_clipboard_text",
    "read_clipboard_html",
    "recent_files_state",
    "set_recent_files_enabled",
    "clear_recent_files",
    "recovery_begin",
    "save_changes",
    "recovery_mark_saved",
    "recovery_end",
    "recovery_candidates",
    "recovery_load",
    "recovery_discard",
    "get_save_suggestion",
    "list_user_dictionaries",
];

fn main() {
    // tauri_build tracks tauri.conf.json, capabilities and the bundled
    // resources, but not frontendDist. The frontend is embedded into the binary
    // by generate_context! at compile time, so without these lines cargo has no
    // dependency edge to the wrapper's own JS: editing it does not mark the
    // crate dirty, and `tauri dev` keeps serving whatever was embedded the last
    // time something else forced a rebuild.
    //
    // frontendDist is ../src-dist: a junction to ../src in dev on Windows, and a
    // copy made by the staging scripts in a regular build, which is what gets
    // embedded. Tracking the files there covers both. Only the wrapper's own
    // files are listed; the directory also holds the sdkjs and web-apps trees,
    // and walking those would add tens of thousands of paths to every build's
    // dependency check.
    for file in [
        "index.html",
        "bridge.js",
        "button-hint-patch.js",
        "font-patches.js",
        "editor-patches.js",
    ] {
        println!("cargo:rerun-if-changed=../src-dist/{file}");
    }

    let manifest = tauri_build::AppManifest::new().commands(APP_COMMANDS);
    let attributes = tauri_build::Attributes::new().app_manifest(manifest);
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
