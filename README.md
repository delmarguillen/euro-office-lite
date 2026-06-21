# Euro-Office Lite

Desktop office suite built on Tauri v2 (WebView2 on Windows) with sdkjs-based document editors.

Supports Word, Excel, and PowerPoint documents with native file operations and direct printing.

## Requirements

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) 1.77+
- Visual Studio 2022 with C++ desktop workload (for native compilation)
- x2t converter binaries in `src-tauri/binaries/` (see `scripts/get-x2t.ps1`)

## Development

```powershell
npm install
npx tauri dev        # Run from Developer PowerShell for VS
```

## Build installer

```powershell
npx tauri build
```

Output goes to `src-tauri/target/release/bundle/`.

## Architecture

```
User ↔ Tauri WebView2 ↔ sdkjs/web-apps (editor UI)
                       ↔ bridge.js (desktop bridge shim)
                       ↔ Rust backend ↔ x2t (format conversion)
                                      ↔ tauri-plugin-printer-v2 (native printing)
```

- **sdkjs / web-apps**: Editor frontend (submodules, do not modify)
- **bridge.js**: Implements the `AscDesktopEditor` API that sdkjs expects from a desktop host
- **x2t**: Converts between DOCX/XLSX/PPTX and Editor.bin (sdkjs internal format)
- **Printing**: Generates PDF via x2t, sends to printer via bundled SumatraPDF (embedded in plugin binary, no extra files needed)

## Log file

Runtime logs are written to `%TEMP%\euro-office-lite\js-debug.log`.
