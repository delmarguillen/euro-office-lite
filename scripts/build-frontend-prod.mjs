import { spawnSync } from 'node:child_process';
import {
  copyFile,
  cp,
  mkdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

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

  if (!html.includes(marker)) {
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

  html = html.replace(
    marker,
    `${scripts.join('\n')}\n${marker}\n${requirePredefineScript}`,
  );
  await writeFile(htmlPath, html, 'utf8');
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

await mkdir(outputRoot, { recursive: true });
await copyRequired(
  path.join(projectRoot, 'src', 'index.html'),
  path.join(outputRoot, 'index.html'),
);
await copyRequired(
  path.join(projectRoot, 'src', 'bridge.js'),
  path.join(outputRoot, 'bridge.js'),
);
await cp(path.join(projectRoot, 'src', 'fonts'), path.join(outputRoot, 'fonts'), {
  recursive: true,
});
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
