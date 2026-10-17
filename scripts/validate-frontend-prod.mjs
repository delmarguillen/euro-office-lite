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
  'button-hint-patch.js',
  'font-patches.js',
  'editor-patches.js',
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
  'sdkjs/common/spell/spell/spell.js',
  'sdkjs/common/spell/spell/spell.wasm',
  'dictionaries/manifest.json',
];

// Every language the manifest offers must be answerable: the shell advertises
// exactly this list to the editor, and a language advertised without its two
// Hunspell files would report every word of it as a mistake.
const manifestPath = path.join(distRoot, 'dictionaries', 'manifest.json');
if (existsSync(manifestPath)) {
  let languages = [];
  try {
    languages = JSON.parse(await readFile(manifestPath, 'utf8'));
  } catch (error) {
    fail(`staged dictionaries/manifest.json is not valid JSON: ${error.message}`);
  }
  if (!Array.isArray(languages) || languages.length === 0) {
    fail('staged dictionaries/manifest.json must be a non-empty array');
  } else {
    for (const language of languages) {
      required.push(
        `dictionaries/${language}/${language}.aff`,
        `dictionaries/${language}/${language}.dic`,
      );
    }
  }
}

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

const editorPatchesPath = path.join(distRoot, 'editor-patches.js');
if (existsSync(editorPatchesPath)) {
  const editorPatches = await readFile(editorPatchesPath, 'utf8');
  for (const marker of [
    "api.asc_isOffline = api['asc_isOffline']",
    'mainController.appOptions.isOffline = true',
    'leftMenuController.setMode(leftMenuController.mode)',
  ]) {
    if (!editorPatches.includes(marker)) {
      fail('editor-patches.js is missing desktop-offline integration marker: ' + marker);
    }
  }
}

