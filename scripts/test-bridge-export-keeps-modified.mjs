// Checks that a PDF export and a print leave a modified document modified, in
// the JS and in the flag sent to Rust, while a Save As to an editable format
// still marks it saved. The EndSave fake follows sdkjs (word/Local/api.js):
// a save sdkjs started has already moved the saved index to the current one;
// EndSave(0) keeps it, any other code puts the old one back, and either way the
// editor then reports whether the history differs from the saved index.
//
// Run: node scripts/test-bridge-export-keeps-modified.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const PDF = 513;
const DOCX = 65;

async function modifiedBridge(dialogAnswer) {
  const sent = [];
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      get_current_path: () => '/docs/report.docx',
      save_file_as: () => 'ok',
      recovery_mark_saved: () => null,
      set_window_title: () => null,
      print_document: () => null,
      open_pdf_viewer: () => null,
      set_document_modified: (args) => { sent.push(args.modified); return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = {
    save: () => Promise.resolve(dialogAnswer),
    message: () => Promise.resolve(),
  };
  const history = { index: 5, savedIndex: 2, lastSavedIndex: undefined };
  const editorWindow = {
    endSaveCalls: [],
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave(error) {
      this.endSaveCalls.push(error);
      if (error !== 0) history.savedIndex = history.lastSavedIndex;
      history.lastSavedIndex = undefined;
      bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(history.index !== history.savedIndex);
    },
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = editorWindow;
  bridge.sandbox.AscDesktopEditor._isModified = true;
  // What asc_Save does before it calls LocalFileSave.
  const startSave = () => {
    history.lastSavedIndex = history.savedIndex;
    history.savedIndex = history.index;
  };
  return { bridge, sent, editorWindow, startSave };
}

function isModified(bridge) {
  return bridge.sandbox.AscDesktopEditor._isModified === true;
}

function lastSent(sent) {
  return sent.length ? sent[sent.length - 1] : undefined;
}

let run = await modifiedBridge('/docs/report.pdf');
run.startSave();
await run.bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${PDF}, '')`);
run.bridge.stopTimers();
check(results, 'a PDF export leaves the document modified',
  isModified(run.bridge), String(run.bridge.sandbox.AscDesktopEditor._isModified));
check(results, '... and Rust is left with modified = true',
  lastSent(run.sent) === true, run.sent.join());
check(results, '... and the saving indicator is closed once',
  run.editorWindow.endSaveCalls.length === 1, run.editorWindow.endSaveCalls.join());

run = await modifiedBridge(null);
run.startSave();
await run.bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${DOCX}, '')`);
run.bridge.stopTimers();
check(results, 'a cancelled Save As leaves the document modified',
  isModified(run.bridge) && lastSent(run.sent) === true, run.sent.join());

run = await modifiedBridge(null);
await run.bridge.run(`AscDesktopEditor.Print('')`);
run.bridge.stopTimers();
check(results, 'a print leaves the document modified',
  isModified(run.bridge), String(run.bridge.sandbox.AscDesktopEditor._isModified));
check(results, '... and never sends modified = false',
  !run.sent.includes(false), run.sent.join());

run = await modifiedBridge('/docs/report.docx');
run.startSave();
await run.bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${DOCX}, '')`);
run.bridge.stopTimers();
check(results, 'a Save As to DOCX marks the document saved',
  !isModified(run.bridge) && lastSent(run.sent) === false, run.sent.join());

finish(results);
