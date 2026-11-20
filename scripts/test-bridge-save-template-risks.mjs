// Checks what keeps a save from writing the wrong file or the wrong format:
// - with no file open (a new document), Ctrl+S asks where and never writes in
//   place, so it cannot reach a file the document was made from;
// - the bridge sends save_file_as and save_file no format, only the token: the
//   backend takes the format from the destination's name, so the name and the
//   content cannot disagree whatever the editor asked for;
// - whether the window keeps its document after a Save As follows the
//   backend's answer for the name chosen, not the format the editor asked for:
//   a template name typed in a document's Save As is a copy, a document name
//   typed in a template's Save As is the document's new file.
//
// Run: node scripts/test-bridge-save-template-risks.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];

// One save from the editor: `saveAs` is the panel's Save As, otherwise Ctrl+S.
async function save({ docType, current, fileType, saveAs, picked = null, answer = 'ok' }) {
  const calls = { picked: [], inPlace: [], saveAs: [], titles: [], recoverySaved: 0, modified: [] };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => (current ? { token: 't-open', name: current } : null),
      pick_save_target: (args) => { calls.picked.push(args); return picked; },
      save_file: (args) => { calls.inPlace.push(args); return 'ok'; },
      save_file_as: (args) => { calls.saveAs.push(args); return answer; },
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
  bridge.run('_eoMarkUnsavedChanges()');
  const param = saveAs ? 'saveas=true' : '';
  await bridge.run(`AscDesktopEditor.LocalFileSave('${param}', '', null, ${fileType}, '')`);
  bridge.stopTimers();
  calls.isModified = bridge.sandbox.AscDesktopEditor._isModified === true;
  return calls;
}

function onlyToken(args) {
  return JSON.stringify(Object.keys(args || {})) === '["token"]';
}

// A new document: no file open, so Ctrl+S picks a destination.
const families = [
  { docType: 'word', fileType: 65, ext: 'docx' },
  { docType: 'cell', fileType: 257, ext: 'xlsx' },
  { docType: 'slide', fileType: 129, ext: 'pptx' },
];
for (const family of families) {
  const calls = await save({ ...family, current: null, saveAs: false });
  check(results, `Ctrl+S on a new ${family.docType} document asks where`,
    calls.picked.length === 1 && calls.picked[0].requestedExt === family.ext, JSON.stringify(calls.picked));
  check(results, '... and writes nothing in place', calls.inPlace.length === 0, JSON.stringify(calls.inPlace));
}

// The format the editor asks for never reaches the backend.
const flows = [
  { name: 'Save As of a document', docType: 'word', current: 'Letter.docx', fileType: 65, saveAs: true,
    picked: { token: 't-save', name: 'Copy.docx' } },
  { name: 'Save as template', docType: 'cell', current: 'Budget.xlsx', fileType: 262, saveAs: true,
    picked: { token: 't-save', name: 'Budget.xltx' }, answer: 'copy' },
  { name: 'Ctrl+S on an open template', docType: 'slide', current: 'Deck.potx', fileType: 135, saveAs: false,
    picked: { token: 't-save', name: 'Deck.pptx' } },
];
for (const flow of flows) {
  const calls = await save(flow);
  check(results, `${flow.name} sends save_file_as only the destination's token`,
    calls.saveAs.length === 1 && onlyToken(calls.saveAs[0]) && calls.saveAs[0].token === 't-save',
    JSON.stringify(calls.saveAs));
  check(results, '... and writes nothing in place', calls.inPlace.length === 0, JSON.stringify(calls.inPlace));
}

const inPlace = await save({ docType: 'word', current: 'Letter.docx', fileType: 76, saveAs: false });
check(results, 'Ctrl+S in place sends save_file no format',
  inPlace.inPlace.length === 1 && JSON.stringify(inPlace.inPlace[0]) === '{"data":""}',
  JSON.stringify(inPlace.inPlace));

// A template name typed in a document's Save As: the backend writes a template
// beside the document and answers 'copy'.
const typedTemplate = await save({
  docType: 'word', current: 'Letter.docx', fileType: 65, saveAs: true,
  picked: { token: 't-save', name: 'Model.dotx' }, answer: 'copy',
});
check(results, 'a .dotx typed in a .docx Save As keeps the document modified',
  typedTemplate.isModified && !typedTemplate.modified.includes(false), typedTemplate.modified.join());
check(results, '... with its title and its recovery session',
  typedTemplate.titles.length === 0 && typedTemplate.recoverySaved === 0,
  typedTemplate.titles.join() + ' ' + typedTemplate.recoverySaved);

// A document name typed in the panel's template Save As: the backend writes a
// document and the window follows it.
const typedDocument = await save({
  docType: 'word', current: 'Letter.docx', fileType: 76, saveAs: true,
  picked: { token: 't-save', name: 'Model.docx' }, answer: 'ok',
});
check(results, 'a .docx typed in a .dotx Save As is the document\'s save',
  !typedDocument.isModified && typedDocument.recoverySaved === 1,
  typedDocument.modified.join() + ' ' + typedDocument.recoverySaved);
check(results, '... and the title shows the new name',
  typedDocument.titles.join() === 'Model.docx', typedDocument.titles.join());

finish(results);
