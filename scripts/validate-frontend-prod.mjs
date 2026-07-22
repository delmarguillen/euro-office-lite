import { existsSync } from 'node:fs';
import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(scriptDir, '..');
const distRoot = path.join(projectRoot, 'src-dist');

function fail(message) {
  console.error(`[frontend-prod] ERROR: ${message}`);
  process.exitCode = 1;
}

function requireFile(relativePath) {
  const absolutePath = path.join(distRoot, relativePath);
  if (!existsSync(absolutePath)) {
    fail(`missing production asset: ${relativePath}`);
  }
}

const required = [
  'index.html',
  'bridge.js',
  'web-apps/vendor/xregexp/xregexp-all-min.js',
  'sdkjs/vendor/polyfill.js',
  'sdkjs/common/AllFonts.js',
  'sdkjs/word/sdk-all-min.js',
  'sdkjs/word/sdk-all.js',
  'sdkjs/word/document/editor.js',
  'sdkjs/cell/sdk-all-min.js',
  'sdkjs/cell/sdk-all.js',
  'sdkjs/slide/sdk-all-min.js',
  'sdkjs/slide/sdk-all.js',
  'web-apps/apps/api/documents/api.js',
];

for (const editor of [
  'documenteditor',
  'spreadsheeteditor',
  'presentationeditor',
]) {
  required.push(
    `web-apps/apps/${editor}/main/index.html`,
    `web-apps/apps/${editor}/main/app.js`,
    `web-apps/apps/${editor}/main/resources/css/app.css`,
  );
}

required.forEach(requireFile);

const shellHtmlPath = path.join(distRoot, 'index.html');
if (existsSync(shellHtmlPath)) {
  const shellHtml = await readFile(shellHtmlPath, 'utf8');
  for (const marker of [
    "api.asc_isOffline = api['asc_isOffline']",
    'mainController.appOptions.isOffline = true',
    'leftMenuController.setMode(leftMenuController.mode)',
  ]) {
    if (!shellHtml.includes(marker)) {
      fail('index.html is missing desktop-offline integration marker: ' + marker);
    }
  }
}

