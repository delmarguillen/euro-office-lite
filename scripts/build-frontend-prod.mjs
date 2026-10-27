import { spawnSync } from 'node:child_process';
import {
  copyFile,
  cp,
  mkdir,
  readdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { applyPatches, revertPatches } from './apply-patches.mjs';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(scriptDir, '..');
const sdkBuildDir = path.join(projectRoot, 'src', 'sdkjs', 'build');
const webAppsBuildDir = path.join(projectRoot, 'src', 'web-apps', 'build');
const buildRoot = path.join(projectRoot, '.frontend-prod');
const outputRoot = path.join(projectRoot, 'src-dist');
const isWindows = process.platform === 'win32';

const sdkGrunt = path.join(
  sdkBuildDir,
  'node_modules',
  '.bin',
  isWindows ? 'grunt.cmd' : 'grunt',
);
const webAppsGrunt = path.join(
  projectRoot,
  'node_modules',
  '.bin',
  isWindows ? 'grunt.cmd' : 'grunt',
);

function fail(message) {
  console.error(`[frontend-prod] ERROR: ${message}`);
  process.exit(1);
}

function requirePath(filePath, hint) {
  if (!existsSync(filePath)) {
    fail(`${filePath} is missing${hint ? `; ${hint}` : ''}`);
  }
}

function run(command, args, cwd, extraEnv = {}) {
  console.log(`\n[frontend-prod] ${path.relative(projectRoot, cwd)} > ${path.basename(command)} ${args.join(' ')}`);
  const result = spawnSync(command, args, {
    cwd,
    env: { ...process.env, ...extraEnv },
    stdio: 'inherit',
    // Windows cannot execute npm-generated .cmd shims with shell: false.
    // All arguments here are fixed by this script, not user-controlled.
    shell: isWindows,
  });

  if (result.error) fail(result.error.message);
  if (result.status !== 0) {
    fail(`${path.basename(command)} exited with status ${result.status}`);
  }
}

async function copyRequired(source, destination) {
  requirePath(source);
  await mkdir(path.dirname(destination), { recursive: true });
  await copyFile(source, destination);
}

// The spellcheck dictionaries are committed, not fetched: the build only has to
// copy them to the root of the served frontend, where the worker looks for
// dictionaries/<lang>/<lang>.aff. manifest.json is the single list the shell
// reads at runtime, so a manifest that disagrees with what is on disk would ship
// either a language the editor offers and cannot check, or one nobody can reach.
// Both directions fail the build here instead.
async function copyDictionaries() {
  const sourceDir = path.join(projectRoot, 'src', 'dictionaries');
  const manifestPath = path.join(sourceDir, 'manifest.json');
  requirePath(manifestPath, 'the spellcheck dictionary manifest is required');

  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  } catch (error) {
    fail(`src/dictionaries/manifest.json is not valid JSON: ${error.message}`);
  }
  if (!Array.isArray(manifest) || manifest.some((entry) => typeof entry !== 'string')) {
    fail('src/dictionaries/manifest.json must be an array of language folder names');
  }

  const entries = await readdir(sourceDir, { withFileTypes: true });
  const onDisk = entries.filter((entry) => entry.isDirectory()).map((entry) => entry.name);

  const missing = manifest.filter((language) => !onDisk.includes(language));
  if (missing.length > 0) {
    fail(
      `src/dictionaries/manifest.json lists ${missing.join(', ')}, but ` +
        `src/dictionaries/ has no such folder (present: ${onDisk.join(', ') || 'none'})`,
    );
  }

  const unlisted = onDisk.filter((language) => !manifest.includes(language));
  if (unlisted.length > 0) {
    fail(
      `src/dictionaries/ contains ${unlisted.join(', ')}, which manifest.json does ` +
        'not list; add the folder name to the manifest or remove the folder',
    );
  }

  for (const language of manifest) {
    for (const extension of ['aff', 'dic']) {
      requirePath(
        path.join(sourceDir, language, `${language}.${extension}`),
        `the ${language} dictionary folder must contain ${language}.${extension}`,
      );
    }
  }

  await cp(sourceDir, path.join(outputRoot, 'dictionaries'), { recursive: true });
  console.log(`[frontend-prod] dictionaries staged: ${manifest.join(', ')}`);
}