// The Linux paste guard in bridge.js lets a paste through when text/x-custom
// starts with asc_internalData2;, the tag sdkjs puts on its own copies. If the
// tag moves, the guard would swallow those pastes again (a slide copies as a
// lone <img> plus that binary).
for (const moduleName of ['word', 'cell', 'slide']) {
  const bundlePath = path.join(distRoot, 'sdkjs', moduleName, 'sdk-all.js');
  // Missing means nothing was checked: that must not read as a pass.
  if (!existsSync(bundlePath)) {
    fail(
      `Linux paste guard: sdkjs/${moduleName}/sdk-all.js is missing, ` +
      `so the internal copy tag could not be checked`,
    );
    continue;
  }
  const bundle = await readFile(bundlePath, 'utf8');
  const tagged = [
    ...bundle.matchAll(
      compiledShape('"text/x-custom"==<r>?this.ClosureParams.setData(<r>,"asc_internalData2;"+<p>)'),
    ),
  ].length;
  if (tagged !== 1) {
    fail(
      `Linux paste guard: ${moduleName}/sdk-all.js no longer tags its internal copies ` +
      `as text/x-custom "asc_internalData2;" (found ${tagged}). Check sdkjs ` +
      'common/clipboard_base.js and the paste listener in bridge.js.',
    );
  }
  // The navigator.clipboard copy has no text/x-custom: the guard then knows the
  // editor's copy by repeating this walk of sdkjs over the pasted HTML.
  const classTags = [
    ...bundle.matchAll(
      compiledShape(
        'function <find>(<node>){if(<node>.children&&<node>.children[0]){' +
          'var <c>=<node>.children[0];<c>=<c>?<c>.getAttribute("class"):null;' +
          'return null!=<c>&&(-1<<c>.indexOf("xslData;")||-1<<c>.indexOf("docData;")||' +
          '-1<<c>.indexOf("pptData;")||-1<<c>.indexOf("pdfData;"))?<c>:<find>(<node>.children[0])}' +
          'return null}',
      ),
    ),
  ].length;
  if (classTags !== 1) {
    fail(
      `Linux paste guard: ${moduleName}/sdk-all.js no longer finds its copies by the ` +
      `xslData;/docData;/pptData;/pdfData; class tags (found ${classTags}). Check ` +
      'searchBinaryClass in sdkjs common/wordcopypaste.js and findBinaryClass in bridge.js.',
    );
  }
  // The Paste menu and the non-Latin Ctrl+V read the clipboard HTML and know an
  // editor copy by these tags in a class (_eoHtmlIsEditorCopy), then paste it
  // with the sequence of sdkjs's own paste handler.
  const htmlPasteProblems = [];
  const writerTags = moduleName === 'cell'
    ? ['docData;', 'pptData;', 'pdfData;', 'xslData;']
    : ['docData;', 'pptData;', 'pdfData;'];
  for (const tag of writerTags) {
    if (!new RegExp(`"${tag}"\\s*\\+`).test(bundle)) {
      htmlPasteProblems.push(`no copy writes "${tag}"`);
    }
  }
  if (!/this\.PasteFlag\s*=\s*(?:!0|true)\s*[;,]\s*this\.Api\.incrementCounterLongAction\(\)\s*[;,]\s*this\.pastedFrom\s*=\s*null/.test(bundle)) {
    htmlPasteProblems.push('the paste handler no longer starts with PasteFlag, incrementCounterLongAction, pastedFrom');
  }
  if (!/CommonIframe_PasteStart\s*:\s*function\s*\([\s\S]{0,3000}?\.Paste_End\(\)\s*;?\s*\}/.test(bundle)) {
    htmlPasteProblems.push('CommonIframe_PasteStart no longer ends in Paste_End');
  }
  if (!/Paste_End\s*:\s*function\s*\(\)\s*\{[^}]*\.decrementCounterLongAction\(\)[^}]*this\.PasteFlag\s*=\s*(?:!1|false)/.test(bundle)) {
    htmlPasteProblems.push('Paste_End no longer lowers the long-action counter and the paste flag');
  }
  if (htmlPasteProblems.length > 0) {
    fail(
      `internal HTML paste: ${moduleName}/sdk-all.js changed (${htmlPasteProblems.join('; ')}). ` +
      'Check the copy writers in sdkjs common/wordcopypaste.js and cell/model/clipboard.js, ' +
      '_private_onpaste, CommonIframe_PasteStart and Paste_End in common/clipboard_base.js, ' +
      'and _eoHtmlIsEditorCopy / _eoPasteEditorHtmlFromClipboard in bridge.js.',
    );
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

// Turns a compiled-code template into a regex. <name> stands for a minified
// identifier (repeats must match the first one), <*> for a short stretch of
// code that may change freely, and tokens are joined with \s* because the
// compiler breaks long lines between arbitrary tokens.
function compiledShape(template) {
  const names = new Set();
  const source = template
    .match(/<\*>|<\w+>|[A-Za-z_$][\w$]*|\S/g)
    .map((token) => {
      if (token === '<*>') return '[\\s\\S]{0,600}?';
      const name = /^<(\w+)>$/.exec(token)?.[1];
      if (!name) return escapeRegExp(token);
      if (names.has(name)) return `\\k<${name}>`;
      names.add(name);
      return `(?<${name}>[A-Za-z_$][\\w$]*)`;
    })
    .join('\\s*');
  return new RegExp(source, 'g');
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// The #49 aspect patch in editor-patches.js wraps
// CPresentation.prototype.internalChangeSizes and fixes up locked objects with
// the CXfrm setters after the stock resize. Undo stays exact only while the
// resize path keeps this shape: internalChangeSizes records sldSz before
// changeSlideSizeFunction, which rescales masters, layouts and slides without
// history and is what the SlideSize change replays on undo, redo and from
// co-authors; Slide.changeSize uses the slide's own size; and
// CShape.prototype.changeSize, which CImageShape and CChartSpace copy, stretches
// X and Y separately, which is exactly invertible. All of it lives in
// sdk-all.js, not in sdk-all-min.js.
const slideFullPath = path.join(distRoot, 'sdkjs', 'slide', 'sdk-all.js');
if (existsSync(slideFullPath)) {
  const bundle = await readFile(slideFullPath, 'utf8');
  const problems = [];
  // A missing name points at upstream; a present name with another shape can
  // be either, so the message says which one it is.
  const expectOne = (label, marker, template) => {
    const found = [...bundle.matchAll(compiledShape(template))];
    if (found.length === 1) return found[0];
    if (!compiledShape(marker).test(bundle)) {
      problems.push(`${label} is gone (renamed or removed upstream)`);
    } else {
      problems.push(`${label} has a new compiled shape (found ${found.length} matches)`);
    }
    return null;
  };

  expectOne(
    'CPresentation.prototype.internalChangeSizes',
    'CPresentation.prototype.internalChangeSizes=',
    'CPresentation.prototype.internalChangeSizes=function(<w>,<h>,<t>){' +
      'var <s>=new CSlideSize;<s>.setCX(<w>);<s>.setCY(<h>);<*>' +
      'this.setSldSz(<s>);this.changeSlideSizeFunction()}',
  );
  expectOne(
    'CPresentation.prototype.changeSlideSizeFunction',
    'CPresentation.prototype.changeSlideSizeFunction=',
    'CPresentation.prototype.changeSlideSizeFunction=function(){' +
      'AscFormat.ExecuteNoHistory(function(){' +
      'var <w>=this.GetWidthMM(),<h>=this.GetHeightMM()<*>' +
      'this.slideMasters[<i>].changeSize(<w>,<h>);<*>' +
      '.sldLayoutLst[<j>].changeSize(<w>,<h>)}<*>' +
      'this.Slides[<k>].changeSize(<w>,<h>)},this,[])}',
  );
  expectOne(
    'the SlideSize change handler',
    'AscDFH.drawingsChangesMap[AscDFH.historyitem_Presentation_SlideSize]=',
    'AscDFH.drawingsChangesMap[AscDFH.historyitem_Presentation_SlideSize]=' +
      'function(<c>,<v>){<c>.sldSz=<v>;<c>.changeSlideSizeFunction()}',
  );
  expectOne(
    'Slide.prototype.changeSize',
    'Slide.prototype.changeSize=function',
    'Slide.prototype.changeSize=function(<a>,<b>){' +
      'var <kw>=<a>/this.Width,<kh>=<b>/this.Height;this.setSlideSize(<a>,<b>);' +
      'this.cSld.forEachSp(function(<o>){<o>.changeSize(<kw>,<kh>)})}',
  );

  const exportsOf = (name) => [
    ...bundle.matchAll(
      new RegExp(`AscFormat\\s*\\.\\s*${name}\\s*=\\s*([A-Za-z_$][\\w$]*)`, 'g'),
    ),
  ];
  const changeSizeDefs = [
    ...bundle.matchAll(
      compiledShape(
        '<cls>.prototype.changeSize=function(<kw>,<kh>){' +
          'if(this.spPr&&this.spPr.xfrm&&this.spPr.xfrm.isNotNull()){' +
          'var <x>=this.spPr.xfrm;' +
          '<x>.setOffX(<x>.offX*<kw>);<x>.setOffY(<x>.offY*<kh>);' +
          '<x>.setExtX(<x>.extX*<kw>);<x>.setExtY(<x>.extY*<kh>)}' +
          '(<t>=this.txXfrm)&&<t>.isNotNull()&&(' +
          '<t>.setOffX(<t>.offX*<kw>),<t>.setOffY(<t>.offY*<kh>),' +
          '<t>.setExtX(<t>.extX*<kw>),<t>.setExtY(<t>.extY*<kh>));' +
          'this.recalcTransform&&this.recalcTransform()}',
      ),
    ),
  ];
  const shapeExports = exportsOf('CShape');
  if (changeSizeDefs.length !== 1) {
    problems.push(
      `expected one non-uniform CShape.prototype.changeSize, found ${changeSizeDefs.length}`,
    );
  } else if (
    shapeExports.length !== 1 ||
    shapeExports[0][1] !== changeSizeDefs[0].groups.cls
  ) {
    problems.push('the non-uniform changeSize is not on the class exported as AscFormat.CShape');
  }
  for (const name of ['CImageShape', 'CChartSpace']) {
    const classExports = exportsOf(name);
    if (classExports.length !== 1) {
      problems.push(`expected one AscFormat.${name} export, found ${classExports.length}`);
      continue;
    }
    const exportAt = classExports[0].index;
    const cls = escapeRegExp(classExports[0][1]);
    // The last changeSize assignment on the class before its export must be
    // a copy from a local alias of AscFormat.CShape.
    const assignments = [
      ...bundle
        .slice(0, exportAt)
        .matchAll(new RegExp(`(?<![\\w$.])${cls}\\s*\\.\\s*prototype\\s*\\.\\s*changeSize\\s*=`, 'g')),
    ];
    const last = assignments.at(-1);
    const copy = last && compiledShape('<src>.prototype.changeSize').exec(
      bundle.slice(last.index + last[0].length).replace(/^\s+/, ''),
    );
    if (!copy || copy.index !== 0) {
      problems.push(`${name}.prototype.changeSize is no longer copied from another class`);
      continue;
    }
    const src = escapeRegExp(copy.groups.src);
    const aliases = [
      ...bundle
        .slice(0, last.index)
        .matchAll(new RegExp(`(?<![\\w$.])${src}\\s*=\\s*([\\w$.]+)`, 'g')),
    ];
    if (aliases.at(-1)?.[1] !== 'AscFormat.CShape') {
      problems.push(`${name}.prototype.changeSize is not copied from AscFormat.CShape`);
    }
  }
  if (problems.length > 0) {
    fail(
      `#49 aspect patch guard: slide/sdk-all.js changed around the slide resize path ` +
      `(${problems.join('; ')}). Compare internalChangeSizes and changeSlideSizeFunction ` +
      'in sdkjs slide/Editor/Format/Presentation.js, changeSize in Slide.js and in ' +
      'common/Drawings/Format/Shape.js, Image.js and ChartSpace.js with the previous sdkjs ' +
      'tag. If the code changed, revisit the aspect patch in editor-patches.js before ' +
      'releasing; if only the compiled output changed, update this check.',
    );
  }
}

// A paste from another document brings its images only because sdkjs's desktop
// layer decides "this document's image or another's" by the documentUrl prefix
// the bridge sets per document, and hands the others to LocalFileGetImageUrl,
// which copies them in. If either path changes upstream, pasted images go back
// to resolving to this document's own images of the same name.
for (const moduleName of ['word', 'cell', 'slide']) {
  const bundlePath = path.join(distRoot, 'sdkjs', moduleName, 'sdk-all.js');
  // Missing means nothing was checked: that must not read as a pass.
  if (!existsSync(bundlePath)) {
    fail(
      `cross-document paste guard: sdkjs/${moduleName}/sdk-all.js is missing, ` +
      `so image paste could not be checked`,
    );
    continue;
  }
  const bundle = await readFile(bundlePath, 'utf8');
  const problems = [];
  const expectOne = (label, template) => {
    const found = [...bundle.matchAll(compiledShape(template))].length;
    if (found !== 1) problems.push(`${label} (found ${found})`);
  };
  expectOne(
    'GetObjectsForImageDownload copies only what getImageLocal rejects',
    'function <f>(<list>,<same>){for(var <byUrl>=[],<urls>=[],<i>=0;<i><<list>.length;++<i>){' +
      'var <img>=<list>[<i>],<u>=<img>.Url;' +
      '!<u>||<d>.getImageLocal(<u>)||<d>.isThemeUrl(<u>)||(<urls>.push(<u>),<byUrl>.push([<img>]))}',
  );
  expectOne(
    'the desktop getImageLocal goes by the documentUrl prefix',
    'prot.getImageLocal=function(<u>){<u>=<u>?<u>.replaceAll("%20"," "):"";' +
      'var <p>=this.documentUrl+"/media/";return 0===<u>.indexOf(<p>)?<u>.substring(<p>.length):',
  );
  expectOne(
    'the desktop sendImgUrls asks LocalFileGetImageUrl',
    'AscCommon.sendImgUrls=function(<a>,<b>,<c>){<a>=[];for(var <i>=0;<i><<b>.length;<i>++){' +
      'var <f>=window.AscDesktopEditor.LocalFileGetImageUrl(<b>[<i>]);' +
      '<a>[<i>]={url:AscCommon.g_oDocumentUrls.getUrl(<f>),path:<f>}}<c>(<a>)}',
  );
  expectOne(
    'NativeCorrectImageUrlOnPaste asks LocalFileGetImageUrl',
    'NativeCorrectImageUrlOnPaste=function(<a>){return window.AscDesktopEditor.LocalFileGetImageUrl(<a>)}',
  );
  if (problems.length > 0) {
    fail(
      `cross-document paste guard: ${moduleName}/sdk-all.js changed around image paste ` +
      `(${problems.join('; ')}). Compare GetObjectsForImageDownload in sdkjs ` +
      'common/wordcopypaste.js and the DocumentUrls overrides and sendImgUrls in ' +
      'common/Local/common.js with the previous sdkjs tag. If the logic changed, revisit ' +
      '_eoSetDocumentUrls and LocalFileGetImageUrl in bridge.js; if only the compiled ' +
      'output changed, update this check.',
    );
  }
}

for (const forbidden of [
  'sdkjs/develop',
  'sdkjs/pdf/src/engine/drawingfile_ie.js',
  'sdkjs/common/libfont/engine/fonts_ie.js',
  'sdkjs/common/spell/spell/spell_ie.js',
  'sdkjs/common/spell/spell/spell.js.mem',
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
