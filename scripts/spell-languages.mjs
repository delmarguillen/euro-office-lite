// Writes the folder names of sdkjs's spell check language table to
// <frontend>/spell-languages.json, which the start screen reads
// (src/bridge.js, _eoSpellLanguages).
//
// The table is defined in src/sdkjs/common/spell/spell.js. The production
// build compiles that file into the sdk-all-min.js bundles and does not copy
// it, so the names are taken here by running the source file.
//
// Run: node scripts/spell-languages.mjs <frontend dir>
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(scriptDir, '..');
const sdkjsSpellJs = path.join(projectRoot, 'src', 'sdkjs', 'common', 'spell', 'spell.js');

export const SPELL_LANGUAGES_FILE = 'spell-languages.json';

// At load spell.js only fills window.AscCommon and declares a constructor, so
// an empty window is all it needs. The context has no require and no process.
function loadSpellTable(source, filename) {
  const window = {};
  window.window = window;
  vm.runInNewContext(source, window, { filename });
  const getLanguages = window.AscCommon && window.AscCommon.spellcheckGetLanguages;
  if (typeof getLanguages !== 'function') {
    throw new Error(`${filename} does not define AscCommon.spellcheckGetLanguages`);
  }
  return getLanguages();
}

// Several language ids share one folder (every Arabic id uses "ar").
function folderNames(table) {
  const names = Object.values(table)
    .map((entry) => entry && entry.name)
    .filter((name) => typeof name === 'string' && name !== '');
  return [...new Set(names)].sort();
}

export async function readSpellLanguages(spellJsPath = sdkjsSpellJs) {
  const source = await readFile(spellJsPath, 'utf8');
  const languages = folderNames(loadSpellTable(source, spellJsPath));
  if (languages.length === 0) {
    throw new Error(`${spellJsPath} lists no languages`);
  }
  return languages;
}

export async function writeSpellLanguages(frontendDir, spellJsPath = sdkjsSpellJs) {
  const languages = await readSpellLanguages(spellJsPath);
  await mkdir(frontendDir, { recursive: true });
  await writeFile(
    path.join(frontendDir, SPELL_LANGUAGES_FILE),
    `${JSON.stringify(languages)}\n`,
    'utf8',
  );
  return languages;
}

async function main() {
  const frontendDir = process.argv[2];
  if (!frontendDir) {
    console.error('usage: spell-languages.mjs <frontend dir>');
    process.exit(1);
  }
  try {
    const languages = await writeSpellLanguages(path.resolve(frontendDir));
    console.log(`[spell-languages] ${languages.length} codes written to ${SPELL_LANGUAGES_FILE}`);
  } catch (error) {
    console.error(`[spell-languages] ERROR: ${error.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