async function injectSdkScripts(editor, moduleName) {
  const htmlPath = path.join(
    outputRoot,
    'web-apps',
    'apps',
    editor,
    'main',
    'index.html',
  );
  const marker = '<script src="../../../vendor/requirejs/require.js"></script>';
  let html = await readFile(htmlPath, 'utf8');
  const usesDynamicLoader = html.includes(pdfLoaderMarker);

  if (!html.includes(marker) && !usesDynamicLoader) {
    fail(`production loader marker not found in ${htmlPath}`);
  }

  // Desktop builds never ship sdkjs/slide/themes/themes.js: the desktop
  // SetThemesPath override (slide/Local/api.js) never requests it, but that
  // override only arrives with the asynchronous sdk-all.js. Until then the
  // web SetThemesPath from sdk-all-min.js issues one loadScript for
  // themes.js, and the missing-asset fallback answers with index.html, which
  // fails as a SyntaxError and is reported as error -82 while the document is
  // still opening. Reproduce the benign desktop miss instead: report the load
  // as failed without issuing a request. AscCommon.loadScript is defined by
  // sdk-all-min.js and is not redefined by sdk-all.js.
  const slideThemesGuardScript = `<script>
(function() {
  var originalLoadScript = AscCommon.loadScript;
  AscCommon.loadScript = function(url, onSuccess, onError) {
    if (typeof url === 'string' && /\\/themes\\.js(?:[?#]|$)/.test(url)) {
      if (onError) setTimeout(onError, 0);
      return;
    }
    return originalLoadScript.apply(this, arguments);
  };
  AscCommon.loadScript.__eoGuard = true;
})();
</script>`;

  const scripts = [
    '<!-- Euro-Office Lite static production SDK loader -->',
    // The SDK references XRegExp during its initial synchronous evaluation.
    // RequireJS starts later, so load this dependency explicitly first.
    `<script src='../../../vendor/xregexp/xregexp-all-min.js'></script>`,
    '<script src="../../../../sdkjs/vendor/polyfill.js"></script>',
    '<script src="../../../../sdkjs/common/AllFonts.js"></script>',
    `<script src="../../../../sdkjs/${moduleName}/sdk-all-min.js"></script>`,
    // Match the upstream compiled loader: apiBase loads sdk-all.js once and
    // asynchronously from loadSdk() after the editor API has been created,
    // so it must not appear as a static tag here.
    ...(moduleName === 'slide' ? [slideThemesGuardScript] : []),
  ];

  // The local Word bootstrap is deliberately outside the Closure bundle.
  // The old develop pipeline appended the same file to word/scripts.js.
  if (moduleName === 'word') {
    scripts.push('<script src="../../../../sdkjs/word/document/editor.js"></script>');
  }

  // The static tags above already evaluated XRegExp, AllFonts.js and
  // sdk-all-min.js, but app.js requires them again as the RequireJS modules
  // 'xregexp', 'allfonts' and 'sdk' (require.config paths + shim). Without
  // these named stubs RequireJS fetches and evaluates each bundle a second
  // time, which re-runs the AscCommon/AscFonts exports and silently wipes
  // every patch installed between the static tags and app start (the slide
  // themes.js guard and the font patches). The 'sdk'
  // stub keeps the original shim dependencies so jquery and socket.io still
  // load before the app starts, exactly like the upstream loader.
  const requirePredefineScript = `<script>
define('xregexp', [], function() { return window.XRegExp; });
define('allfonts', [], function() {});
define('sdk', ['jquery', 'allfonts', 'xregexp', 'socketio'], function() {});
</script>`;

  if (usesDynamicLoader) {
    html = injectIntoDynamicLoader(html, htmlPath, scripts, requirePredefineScript);
  } else {
    html = html.replace(
      marker,
      `${scripts.join('\n')}\n${marker}\n${requirePredefineScript}`,
    );
  }
  await writeFile(htmlPath, html, 'utf8');
}

