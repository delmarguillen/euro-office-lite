# Euro-Office Lite printer fork

Base: the published `tauri-plugin-printer-v2` **0.2.4** crate from <https://crates.io/crates/tauri-plugin-printer-v2/0.2.4> (MIT, chen-collab). Crate SHA-256: `0d7390d2a37bfe17331b2f5f775f204f304657804819a6b96579fd567d5712be`. Cargo selects this copy through the application's `[patch.crates-io]` entry. The Windows implementation and its bundled SumatraPDF executable are retained from that crate. The executable is required for the existing Windows behavior.

Linux additions are intentionally limited to what Euro-Office Lite needs:

- `plugin:printer|get_printers` and `PrinterExt::get_printers()` return a JSON string containing CUPS destination names, the default destination, and the advertised color/duplex capabilities. Named CUPS instances are included.
- `linux::print_pdf(printer, path)` is a Rust-only API that submits a PDF to a freshly enumerated destination and returns its CUPS job ID. It preserves the destination's configured options except for `print-scaling`, which is set to `none` so the print keeps its intended dimensions and margins. It never deletes the source file or changes the destination's saved defaults.
- Discovery invoked from JavaScript runs off the webview thread. The app also calls PDF submission on a blocking worker.
- The missing upstream `UnsupportedPlatform` error variant is defined so the plugin compiles on Linux.

No Linux implementation is provided for HTML/URL printing, job management, or the plugin's unrestricted webview PDF commands. Euro-Office Lite exposes only printer discovery to the webview; printing goes through its existing backend
command, which validates the app-generated PDF. Copies, page ranges, and other native print options are not newly implemented; the app currently passes only the selected printer to that command. CUPS destination defaults apply except for the explicit `print-scaling=none` job option.

Build dependency: CUPS 2 development files (`libcups2-dev` on Debian/Ubuntu, `libcups` on Arch). Runtime: the CUPS client library and an accessible configured CUPS scheduler; the app does not install or configure printers. Flatpak needs `--socket=cups` and libcups in its runtime. The standard GNOME runtime supplies the library.

Run `bash scripts/test-printer-linux.sh` from the repository root for backend tests that do not submit real print jobs. The bridge tests cover discovery, selection, fallback, and submission errors separately.

Native and Flatpak version was tested on Arch Linux using CUPS 2.4.20 and a Canon MF655Cdw printer.