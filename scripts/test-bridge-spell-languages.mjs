// Checks that "Add spell check language…" on the start screen knows the
// languages of sdkjs's own table.
//
// The list is staged from the real src/sdkjs/common/spell/spell.js by the
// function the builds call. The bridge then reads it through a fetch that
// answers as the app does: a staged file with its content, anything else with
// the start page.
//
// Run: git submodule update --init --depth 1 src/sdkjs
//      node scripts/test-bridge-spell-languages.mjs [path/to/bridge.js]
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';
import { SPELL_LANGUAGES_FILE, writeSpellLanguages } from './spell-languages.mjs';

const EXPECTED = ['sv_SE', 'fr_FR', 'de_DE', 'en_US'];
const results = [];

const startPage = await readFile(path.join(projectRoot, 'src', 'index.html'), 'utf8');
const frontend = await mkdtemp(path.join(os.tmpdir(), 'eo-spell-languages-'));

function isStaged(url) {
  return existsSync(path.join(frontend, url));
}

async function servedText(url) {
  if (!isStaged(url)) return startPage;
  return readFile(path.join(frontend, url), 'utf8');
}

// Runs "Add spell check language…" with no editor open and returns the codes
// the backend receives, the files the bridge asked for and what it logged.
async function knownOnStartScreen() {
  const imports = [];
  const bridge = await loadBridge({
    invoke: {
      list_user_dictionaries: () => ({ valid: [], skipped: [] }),
      // null is the backend's answer to a cancelled file dialog.
      import_dictionary: (args) => { imports.push(args); return null; },
    },
  });
  const fetched = [];
  bridge.sandbox.fetch = async (url) => {
    fetched.push(url);
    const text = await servedText(url);
    return { text: async () => text };
  };
  await bridge.run('_eoAddLanguage()');
  bridge.stopTimers();
  const known = imports[0] ? Array.from(imports[0].known) : [];
  return { known, fetched, logs: bridge.logs };
}

function missingFrom(codes) {
  return EXPECTED.filter((code) => !codes.includes(code));
}

try {
  const staged = await writeSpellLanguages(frontend);
  check(results, 'the staged list holds sv_SE, fr_FR, de_DE and en_US',
    missingFrom(staged).length === 0, `${staged.length} codes, missing: ${missingFrom(staged).join(', ') || 'none'}`);

  const onDisk = JSON.parse(await readFile(path.join(frontend, SPELL_LANGUAGES_FILE), 'utf8'));
  check(results, 'the staged file is a JSON array of those codes',
    JSON.stringify(onDisk) === JSON.stringify(staged));

  const start = await knownOnStartScreen();
  check(results, 'on the start screen the import knows sv_SE, fr_FR, de_DE and en_US',
    missingFrom(start.known).length === 0,
    `${start.known.length} codes, missing: ${missingFrom(start.known).join(', ') || 'none'}`);
  check(results, 'the import receives every staged code',
    JSON.stringify(start.known) === JSON.stringify(staged));
  check(results, 'the bridge only asks for a file the build stages',
    start.fetched.length === 1 && start.fetched.every(isStaged), JSON.stringify(start.fetched));

  await rm(path.join(frontend, SPELL_LANGUAGES_FILE));
  const without = await knownOnStartScreen();
  check(results, 'without the staged list no code is sent and the log says why',
    without.known.length === 0
    && without.logs.some((line) => line.includes('[SPELL] language table unreadable')),
    JSON.stringify(without.known));
} finally {
  await rm(frontend, { recursive: true, force: true });
}

finish(results);