// The PDF editor appends require.js from startApp() once it knows whether the
// document is a form, so there is no static tag to anchor on. The SDK tags go
// before that inline script, and the module stubs run from the require.js
// onload handler, before the app module is requested.
const pdfLoaderMarker = 'script.src = "../../../vendor/requirejs/require.js";';
const pdfStartAppPattern = /<script>\s*function startApp\(\)/;
const pdfDataMain = 'script.setAttribute("data-main", "app");';

function injectIntoDynamicLoader(html, htmlPath, scripts, requirePredefineScript) {
  const startApp = html.match(pdfStartAppPattern);
  if (!startApp) {
    fail(`startApp() loader not found in ${htmlPath}`);
  }
  if (!html.includes(pdfDataMain)) {
    fail(`data-main assignment not found in ${htmlPath}`);
  }

  const stubs = requirePredefineScript
    .replace('<script>', '')
    .replace('</script>', '')
    .trim();
  const onload = [
    'script.onload = function() {',
    stubs,
    "require(['app']);",
    '};',
  ].join('\n');

  const withScripts = html.replace(
    startApp[0],
    `${scripts.join('\n')}\n${startApp[0]}`,
  );
  return withScripts.replace(pdfDataMain, onload);
}

requirePath(
  sdkGrunt,
  'run `npm ci` in src/sdkjs/build before building the production frontend',
);
requirePath(
  webAppsGrunt,
  'run `npm ci` at the repository root before building the production frontend',
);
requirePath(
  path.join(projectRoot, 'src', 'sdkjs', 'common', 'AllFonts.js'),
  'run scripts/prepare-fonts.sh or scripts/prepare-fonts.ps1 first',
);

await rm(buildRoot, { recursive: true, force: true });
await rm(outputRoot, { recursive: true, force: true });
await mkdir(buildRoot, { recursive: true });

// grunt builds sdkjs and web-apps from the submodule trees, so the patches in
// patches/ go in first. They come back out on any exit, a failed step included.
const appliedPatches = applyPatches();
process.on('exit', () => revertPatches(appliedPatches));

const commonEnv = {
  APP_COPYRIGHT: 'Copyright (C) Euro-Office contributors. All rights reserved',
  BUILD_ROOT: buildRoot,
  COMPANY_NAME: 'Euro-Office',
  PRODUCT_NAME: 'Euro-Office Lite',
  PUBLISHER_NAME: 'Euro-Office contributors',
  PUBLISHER_URL: 'https://github.com/delmarguillen/euro-office-lite',
  SYSTEM_ENCODING: 'utf8',
  THEME: 'euro-office',
};

run(
  sdkGrunt,
  [
    'clean-deploy',
    'compile-word',
    'compile-cell',
    'compile-slide',
    'copy-other',
    '--desktop=true',
    '--level=SIMPLE',
    '--no-color',
  ],
  sdkBuildDir,
  commonEnv,
);

// copy-other intentionally does not include these generated/local desktop files.
await copyRequired(
  path.join(projectRoot, 'src', 'sdkjs', 'common', 'AllFonts.js'),
  path.join(buildRoot, 'sdkjs', 'common', 'AllFonts.js'),
);
await copyRequired(
  path.join(projectRoot, 'src', 'sdkjs', 'word', 'document', 'editor.js'),
  path.join(buildRoot, 'sdkjs', 'word', 'document', 'editor.js'),
);

// Build common desktop resources without deploy-sdk: sdkjs was compiled above
// into the same BUILD_ROOT, and deploy-sdk expects bundles inside the source tree.
run(
  webAppsGrunt,
  [
    'deploy-theme',
    'init-build-common',
    'deploy-api',
    'deploy-apps-common',
    'deploy-socketio',
    'deploy-xregexp',
    'deploy-requirejs',
    'deploy-megapixel',
    'deploy-jquery',
    'deploy-underscore',
    'deploy-iscroll',
    'deploy-fetch',
    'deploy-es6-promise',
    '--desktop=true',
    '--no-color',
  ],
  webAppsBuildDir,
  commonEnv,
);

