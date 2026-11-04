// Checks Ctrl+S on a document whose format the app does not write back
// (a .doc, a .dotx, an .xlsm, a .fodt): it goes through Save As with the
// family's format proposed, and save_file is never called. A .docx, .xlsx,
// .odt... is still saved in place. Also checks that _EO_SAVE_IN_PLACE matches
// SAVE_IN_PLACE_EXTENSIONS in src-tauri/src/file_dialogs.rs.
//
// Run: node scripts/test-bridge-save-unwritable-format.mjs [path/to/bridge.js]
import fs from 'node:fs';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];

// One Ctrl+S on the open document `name`, in the `docType` editor.
async function ctrlS({ name, docType, fileType, answer = null }) {
  const calls = { picked: [], savedInPlace: 0, savedTo: [] };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 't-open', name }),
      pick_save_target: (args) => { calls.picked.push(args); return answer; },
      save_file: () => { calls.savedInPlace += 1; return 'ok'; },
      save_file_as: (args) => { calls.savedTo.push(args.token); return 'ok'; },
      recovery_mark_saved: () => null,
      set_window_title: () => null,
    },
  });
  bridge.sandbox.__TAURI__.dialog = { message: () => Promise.resolve() };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: () => {},
  };
  await bridge.run(`AscDesktopEditor.LocalFileSave('', '', null, ${fileType}, '')`);
  bridge.stopTimers();
  return calls;
}

const cases = [
  { name: 'Letter.dotx', docType: 'word', fileType: 76, proposed: 'docx' },
  { name: 'Macros.docm', docType: 'word', fileType: 75, proposed: 'docx' },
  { name: 'Flat.fodt', docType: 'word', fileType: 78, proposed: 'docx' },
  { name: 'Book.xlsm', docType: 'cell', fileType: 261, proposed: 'xlsx' },
  { name: 'Book.XLSB', docType: 'cell', fileType: 264, proposed: 'xlsx' },
  { name: 'Deck.potx', docType: 'slide', fileType: 135, proposed: 'pptx' },
  { name: 'Old.doc', docType: 'word', fileType: 66, proposed: 'docx' },
  { name: 'Sheet.xls', docType: 'cell', fileType: 258, proposed: 'xlsx' },
  { name: 'Talk.PPT', docType: 'slide', fileType: 130, proposed: 'pptx' },
];
for (const c of cases) {
  const calls = await ctrlS(c);
  check(results, `Ctrl+S on ${c.name} opens Save As proposing .${c.proposed}`,
    calls.picked.length === 1 && calls.picked[0].requestedExt === c.proposed, JSON.stringify(calls.picked));
  check(results, `... and does not write ${c.name} in place`, calls.savedInPlace === 0, String(calls.savedInPlace));
}

let calls = await ctrlS({ name: 'Book.xlsm', docType: 'cell', fileType: 261,
  answer: { token: 't-new', name: 'Book.xlsx' } });
check(results, 'the Save As it opens saves with the token it got back',
  calls.savedTo.join() === 't-new' && calls.savedInPlace === 0, calls.savedTo.join());

for (const name of ['Report.docx', 'Book.xlsx', 'Deck.pptx', 'Letter.odt', 'Data.CSV']) {
  calls = await ctrlS({ name, docType: 'word', fileType: 0 });
  check(results, `Ctrl+S on ${name} still saves in place`,
    calls.savedInPlace === 1 && calls.picked.length === 0, JSON.stringify(calls));
}

// The extensions in SAVE_IN_PLACE_EXTENSIONS, read from the Rust source.
function rustSaveInPlaceExtensions() {
  const source = fs.readFileSync(path.join(projectRoot, 'src-tauri/src/file_dialogs.rs'), 'utf8');
  const match = source.match(/const SAVE_IN_PLACE_EXTENSIONS: \[&str; \d+\] = \[([^\]]*)\]/);
  if (!match) return null;
  return [...match[1].matchAll(/"([a-z]+)"/g)].map((m) => m[1]);
}

const bridge = await loadBridge();
const inBridge = JSON.parse(bridge.run('JSON.stringify(_EO_SAVE_IN_PLACE)'));
bridge.stopTimers();
const inRust = rustSaveInPlaceExtensions();
check(results, 'the bridge saves in place exactly the formats Rust does',
  inRust !== null && [...inBridge].sort().join() === [...inRust].sort().join(),
  `bridge: ${inBridge.join()} rust: ${inRust && inRust.join()}`);

const savesInPlace = (name) => bridge.run(`_eoSavesInPlace(${JSON.stringify(name)})`);
for (const name of ['a.doc', 'a.DOC', 'a.xls', 'a.ppt', 'a.dotx', 'a.fodt', 'a']) {
  check(results, `_eoSavesInPlace('${name}') is false`, savesInPlace(name) === false, String(savesInPlace(name)));
}
for (const name of ['a.docx', 'a.XLSX', 'a.pptx', 'a.odt', 'a.pdf']) {
  check(results, `_eoSavesInPlace('${name}') is true`, savesInPlace(name) === true, String(savesInPlace(name)));
}

finish(results);