const slideBundlePath = path.join(distRoot, 'sdkjs', 'slide', 'sdk-all-min.js');
if (existsSync(slideBundlePath)) {
  const slideBundle = await readFile(slideBundlePath, 'utf8');
  // The runtime guard injected into the presentation HTML intercepts this
  // exact call through the AscCommon.loadScript property. If the compiled
  // shape changes, the guard must be revisited.
  const themeLoaderCalls = [
    ...slideBundle.matchAll(
      /AscCommon\.loadScript\(([A-Za-z_$][\w$]*)\+"\/themes\.js",/g,
    ),
  ];
  if (themeLoaderCalls.length !== 1) {
    fail(
      'slide/sdk-all-min.js must contain exactly one themes.js loader ' +
      `calling AscCommon.loadScript; found ${themeLoaderCalls.length}`,
    );
  }
  if (slideBundle.includes(".replace(/\\/+$/,'')+'/themes.js'")) {
    fail('slide/sdk-all-min.js still contains the obsolete themes.js rewrite');
  }
}

for (const forbidden of [
  'sdkjs/develop',
  'sdkjs/pdf/src/engine/drawingfile_ie.js',
  'sdkjs/common/libfont/engine/fonts_ie.js',
  'web-apps/vendor/less',
  'web-apps/apps/documenteditor/main/resources/help',
  'web-apps/apps/spreadsheeteditor/main/resources/help',
  'web-apps/apps/presentationeditor/main/resources/help',
]) {
  if (existsSync(path.join(distRoot, forbidden))) {
    fail(`excluded asset was staged: ${forbidden}`);
  }
}

const editorModules = new Map([
  ['documenteditor', 'word'],
  ['spreadsheeteditor', 'cell'],
  ['presentationeditor', 'slide'],
]);

for (const [editor, moduleName] of editorModules) {
  const relativePath = `web-apps/apps/${editor}/main/index.html`;
  const htmlPath = path.join(distRoot, relativePath);
  if (!existsSync(htmlPath)) continue;

  const html = await readFile(htmlPath, 'utf8');
  for (const marker of [
    "less.env='development'",
    'develop/sdkjs',
    'data-main="app_dev"',
  ]) {
    if (html.includes(marker)) fail(`${relativePath} contains ${marker}`);
  }

  for (const expected of [
    '../../../vendor/xregexp/xregexp-all-min.js',
    '../../../../sdkjs/vendor/polyfill.js',
    '../../../../sdkjs/common/AllFonts.js',
    `../../../../sdkjs/${moduleName}/sdk-all-min.js`,
  ]) {
    if (!html.includes(expected)) {
      fail(`${relativePath} does not load ${expected}`);
    }
  }

  const xregexpPath = '../../../vendor/xregexp/xregexp-all-min.js';
  const polyfillPath = '../../../../sdkjs/vendor/polyfill.js';
  const allFontsPath = '../../../../sdkjs/common/AllFonts.js';
  const sdkMinPath = `../../../../sdkjs/${moduleName}/sdk-all-min.js`;
  const sdkAllPath = `../../../../sdkjs/${moduleName}/sdk-all.js`;
  const notLoadStatement = "window['AscNotLoadAllScript'] = true;";
  const requirePos = html.indexOf('vendor/requirejs/require.js');
  if (html.includes(sdkAllPath)) {
    fail(`${relativePath} loads ${sdkAllPath} statically`);
  }
  if (html.includes(notLoadStatement)) {
    fail(`${relativePath} disables the asynchronous sdk-all.js loader`);
  }
  if (html.includes('window.__eoThemePathPatched')) {
    fail(`${relativePath} contains the obsolete runtime theme-path patch`);
  }
  if (requirePos === -1) {
    fail(`${relativePath} is missing require.js`);
  }
  if (
    requirePos !== -1 &&
    !(
      html.indexOf(xregexpPath) < html.indexOf(polyfillPath) &&
      html.indexOf(polyfillPath) < html.indexOf(allFontsPath) &&
      html.indexOf(allFontsPath) < html.indexOf(sdkMinPath) &&
      html.indexOf(sdkMinPath) < requirePos
    )
  ) {
    fail(
      `${relativePath} must load XRegExp, polyfill.js, AllFonts.js, ` +
      'sdk-all-min.js, and require.js in that order',
    );
  }

  // app.js requires 'sdk', 'allfonts' and 'xregexp' through RequireJS even
  // though the static loader already evaluated those bundles. The named
  // define stubs must sit after require.js so RequireJS reuses the loaded
  // bundles instead of fetching and re-evaluating them, which would wipe the
  // patches installed between the static tags and app start.
  for (const stub of [
    "define('xregexp', [], function() { return window.XRegExp; });",
    "define('allfonts', [], function() {});",
    "define('sdk', ['jquery', 'allfonts', 'xregexp', 'socketio'], function() {",
  ]) {
    const stubPos = html.indexOf(stub);
    if (stubPos === -1) {
      fail(`${relativePath} is missing the RequireJS module stub: ${stub}`);
    } else if (stubPos < requirePos) {
      fail(
        `${relativePath} must register RequireJS module stubs after require.js`,
      );
    }
  }

  // The slide guard suppresses the one themes.js request the web
  // SetThemesPath from sdk-all-min.js fires before the desktop override in
  // sdk-all.js takes over; without it the missing-asset fallback serves HTML
  // as JS and the resulting SyntaxError surfaces as error -82.
  const themesGuardMarker = String.raw`/\/themes\.js(?:[?#]|$)/.test(url)`;
  const themesGuardPos = html.indexOf(themesGuardMarker);
  if (moduleName === 'slide') {
    if (themesGuardPos === -1) {
      fail(`${relativePath} is missing the slide themes.js loader guard`);
    } else if (
      !(html.indexOf(sdkMinPath) < themesGuardPos && themesGuardPos < requirePos)
    ) {
      fail(
        `${relativePath} must install the slide themes.js loader guard ` +
        'between sdk-all-min.js and require.js',
      );
    }
  } else if (themesGuardPos !== -1) {
    fail(`${relativePath} unexpectedly contains the slide themes.js guard`);
  }
}

async function measure(directory) {
  let files = 0;
  let bytes = 0;
  const entries = await readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      const child = await measure(fullPath);
      files += child.files;
      bytes += child.bytes;
    } else if (entry.isFile()) {
      files += 1;
      bytes += (await stat(fullPath)).size;
    }
  }
  return { files, bytes };
}

if (existsSync(distRoot)) {
  const { files, bytes } = await measure(distRoot);
  const mebibytes = bytes / 1024 / 1024;
  console.log(
    `[frontend-prod] validated ${files.toLocaleString('en-US')} files, ${mebibytes.toFixed(1)} MiB`,
  );

  // Guard against accidentally reintroducing source trees or offline help
  // media. The pruned production staging is expected around 164 MiB / 1,170
  // files; the old develop staging was 207.7 MiB / 5,662 files.
  if (files > 1_500) {
    fail(`production frontend exceeds file budget: ${files} > 1,500`);
  }
  if (mebibytes > 180) {
    fail(
      `production frontend exceeds size budget: ${mebibytes.toFixed(1)} MiB > 180 MiB`,
    );
  }
}

if (process.exitCode) process.exit(process.exitCode);
