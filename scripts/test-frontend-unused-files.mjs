// Checks that the production build leaves no source map and no asm.js
// fallback in the frontend, and keeps the files next to them.
//
// Run: node scripts/test-frontend-unused-files.mjs
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { check, finish } from './bridge-test-harness.mjs';
import {
  removeUnusedFrontendFiles,
  sourceMapsIn,
  unusedFrontendFiles,
} from './frontend-unused-files.mjs';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const results = [];

// What the app loads from the folders the unused files sit in.
const kept = [
  'sdkjs/pdf/src/engine/drawingfile.js',
  'sdkjs/pdf/src/engine/drawingfile.wasm',
  'sdkjs/common/libfont/engine/fonts.js',
  'sdkjs/common/libfont/engine/fonts.wasm',
  'sdkjs/common/spell/spell/spell.js',
  'sdkjs/common/spell/spell/spell.wasm',
  'sdkjs/common/zlib/engine/zlib.js',
  'sdkjs/common/zlib/engine/zlib.wasm',
  'web-apps/apps/documenteditor/main/app.js',
  'web-apps/apps/documenteditor/main/code.js',
  'web-apps/apps/spreadsheeteditor/main/app.js',
  'web-apps/apps/spreadsheeteditor/main/code.js',
  'web-apps/apps/presentationeditor/main/app.js',
  'web-apps/apps/presentationeditor/main/code.js',
  'web-apps/apps/pdfeditor/main/app.js',
  'web-apps/apps/pdfeditor/main/code.js',
];

async function writeTree(root, relativePaths) {
  for (const relativePath of relativePaths) {
    const filePath = path.join(root, relativePath);
    await mkdir(path.dirname(filePath), { recursive: true });
    await writeFile(filePath, 'x');
  }
}

// Every file under root, as a sorted list of paths with forward slashes.
async function filesIn(root, folder = '') {
  const files = [];
  const entries = await readdir(path.join(root, folder), { withFileTypes: true });
  for (const entry of entries) {
    const relativePath = folder ? `${folder}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      files.push(...(await filesIn(root, relativePath)));
    } else {
      files.push(relativePath);
    }
  }
  return files.sort();
}

const root = await mkdtemp(path.join(os.tmpdir(), 'eo-frontend-unused-'));
try {
  await writeTree(root, [...kept, ...unusedFrontendFiles]);
  await removeUnusedFrontendFiles(root);
  const left = await filesIn(root);

  const maps = sourceMapsIn(left);
  check(results, 'no source map is left', maps.length === 0, maps.join(', '));
  const fallbacks = left.filter((file) => file.endsWith('_ie.js') || file.endsWith('.js.mem'));
  check(results, 'no asm.js fallback is left', fallbacks.length === 0, fallbacks.join(', '));
  check(results, 'zlib_ie.js is gone', !left.includes('sdkjs/common/zlib/engine/zlib_ie.js'));
  const keptAll = JSON.stringify(left) === JSON.stringify([...kept].sort());
  check(results, 'the files the app loads are kept', keptAll, keptAll ? '' : left.join(', '));

  // A frontend without those files, as a second run finds it.
  await removeUnusedFrontendFiles(root);
  check(results, 'a second run removes nothing more', (await filesIn(root)).length === kept.length);
} finally {
  await rm(root, { recursive: true, force: true });
}

const listed = new Set(unusedFrontendFiles);
for (const editor of ['documenteditor', 'spreadsheeteditor', 'presentationeditor', 'pdfeditor']) {
  for (const name of ['app.js.map', 'code.js.map']) {
    const sourceMap = `web-apps/apps/${editor}/main/${name}`;
    check(results, `${sourceMap} is listed`, listed.has(sourceMap));
  }
}

check(
  results,
  'a source map in any folder is found',
  JSON.stringify(sourceMapsIn(['a/app.js', 'a/app.js.map', 'b/style.css.map', 'map.js'])) ===
    JSON.stringify(['a/app.js.map', 'b/style.css.map']),
);

// The build removes them from BUILD_ROOT before it copies BUILD_ROOT to
// src-dist/, and the validation refuses a frontend that has one.
const buildScript = await readFile(path.join(scriptDir, 'build-frontend-prod.mjs'), 'utf8');
const removal = buildScript.indexOf('await removeUnusedFrontendFiles(buildRoot);');
const staging = buildScript.indexOf("await cp(path.join(buildRoot, 'sdkjs')");
check(results, 'the build removes them before staging src-dist', removal !== -1 && removal < staging);
const validateScript = await readFile(path.join(scriptDir, 'validate-frontend-prod.mjs'), 'utf8');
check(
  results,
  'the validation checks the same list',
  validateScript.includes('...unusedFrontendFiles,') && validateScript.includes('sourceMapsIn(staged)'),
);

finish(results);
