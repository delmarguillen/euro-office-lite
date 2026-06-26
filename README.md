# Euro-Office Lite

Lightweight desktop office suite built on Tauri v2 and Euro-Office editors. ~96 MB installer, no cloud, no telemetry.

Supports Word, Excel, and PowerPoint documents with native file operations and direct printing. Available for Windows (x64, ARM64), macOS (Apple Silicon), and Linux (x64 .deb).

> **Alpha**: This project is in early development. Core features work (create, open, edit, save), but expect rough edges. Printing works on Windows; PDF export works on Windows and Linux (Word documents).

<p align="center">
  <img src="assets/demo.gif" alt="Euro-Office Lite demo" width="800">
  <br><br>
  <a href="https://github.com/delmarguillen/euro-office-lite/releases"><strong>Download the latest release</strong></a>
</p>

## Requirements

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) 1.77+
- Visual Studio 2022 with C++ desktop workload (for native compilation)

## Getting started

```powershell
git clone --recursive https://github.com/delmarguillen/euro-office-lite.git
cd euro-office-lite
npm install
.\scripts\setup.ps1       # generates sdkjs develop scripts and fonts
.\scripts\get-x2t.ps1     # downloads x2t converter binaries (requires admin)
npx tauri dev
```

`setup.ps1` runs `grunt develop` inside sdkjs to generate the JS module loaders (`scripts.js`) that each editor needs at runtime. Run it again after updating the sdkjs submodule.

`get-x2t.ps1` downloads the x2t converter binaries (~60 MB) from the repo's `dependencies` release into `src-tauri/binaries/`. Requires [GitHub CLI](https://cli.github.com/) (`gh`) authenticated.

## Build installer

```powershell
.\scripts\prepare-dist.ps1    # stages slim frontend into src-dist/
npx tauri build
```

Output goes to `src-tauri/target/release/bundle/nsis/`.

## Architecture

```
User <-> Tauri WebView2 <-> sdkjs/web-apps (editor UI)
                        <-> bridge.js (desktop bridge shim)
                        <-> Rust backend <-> x2t (format conversion)
                                         <-> tauri-plugin-printer-v2 (native printing)
```

- **sdkjs / web-apps**: Editor frontend (submodules, do not modify)
- **bridge.js**: Implements the `AscDesktopEditor` API that sdkjs expects from a desktop host
- **x2t**: Converts between DOCX/XLSX/PPTX and Editor.bin (sdkjs internal format)
- **Printing**: Generates PDF via x2t + DoctRenderer. On Windows, sends to printer via bundled SumatraPDF. On Linux, opens PDF with the system viewer via xdg-open

## CI/CD

Pushing a tag `v*` triggers the GitHub Actions workflow which builds Windows (x64, ARM64), macOS (Apple Silicon), and Linux (x64) installers and creates a GitHub Release. Tags containing `alpha`, `beta`, or `rc` are marked as pre-release.

## Installing on Linux

Install the `.deb` with `apt` so dependencies are resolved automatically:

```bash
sudo apt install ./Euro-Office-Lite_0.1.0-alpha_amd64.deb
```

Do **not** use `dpkg -i` directly — it will not install the required `libwebkit2gtk-4.1-0` dependency.

## Log files

- **Windows**: `%TEMP%\euro-office-lite\js-debug.log`
- **macOS / Linux**: `/tmp/euro-office-lite/js-debug.log`

## License

[AGPL-3.0](LICENSE)
