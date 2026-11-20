// Checks Save As from the editor's format panel with a template format: the
// dialog is asked for that template's extension, and what happens next follows
// the backend's answer. 'copy' (the template was written next to the open
// document) leaves the document modified, with its title and its recovery
// session; 'ok' (the open template was written over itself) is a save.
//
// Run: node scripts/test-bridge-save-template.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];

async function saveAs({ docType, currentName, fileType, savedName, answer }) {
  const calls = { picked: [], titles: [], recoverySaved: 0, modified: [] };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 't-open', name: currentName }),
      pick_save_target: (args) => { calls.picked.push(args); return { token: 't-save', name: savedName }; },
      save_file_as: () => answer,
      recovery_mark_saved: () => { calls.recoverySaved += 1; return null; },
      set_window_title: (args) => { calls.titles.push(args.name); return null; },
      set_document_modified: (args) => { calls.modified.push(args.modified); return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = { message: () => Promise.resolve() };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  // As sdkjs leaves it once asc_Save has started: the saved index moved to the
  // current one, and EndSave puts the old one back unless its code is 0.
  const history = { index: 5, savedIndex: 5, lastSavedIndex: 2 };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave(error) {
      if (error !== 0) history.savedIndex = history.lastSavedIndex;
      bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(history.index !== history.savedIndex);
    },
  };
  bridge.sandbox.AscDesktopEditor._isModified = true;
  await bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${fileType}, '')`);
  bridge.stopTimers();
  calls.isModified = bridge.sandbox.AscDesktopEditor._isModified === true;
  return calls;
}

const families = [
  { docType: 'word', currentName: 'Letter.docx', fileType: 76, ext: 'dotx' },
  { docType: 'cell', currentName: 'Budget.xlsx', fileType: 262, ext: 'xltx' },
  { docType: 'slide', currentName: 'Deck.pptx', fileType: 135, ext: 'potx' },
];

for (const family of families) {
  const calls = await saveAs({ ...family, savedName: 'Model.' + family.ext, answer: 'copy' });
  const picked = calls.picked[0] || {};
  check(results, `format ${family.fileType} asks the dialog for .${family.ext}`,
    picked.requestedExt === family.ext, JSON.stringify(picked));
  check(results, '... with that extension as the only filter',
    JSON.stringify((picked.filters || []).map((filter) => filter.extensions)) === JSON.stringify([[family.ext]]),
    JSON.stringify(picked.filters));
  check(results, '... and a copy leaves the document modified',
    calls.isModified && !calls.modified.includes(false), calls.modified.join());
  check(results, '... with its title and its recovery session',
    calls.titles.length === 0 && calls.recoverySaved === 0,
    calls.titles.join() + ' ' + calls.recoverySaved);
}

const own = await saveAs({
  docType: 'word', currentName: 'Letter.dotx', fileType: 76, savedName: 'Letter.dotx', answer: 'ok',
});
check(results, 'the open template written over itself is a save',
  !own.isModified && own.recoverySaved === 1, own.modified.join() + ' ' + own.recoverySaved);
check(results, '... and the title shows its name',
  own.titles.join() === 'Letter.dotx', own.titles.join());

finish(results);
