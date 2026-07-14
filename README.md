# Euro-Office Lite

Lightweight desktop office suite built on Tauri v2 and Euro-Office editors. Installers: ~99 MB (Windows), ~129-135 MB (macOS), ~135 MB (Linux). No cloud, no telemetry.

Supports Word, Excel, and PowerPoint documents with native file operations and direct printing. Available for Windows (x64, ARM64), macOS (Apple Silicon and Intel, signed and notarized), and Linux (x64 .deb and Flatpak).

> **Alpha**: This project is in early development. Core features work (create, open, edit, save), but expect rough edges. Printing works on Windows; PDF export works on Windows and Linux (Word documents).

<p align="center">
  <img src="assets/demo.gif" alt="Euro-Office Lite demo" width="800">
  <br><br>
  <a href="https://github.com/delmarguillen/euro-office-lite/releases"><strong>Download the latest release</strong></a>
</p>

## Known issues

Euro-Office Lite is alpha. A few things you will run into early:

- **Spell check does not work yet** (#6).
- **No crash recovery.** Closing the window prompts you to save unsaved changes, but if the app crashes or is force-quit mid-edit, that unsaved work is lost. Save often.
- **Copy and Cut from the right-click menu do nothing on macOS and Linux.** The keyboard shortcuts (Cmd/Ctrl+C and Cmd/Ctrl+X) work on every platform (#17).
- **Some Linux setups show rendering glitches** such as rough text or a brief gray flash after Ctrl+C in Documents and Slides. The font list and editing itself are not affected, and this is under investigation (#27).

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

Pushing a tag `v*` triggers the GitHub Actions workflow which builds Windows (x64, ARM64), macOS (Apple Silicon and Intel, signed and notarized), and Linux (x64) installers and creates a GitHub Release. Tags containing `alpha`, `beta`, or `rc` are marked as pre-release.

## Installing on macOS

DMG builds are signed and notarized by Apple: download, open the DMG, drag to Applications and double-click. No Gatekeeper workarounds needed. Requires macOS 12 (Monterey) or later. Since v0.16.0-alpha, DMGs are available for both Apple Silicon (`aarch64`) and Intel (`x86_64`) Macs; download the one matching your machine.

## Installing on Linux

Install the `.deb` with `apt` so dependencies are resolved automatically:

```bash
sudo apt install ./Euro-Office-Lite_<version>_amd64.deb
```

Do **not** use `dpkg -i` directly, since it will not install the required `libwebkit2gtk-4.1-0` dependency.

For Fedora, Arch, immutable distros, or any Linux with Flatpak:

```bash
# Add repository (one-time)
flatpak remote-add --user --if-not-exists --no-gpg-verify euro-office \
  https://delmarguillen.github.io/euro-office-lite/repo

# Install
flatpak install --user euro-office org.eurooffice.Lite

# Update (future releases)
flatpak update org.eurooffice.Lite
```

Or download the `.flatpak` bundle from the release assets.

## Log files

- **Windows**: `%TEMP%\euro-office-lite\js-debug.log`
- **macOS / Linux**: `/tmp/euro-office-lite/js-debug.log`

## Credits and attribution

Euro-Office Lite is a lightweight desktop shell around the document editors of the [Euro-Office](https://github.com/Euro-Office) project, an independent community fork of the editors originally developed by ONLYOFFICE (Ascensio System SIA). The editing engine ([sdkjs](https://github.com/Euro-Office/sdkjs)) and UI ([web-apps](https://github.com/Euro-Office/web-apps)) are included as unmodified submodules under AGPL-3.0. The x2t document converter binaries are currently taken unmodified from [ONLYOFFICE Desktop Editors](https://github.com/ONLYOFFICE/DesktopEditors) releases; their corresponding source is available at [ONLYOFFICE/core](https://github.com/ONLYOFFICE/core) under the matching version tags (AGPL-3.0). All credit for the editing engine belongs to their respective developers.

What this project adds on top: the Tauri v2 native shell and desktop bridge, macOS support with signed and notarized builds, packaging for Windows (NSIS), Linux (.deb and Flatpak with a self-hosted repo), fully offline operation with no cloud, no accounts and no telemetry, and native file operations, printing and PDF export.

## License

[AGPL-3.0](LICENSE)
