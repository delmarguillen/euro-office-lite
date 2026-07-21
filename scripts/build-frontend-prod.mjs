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

async function normalizeSlideThemesScriptPath(bundlePath) {
  const themeScriptPattern =
    /AscCommon\.loadScript\(([A-Za-z_$][\w$]*)\+"\/themes\.js",/g;
  let bundle = await readFile(bundlePath, 'utf8');
  const matches = [...bundle.matchAll(themeScriptPattern)];

  if (matches.length !== 1) {
    fail(
      `expected exactly one slide themes.js loader in ${bundlePath}; ` +
      `found ${matches.length}`,
    );
  }

  bundle = bundle.replace(
    themeScriptPattern,
    (_match, themePath) =>
      `AscCommon.loadScript(${themePath}.replace(/\\/+$/,'')+'/themes.js',`,
  );
  await writeFile(bundlePath, bundle, 'utf8');
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

  const diagScript1 = `<script>
(function() {
  window.__eoFontDiag = { t0: performance.now(), events: [] };
  function diag(label, data) {
    var t = Math.round(performance.now() - window.__eoFontDiag.t0);
    var msg = '[FONT-DIAG ' + t + 'ms] ' + label + (data ? ' ' + data : '');
    window.__eoFontDiag.events.push(msg);
    try { console.log(msg); } catch(e) {}
    try { window.parent._eoLog(msg); } catch(e) {}
  }
  window.__eoDiag = diag;
  diag('init', 'diagnostics installed');
  window.addEventListener('error', function(ev) {
    diag('error-listener', 'msg=' + (ev.message || '') +
      ' src=' + (ev.filename || '').split('/').pop() + ':' + (ev.lineno || 0));
  }, true);
  window.addEventListener('unhandledrejection', function(ev) {
    var reason = ev && ev.reason;
    diag('rejection-listener', 'reason=' + (reason && reason.message || reason));
  });
  var origXhrOpen = XMLHttpRequest.prototype.open;
  var fontXhrCount = 0;
  XMLHttpRequest.prototype.open = function(method, url) {
    var self = this;
    if (typeof url === 'string' && (/\\.ttf|\\.otf/i.test(url) || /\\/fonts\\//i.test(url) || /ascdesktop:\\/\\/fonts\\//i.test(url))) {
      fontXhrCount++;
      self.__eoFontUrl = url.split('/').pop().split('?')[0];
      self.__eoFontNum = fontXhrCount;
      self.addEventListener('loadend', function() {
        var bytes = 0;
        try {
          var response = self.response;
          if (response && typeof response.byteLength === 'number') bytes = response.byteLength;
          else if (response && typeof response.size === 'number') bytes = response.size;
          else if (typeof self.responseText === 'string') bytes = self.responseText.length;
        } catch(e) {}
        if (self.__eoFontNum <= 10 || self.status !== 200) {
          diag('font-xhr', '#' + self.__eoFontNum + ' file=' + self.__eoFontUrl +
            ' status=' + self.status + ' bytes=' + bytes +
            ' ascdesktop=' + (url.indexOf('ascdesktop://fonts/') !== -1));
        } else if (self.__eoFontNum === 50) {
          diag('font-xhr', 'aggregate: 50 requests so far');
        }
      }, { once: true });
    }
    return origXhrOpen.apply(this, arguments);
  };
})();
</script>`;

  const diagScript2 = `<script>
(function() {
  var diag = window.__eoDiag || function(){};
  diag('post-AllFonts',
    '__fonts_files=' + (window['__fonts_files'] ? window['__fonts_files'].length : 'undef') +
    ' __fonts_infos=' + (window['__fonts_infos'] ? window['__fonts_infos'].length : 'undef') +
    ' g_fonts_selection_bin.len=' + ((window['g_fonts_selection_bin'] || '').length));
})();
</script>`;

  const diagScript3 = `<script>
(function() {
  var diag = window.__eoDiag || function(){};
  var af = window.AscFonts;
  if (!af) { diag('post-sdk-min', 'AscFonts=undefined'); return; }
  var initialGfa = af.g_fontApplication;
  window.__eoInitialGfa = initialGfa;
  var initialLFA = af.CFontFileLoader && af.CFontFileLoader.prototype.LoadFontAsync;
  window.__eoInitialLFA = initialLFA;
  function describeLFA(fn) {
    var src = String(fn || '');
    return 'changed=' + (fn !== window.__eoInitialLFA) +
      ' ascdesktopFonts=' + (src.indexOf('ascdesktop://fonts/') !== -1) +
      ' tauriAbs=' + (src.indexOf('abs/') !== -1) +
      ' sourceLen=' + src.length;
  }
  window.__eoDescribeLFA = describeLFA;
  var sel = initialGfa && initialGfa.g_fontSelections;
  diag('post-sdk-min',
    'IsInit=' + (sel && sel.IsInit) +
    ' List.length=' + (sel && sel.List ? sel.List.length : 'null') +
    ' g_font_files=' + (af.g_font_files ? af.g_font_files.length : 'null') +
    ' g_font_infos=' + (af.g_font_infos ? af.g_font_infos.length : 'null') +
    ' selection_bin.len=' + ((window['g_fonts_selection_bin'] || '').length));
  diag('post-sdk-min', 'LoadFontAsync: ' + describeLFA(initialLFA));
  function reportFullSdkLoaded(source) {
    var currentAf = window.AscFonts;
    var currentGfa = currentAf && currentAf.g_fontApplication;
    var currentLFA = currentAf && currentAf.CFontFileLoader &&
      currentAf.CFontFileLoader.prototype.LoadFontAsync;
    diag('sdk-all-loaded',
      'source=' + source +
      ' gfaReplaced=' + (currentGfa !== window.__eoInitialGfa) +
      ' g_font_files=' + (currentAf && currentAf.g_font_files ? currentAf.g_font_files.length : 'null') +
      ' g_font_infos=' + (currentAf && currentAf.g_font_infos ? currentAf.g_font_infos.length : 'null'));
    diag('sdk-all-loaded', 'LoadFontAsync: ' + describeLFA(currentLFA));
  }
  var sdkInsertCount = 0;
  var obs = new MutationObserver(function(mutations) {
    for (var i = 0; i < mutations.length; i++) {
      var nodes = mutations[i].addedNodes;
      for (var j = 0; j < nodes.length; j++) {
        var node = nodes[j];
        if (node.tagName === 'SCRIPT' && node.src && /\\/sdk-all\\.js(?:[?#]|$)/.test(node.src)) {
          sdkInsertCount++;
          diag('sdk-all-dynamic-insert', 'count=' + sdkInsertCount + ' src=' + node.src.split('/').pop());
          node.addEventListener('load', function() {
            reportFullSdkLoaded('dynamic-script');
          }, { once: true });
          node.addEventListener('error', function() {
            diag('sdk-all-dynamic-error', 'src=' + node.src);
          }, { once: true });
        }
      }
    }
  });
  obs.observe(document.head, { childList: true });
  window.__eoSdkObserver = obs;
  if (window.AscCommon && window.AscCommon.loadSdk) {
    var origLoadSdk = window.AscCommon.loadSdk;
    window.AscCommon.loadSdk = function(name, onSuccess, onError) {
      diag('loadSdk-called', 'name=' + name);
      return origLoadSdk.call(this, name, function() {
        var currentGfa = window.AscFonts.g_fontApplication;
        var gfaReplaced = currentGfa !== window.__eoInitialGfa;
        diag('loadSdk-callback',
          'gfaReplaced=' + gfaReplaced +
          ' g_font_files=' + (window.AscFonts.g_font_files ? window.AscFonts.g_font_files.length : 'null') +
          ' g_font_infos=' + (window.AscFonts.g_font_infos ? window.AscFonts.g_font_infos.length : 'null'));
        var lfn2 = window.AscFonts.CFontFileLoader.prototype.LoadFontAsync;
        var describe = window.__eoDescribeLFA || function() { return 'unavailable'; };
        diag('loadSdk-callback', 'LoadFontAsync: ' + describe(lfn2));
        var gfa = window.AscFonts.g_fontApplication;
        if (gfa && gfa.g_fontSelections && !gfa.g_fontSelections.__eoDiagWrapped) {
          gfa.g_fontSelections.__eoDiagWrapped = true;
          var origInit = gfa.g_fontSelections.Init;
          gfa.g_fontSelections.Init = function() {
            var binLen = (window['g_fonts_selection_bin'] || '').length;
            diag('fontSelections.Init-BEFORE',
              'IsInit=' + gfa.g_fontSelections.IsInit +
              ' selection_bin.len=' + binLen +
              ' g_font_files=' + (window.AscFonts.g_font_files ? window.AscFonts.g_font_files.length : 'null'));
            var result = origInit.apply(this, arguments);
            diag('fontSelections.Init-AFTER',
              'List.length=' + (gfa.g_fontSelections.List ? gfa.g_fontSelections.List.length : 'null') +
              ' hasFamiliesNotASCW3=' + !!(gfa.g_fontSelections.List && gfa.g_fontSelections.List.some(function(e) {
                return e.m_wsFontName && e.m_wsFontName !== 'ASCW3';
              })));
            return result;
          };
        }
        return onSuccess.apply(this, arguments);
      }, onError);
    };
  }
  setTimeout(function() {
    try {
      var entries = performance.getEntriesByType('resource')
        .filter(function(e) { return /\\/sdk-all\\.js(?:[?#]|$)/.test(e.name); });
      diag('resource-count', 'sdk-all.js=' + entries.length);
    } catch(e) {}
  }, 10000);
})();
</script>`;

  const scripts = [
    '<!-- Euro-Office Lite static production SDK loader -->',
    diagScript1,
    // The SDK references XRegExp during its initial synchronous evaluation.
    // RequireJS starts later, so load this dependency explicitly first.
    `<script src='../../../vendor/xregexp/xregexp-all-min.js'></script>`,
    '<script src="../../../../sdkjs/vendor/polyfill.js"></script>',
    '<script src="../../../../sdkjs/common/AllFonts.js"></script>',
    diagScript2,
    `<script src="../../../../sdkjs/${moduleName}/sdk-all-min.js"></script>`,
    // Match ONLYOFFICE's compiled loader: apiBase loads sdk-all.js once and
    // asynchronously from loadSdk() after the editor API has been created.
    diagScript3,
  ];

  // The local Word bootstrap is deliberately outside the Closure bundle.
  // The old develop pipeline appended the same file to word/scripts.js.
  if (moduleName === 'word') {
    scripts.push('<script src="../../../../sdkjs/word/document/editor.js"></script>');
  }

  html = html.replace(marker, `${scripts.join('\n')}\n${marker}`);
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

// The presentation app passes a directory with a trailing slash while the
// partial SDK appends "/themes.js". Normalize only that script request in the
// compiled bundle; themeN/theme.bin paths must retain their trailing slash.
await normalizeSlideThemesScriptPath(
  path.join(buildRoot, 'sdkjs', 'slide', 'sdk-all-min.js'),
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
