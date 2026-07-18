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

for (const forbidden of [
  'sdkjs/develop',
  'web-apps/vendor/less',
]) {
  if (existsSync(path.join(distRoot, forbidden))) {
    fail(`development-only directory was staged: ${forbidden}`);
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
    '../../../../sdkjs/vendor/polyfill.js',
    '../../../../sdkjs/common/AllFonts.js',
    `../../../../sdkjs/${moduleName}/sdk-all-min.js`,
    `../../../../sdkjs/${moduleName}/sdk-all.js`,
  ]) {
    if (!html.includes(expected)) {
      fail(`${relativePath} does not load ${expected}`);
    }
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
  console.log(
    `[frontend-prod] validated ${files.toLocaleString('en-US')} files, ${(bytes / 1024 / 1024).toFixed(1)} MiB`,
  );
}

if (process.exitCode) process.exit(process.exitCode);