for (const editor of [
  'documenteditor',
  'spreadsheeteditor',
  'presentationeditor',
]) {
  run(
    webAppsGrunt,
    [
      'deploy-theme',
      `init-build-${editor}`,
      'deploy-app-main',
      '--desktop=true',
      '--skip-babel',
      '--no-color',
    ],
    webAppsBuildDir,
    commonEnv,
  );
}

run(
  webAppsGrunt,
  ['deploy-theme', 'deploy-theme-images', '--desktop=true', '--no-color'],
  webAppsBuildDir,
  commonEnv,
);

// Match the existing desktop staging policy: the embedded help center contains
// hundreds of megabytes of duplicated GIFs and is not required by the editor
// runtime. The application already falls back to its configured online help.
for (const editor of [
  'documenteditor',
  'spreadsheeteditor',
  'presentationeditor',
]) {
  await rm(
    path.join(
      buildRoot,
      'web-apps',
      'apps',
      editor,
      'main',
      'resources',
      'help',
    ),
    { recursive: true, force: true },
  );
}

// Keep the WebAssembly engines used by fonts and the PDF viewer, but drop their
// asm.js fallbacks. All supported WebView2, WKWebView and WebKitGTK runtimes
// provide WebAssembly; the two fallback files add about 25 MiB.
await rm(
  path.join(buildRoot, 'sdkjs', 'pdf', 'src', 'engine', 'drawingfile_ie.js'),
  { force: true },
);
await rm(
  path.join(buildRoot, 'sdkjs', 'common', 'libfont', 'engine', 'fonts_ie.js'),
  { force: true },
);
// Same reasoning for the spellcheck engine, whose asm.js fallback adds about
// 770 KiB: the worker instantiates spell.wasm on every runtime we support.
// spell.js.mem is that fallback's startup memory image, referenced by nothing
// else, so it goes with it.
await rm(
  path.join(buildRoot, 'sdkjs', 'common', 'spell', 'spell', 'spell_ie.js'),
  { force: true },
);
await rm(
  path.join(buildRoot, 'sdkjs', 'common', 'spell', 'spell', 'spell.js.mem'),
  { force: true },
);

await mkdir(outputRoot, { recursive: true });
await copyRequired(
  path.join(projectRoot, 'src', 'index.html'),
  path.join(outputRoot, 'index.html'),
);
await copyRequired(
  path.join(projectRoot, 'src', 'bridge.js'),
  path.join(outputRoot, 'bridge.js'),
);
await copyRequired(
  path.join(projectRoot, 'src', 'button-hint-patch.js'),
  path.join(outputRoot, 'button-hint-patch.js'),
);
await copyRequired(
  path.join(projectRoot, 'src', 'font-patches.js'),
  path.join(outputRoot, 'font-patches.js'),
);
await copyRequired(
  path.join(projectRoot, 'src', 'editor-patches.js'),
  path.join(outputRoot, 'editor-patches.js'),
);
await cp(path.join(projectRoot, 'src', 'fonts'), path.join(outputRoot, 'fonts'), {
  recursive: true,
});
await copyDictionaries();
await cp(path.join(buildRoot, 'sdkjs'), path.join(outputRoot, 'sdkjs'), {
  recursive: true,
});
await cp(path.join(buildRoot, 'web-apps'), path.join(outputRoot, 'web-apps'), {
  recursive: true,
});

await injectSdkScripts('documenteditor', 'word');
await injectSdkScripts('spreadsheeteditor', 'cell');
await injectSdkScripts('presentationeditor', 'slide');

run(
  process.execPath,
  [path.join(scriptDir, 'validate-frontend-prod.mjs')],
  projectRoot,
);

if (process.env.KEEP_FRONTEND_BUILD !== '1') {
  await rm(buildRoot, { recursive: true, force: true });
}

console.log('\n[frontend-prod] Production frontend staged in src-dist/');
