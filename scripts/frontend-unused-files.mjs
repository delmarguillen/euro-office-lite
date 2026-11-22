// Files the sdkjs and web-apps builds write that the app never loads. The
// production build removes them (build-frontend-prod.mjs) and the validation
// fails if one is staged (validate-frontend-prod.mjs).
//
// Paths are relative to the frontend root: BUILD_ROOT during the build,
// src-dist/ afterwards.
import { rm } from 'node:fs/promises';
import path from 'node:path';

// The asm.js fallbacks of the WebAssembly engines. sdkjs asks for one when
// window.WebAssembly is missing, and every supported WebView2, WKWebView and
// WebKitGTK runtime provides it. drawingfile_ie.js and fonts_ie.js add about
// 25 MiB, spell_ie.js about 770 KiB and zlib_ie.js about 2.8 MiB.
// spell.js.mem is the startup memory image of spell_ie.js, referenced by
// nothing else.
const asmJsFallbacks = [
  'sdkjs/pdf/src/engine/drawingfile_ie.js',
  'sdkjs/common/libfont/engine/fonts_ie.js',
  'sdkjs/common/spell/spell/spell_ie.js',
  'sdkjs/common/spell/spell/spell.js.mem',
  'sdkjs/common/zlib/engine/zlib_ie.js',
];

// terser writes a source map next to app.js and code.js of each editor, about
// 12 MiB in all. Neither bundle carries a sourceMappingURL comment, so
// nothing requests them.
const editors = [
  'documenteditor',
  'spreadsheeteditor',
  'presentationeditor',
  'pdfeditor',
];
const sourceMaps = editors.flatMap((editor) => [
  `web-apps/apps/${editor}/main/app.js.map`,
  `web-apps/apps/${editor}/main/code.js.map`,
]);

export const unusedFrontendFiles = [...asmJsFallbacks, ...sourceMaps];

export async function removeUnusedFrontendFiles(frontendRoot) {
  for (const relativePath of unusedFrontendFiles) {
    await rm(path.join(frontendRoot, relativePath), { force: true });
  }
}

// Every source map under the frontend root, whatever wrote it.
export function sourceMapsIn(relativePaths) {
  return relativePaths.filter((relativePath) => relativePath.endsWith('.map'));
}
