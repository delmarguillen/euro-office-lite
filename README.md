# Euro-Office Lite

<p align="center">
  <img src="assets/demo.gif" alt="Euro-Office Lite demo" width="800">
  <br><br>
  <a href="https://github.com/delmarguillen/euro-office-lite/releases"><strong>Download the latest release</strong></a>
</p>

Lightweight desktop office suite built on Tauri v2 and Euro-Office editors. **No cloud, no telemetry.**

## What it opens

- **Documents, spreadsheets and presentations**: Word, Excel and PowerPoint, OpenDocument, RTF, CSV and plain text, with native file operations and printing.
- **Markdown** (.md): its own editor, with the text on one side and a live preview on the other. Create one from the start screen and export it as HTML.
- **PDF**: a PDF editor where you can add comments, highlights and free text and save them.
- **Older formats** (.doc, .xls, .ppt): they open and are saved as .docx, .xlsx or .pptx.
- **Office templates** (.dotx, .xltx, .potx): they open, but saving as a template is not supported yet.

## Platforms

| Platform | Package | Installer size |
|---|---|---|
| [Windows](#windows) | x64, ARM64 | ~86 MB |
| [macOS](#macos) | Apple Silicon and Intel, signed and notarized | ~119-126 MB |
| [Linux x64](#linux) | .deb | ~89 MB |
| [Linux ARM64](#linux) | .deb | ~84 MB |
| [Linux x64](#linux) | Flatpak | ~89 MB |

> **Alpha**: This project is in early development. Core features work (create, open, edit, save, PDF export), but expect rough edges. Printing sends the document to the selected printer on Windows and Linux. Without a selected printer, or on macOS, it opens the system PDF viewer.

## Installing

### Windows

Download the installer that matches your machine: `x64-setup.exe` for most PCs, `arm64-setup.exe` for Windows on ARM (for example a Surface Pro X or a Snapdragon laptop). Run it and follow the steps.

The installers are not signed yet. The first time you run one, Windows may show a SmartScreen warning: choose "More info" and then "Run anyway".

On Windows 11 with Smart App Control turned on, the installer is blocked and there is no option to run it anyway. You can turn it off in Windows Security > App & browser control > Smart App Control, install the app, and turn it back on.

### macOS

DMG builds are signed and notarized by Apple: download, open the DMG, drag to Applications and double-click. No Gatekeeper workarounds needed. Requires macOS 12 (Monterey) or later. Since v0.16.0-alpha, DMGs are available for both Apple Silicon (`aarch64`) and Intel (`x86_64`) Macs; download the one matching your machine.

### Linux

Install the `.deb` with `apt` so dependencies are resolved automatically. Use the `amd64` file on x64 machines and the `arm64` file on 64-bit ARM machines (for example a Raspberry Pi with a 64-bit OS):

```bash
sudo apt install ./Euro-Office-Lite_<version>_amd64.deb   # x64
sudo apt install ./Euro-Office-Lite_<version>_arm64.deb   # ARM64
```

The `.deb` needs glibc 2.34 or later: Debian 12 or later, Ubuntu 22.04 or later, or Raspberry Pi OS based on Debian 12 or later. Raspberry Pi OS based on Debian 11 is too old.

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

**Title bar on Wayland.** On Wayland the app draws its own GTK title bar. If your compositor adds a second one (for example KDE Plasma set to server-side decorations), or you use a tiling compositor, start the app with one of these options. `--system-title-bar` leaves the title bar to the compositor, so a compositor that draws its own bar shows only that one. `--no-title-bar` removes the title bar completely. It is meant for tiling compositors: the window has no minimize button, and you move it with the compositor's shortcuts. The environment variable `EURO_OFFICE_LITE_TITLE_BAR` does the same (`system` or `none`); an option on the command line wins over it. The options do nothing on X11, Windows or macOS.

```bash
# Flatpak, once:
flatpak run org.eurooffice.Lite --system-title-bar
# Flatpak, every launch:
flatpak override --user --env=EURO_OFFICE_LITE_TITLE_BAR=system org.eurooffice.Lite
```

With the `.deb`, copy the desktop file to `~/.local/share/applications/` and change its `Exec` line to `Exec=euro-office-lite --system-title-bar %F`.

**Slow scrolling or jagged text.** Since 0.18.3, the app turns off WebKitGTK's DMABUF renderer on Linux only when the proprietary NVIDIA driver is loaded, because with it text could look jagged ([#27](https://github.com/delmarguillen/euro-office-lite/issues/27)). On other GPUs it stays on, which made scrolling smoother on an Intel GPU ([#65](https://github.com/delmarguillen/euro-office-lite/issues/65)). To change it, use the "Hardware acceleration" option in Settings (Automatic, On, Off); it applies after a restart. If you start the app with `WEBKIT_DISABLE_DMABUF_RENDERER` set, that value wins over the setting.

## Known issues

Euro-Office Lite is alpha. A few things you will run into early:

- **Only English and Spanish are checked out of the box** (#6). Dictionaries for both ship with the app (every regional Spanish variant is checked). Text in other languages is left unchecked, not marked as wrong, and you can add any other language yourself: see [Adding more spellcheck languages](#adding-more-spellcheck-languages).
- **The Windows installers are not signed yet**, so Windows may warn you or block the installer: see [Installing on Windows](#windows).
- **Text files with emoji or other characters outside the Basic Multilingual Plane are refused** with a message instead of opening.

## Adding more spellcheck languages

The spell checker works with any language from the [Euro-Office dictionaries repository](https://github.com/Euro-Office/dictionaries). Each language there is a folder named after its locale, holding the two files hunspell needs. A language the repository names `xx_YY` goes in as `xx_YY/xx_YY.aff` and `xx_YY/xx_YY.dic`, inside your dictionaries folder:

- **Windows**: `%APPDATA%\org.euro-office.lite\dictionaries\`
- **macOS**: `~/Library/Application Support/org.euro-office.lite/dictionaries/`
- **Linux**: `~/.local/share/org.euro-office.lite/dictionaries/`

The easiest way is the "Add spell check language…" option in the settings gear on the start screen: choose the .aff and .dic files of a Hunspell dictionary (for example from LibreOffice's dictionaries) and the app copies them into the right folder. If that language is already installed, it asks before replacing it.

To copy the files by hand instead, create the folder if it is not there yet, restart the app, and the language shows up in the language list with a dictionary indicator, checked like any other. Keep the folder name exactly as the repository spells it, because that name is how the app matches the folder to the language set on the text. A folder missing either file is ignored, and the reason is written to the log file listed below.

Only English and Spanish come preinstalled for now; every other language is added this way. The app downloads nothing: you fetch the dictionary with your browser and add it yourself.

## Log files

Each window of the app writes its own log, `js-debug-<number>-<number>.log`. The newest one has the highest first number.

- **Windows**: `%TEMP%\euro-office-lite\`
- **macOS / Linux**: `/tmp/euro-office-lite-<user id>/` (`id -u` shows your user id)
- **Flatpak**: `~/.var/app/org.eurooffice.Lite/cache/org.euro-office.lite/tmp/`

## Building from source

You only need this to work on the app itself. To use it, download an installer from the [releases page](https://github.com/delmarguillen/euro-office-lite/releases).

### Requirements

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) 1.77+
- Visual Studio 2022 with C++ desktop workload (for native compilation)

### Run from source

```powershell
git clone --recursive https://github.com/delmarguillen/euro-office-lite.git
cd euro-office-lite
npm install
.\scripts\setup.ps1       # generates sdkjs develop scripts and fonts
.\scripts\get-x2t.ps1     # downloads x2t converter binaries (requires admin)
npx tauri dev
```

`setup.ps1` runs `grunt develop` inside sdkjs to generate the JS module loaders (`scripts.js`) that each editor needs at runtime. Run it again after updating the sdkjs submodule.

`get-x2t.ps1` downloads the x2t converter binaries (~47 MB) from the repo's `dependencies` release into `src-tauri/binaries/`. Requires [GitHub CLI](https://cli.github.com/) (`gh`) authenticated.

### Build installer

```powershell
.\scripts\prepare-dist.ps1    # stages slim frontend into src-dist/
npx tauri build
```

Output goes to `src-tauri/target/release/bundle/nsis/`.

### Architecture

```
User <-> Tauri webview <-> sdkjs/web-apps (editor UI)
                        <-> bridge.js (desktop bridge shim)
                        <-> Rust backend <-> x2t (format conversion)
                                         <-> tauri-plugin-printer-v2 (Windows printing)
                                         <-> CUPS (Linux printing)
```

- **sdkjs / web-apps**: Editor frontend (submodules, do not modify)
- **bridge.js**: Implements the `AscDesktopEditor` API that sdkjs expects from a desktop host
- **x2t**: Converts between DOCX/XLSX/PPTX and Editor.bin (sdkjs internal format)
- **Printing**: Generates PDF via x2t + DoctRenderer. On Windows, sends to printer via bundled SumatraPDF; on Linux, submits it through CUPS. On macOS, opens the PDF in the system viewer

### CI/CD

Pushing a tag `v*` triggers the GitHub Actions workflow which builds Windows (x64, ARM64), macOS (Apple Silicon and Intel, signed and notarized), and Linux (x64, ARM64) installers and creates a GitHub Release. Tags containing `alpha`, `beta`, or `rc` are marked as pre-release.

## Credits and attribution

Euro-Office Lite is a lightweight desktop shell around the document editors of the [Euro-Office](https://github.com/Euro-Office) project, an independent community fork of the editors originally developed by ONLYOFFICE (Ascensio System SIA). The editing engine ([sdkjs](https://github.com/Euro-Office/sdkjs)) and UI ([web-apps](https://github.com/Euro-Office/web-apps)) are included as submodules under AGPL-3.0, with a few small patches applied at build time (see `patches/`). The x2t document converter binaries are currently taken unmodified from [ONLYOFFICE Desktop Editors](https://github.com/ONLYOFFICE/DesktopEditors) releases, and the scripts x2t uses for PDF export are built from [ONLYOFFICE/sdkjs](https://github.com/ONLYOFFICE/sdkjs) at the tag that matches the converter; their corresponding source is available at [ONLYOFFICE/core](https://github.com/ONLYOFFICE/core) and ONLYOFFICE/sdkjs under the matching version tags (AGPL-3.0). On Windows, the converter also ships with the Microsoft Visual C++ runtime files it needs (msvcp140.dll, vcruntime140.dll and vcruntime140_1.dll), redistributed under the Microsoft Visual Studio license terms. All credit for the editing engine belongs to their respective developers.

What this project adds on top: the Tauri v2 native shell and desktop bridge, macOS support with signed and notarized builds, packaging for Windows (NSIS), Linux (.deb and Flatpak with a self-hosted repo), fully offline operation with no cloud, no accounts and no telemetry, and native file operations, printing and PDF export.

## License

[AGPL-3.0](LICENSE)
