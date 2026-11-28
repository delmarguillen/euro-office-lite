// Checks where font-patches.js reads a font file from: a system font by the
// absolute path AllFonts.js gives it, a bundled font from the folder the
// backend names (binaries/fonts), both through the abs/ route of ascdesktop.
//
// Run: node scripts/test-bridge-font-urls.mjs
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { check, finish, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const source = await readFile(path.join(projectRoot, 'src', 'font-patches.js'), 'utf8');

// font-patches.js in a page whose backend bundles its fonts in `fontsDir`.
function loadFontPatches(protoBase, fontsDir) {
  const invoked = [];
  const window = {
    _eoAscProtoBase: protoBase,
    _eoLog() {},
    __TAURI__: {
      core: {
        invoke(command) {
          invoked.push(command);
          return Promise.resolve(fontsDir);
        },
      },
    },
  };
  const sandbox = { window, console, encodeURIComponent, Promise };
  vm.runInNewContext(source, sandbox);
  return { url: (id) => window._eoFontUrl(id), prepare: () => sandbox._prepareBundledFonts(), invoked };
}

const linux = loadFontPatches('ascdesktop://', '/usr/lib/Euro-Office-Lite/binaries/fonts');
await linux.prepare();
check(results, 'Linux: a bundled font is read from the bundled folder',
  linux.url('Carlito-Bold.ttf') === 'ascdesktop://abs//usr/lib/Euro-Office-Lite/binaries/fonts/Carlito-Bold.ttf',
  linux.url('Carlito-Bold.ttf'));
check(results, 'Linux: a system font is read from its own path',
  linux.url('/usr/share/fonts/DejaVuSans.ttf') === 'ascdesktop://abs//usr/share/fonts/DejaVuSans.ttf',
  linux.url('/usr/share/fonts/DejaVuSans.ttf'));
await linux.prepare();
check(results, 'the folder is asked for once', linux.invoked.join() === 'bundled_fonts_dir', linux.invoked.join());

const windows = loadFontPatches('http://ascdesktop.localhost/', 'C:/Users/Ana #1/AppData/Local/Euro-Office-Lite/binaries/fonts');
await windows.prepare();
check(results, 'Windows: the folder is percent-encoded, so # and spaces stay in the path',
  windows.url('LiberationSans-Regular.ttf') === 'http://ascdesktop.localhost/abs/C%3A/Users/Ana%20%231/AppData/Local/Euro-Office-Lite/binaries/fonts/LiberationSans-Regular.ttf',
  windows.url('LiberationSans-Regular.ttf'));
check(results, 'Windows: a system font is read from its own path',
  windows.url('C:/Windows/Fonts/arial.ttf') === 'http://ascdesktop.localhost/abs/C:/Windows/Fonts/arial.ttf',
  windows.url('C:/Windows/Fonts/arial.ttf'));
check(results, 'Windows: the extended prefix x2t writes is dropped',
  windows.url('/?/C:/Windows/Fonts/arial.ttf') === 'http://ascdesktop.localhost/abs/C:/Windows/Fonts/arial.ttf',
  windows.url('/?/C:/Windows/Fonts/arial.ttf'));

const macos = loadFontPatches('ascdesktop://', '/Applications/Euro-Office-Lite.app/Contents/Resources/binaries/fonts');
await macos.prepare();
check(results, 'macOS: a bundled font is read from the resources of the .app',
  macos.url('LiberationSerif-Italic.ttf') === 'ascdesktop://abs//Applications/Euro-Office-Lite.app/Contents/Resources/binaries/fonts/LiberationSerif-Italic.ttf',
  macos.url('LiberationSerif-Italic.ttf'));

check(results, 'no font is read from the embedded frontend', !source.includes("'/fonts/'"));

finish(results);
