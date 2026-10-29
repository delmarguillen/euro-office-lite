// Checks that a PDF reads as unmodified right after it opens. The PDF editor
// takes its modified mark from a plain History.Have_Changes(), and the desktop
// version of that function (src/sdkjs/word/Local/api.js, run here as it is)
// counts an untouched history as a change when the file was never saved. The
// other editors call it with the flag that skips that check, so they are
// unaffected either way.
//
// Run: node scripts/test-bridge-pdf-not-modified-on-open.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, loadSdkjsSnippet, projectRoot } from './bridge-test-harness.mjs';

const results = [];

async function setup(docType) {
  const bridge = await loadBridge();
  bridge.sandbox.AscCommon = { CHistory: function() {} };
  await loadSdkjsSnippet(bridge, 'src/sdkjs/word/Local/api.js',
    'AscCommon.CHistory.prototype.Have_Changes = function(IsNotUserSave, IsNoSavedNoModifyed)',
    '\n};');
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  // What the history looks like once a file is open and nothing was touched:
  // no points, no save yet, user-save mode on as the bridge sets it.
  bridge.run(`var history = new AscCommon.CHistory();
    history.Index = -1; history.UserSavedIndex = null; history.SavedIndex = null;
    history.ForceSave = false; history.Is_UserSaveMode = function() { return true; };`);
  return bridge;
}

let bridge = await setup('pdf');
check(results, 'an untouched PDF has no changes', bridge.run('history.Have_Changes()') === false);
check(results, '... the file counts as saved', bridge.run('AscDesktopEditor.LocalFileGetSaved()') === true);
bridge.run('history.Index = 0;');
check(results, 'a PDF with an edit has changes', bridge.run('history.Have_Changes()') === true);
bridge.stopTimers();

for (const docType of ['word', 'cell', 'slide']) {
  bridge = await setup(docType);
  check(results, `${docType}: the modified check its editor makes is unchanged`,
    bridge.run('history.Have_Changes(undefined, true)') === false);
  check(results, `${docType}: the saved answer stays false`,
    bridge.run('AscDesktopEditor.LocalFileGetSaved()') === false);
  bridge.stopTimers();
}

// The case above only holds while the PDF editor asks this way.
const pdfApi = await readFile(path.join(projectRoot, 'src/sdkjs/pdf/api.js'), 'utf8');
check(results, 'the PDF editor still takes its mark from a plain Have_Changes()',
  pdfApi.includes('true === AscCommon.History.Have_Changes() ||'));

finish(results);
