// Checks that a failed save never leaves the title without its modified mark:
// when the error dialog opens, the JS flag and the last flag sent to Rust (the
// one that draws the mark) already say modified, and they still do after the
// editor's EndSave. The fake editor reports the document as saved while the
// save runs, as sdkjs does once asc_Save has moved its saved index.
//
// Run: node scripts/test-bridge-failed-save-keeps-modified.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const DOCX = 65;

async function failingBridge({ docType, currentName, saveName }) {
  const sent = [];
  const atDialog = [];
  let bridge;
  const reportSavedThenFail = () => {
    bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(false);
    throw new Error('Permission denied');
  };
  bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'tok-current', name: currentName }),
      pick_save_target: () => (saveName ? { token: 'tok-copy', name: saveName } : null),
      recovery_mark_saved: () => null,
      save_file: reportSavedThenFail,
      save_file_as: reportSavedThenFail,
      save_pdf: reportSavedThenFail,
      set_window_title: () => null,
      set_document_modified: (args) => { sent.push(args.modified); return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = {
    message: () => {
      atDialog.push({ js: bridge.sandbox.AscDesktopEditor._isModified, rust: sent[sent.length - 1] });
      return Promise.resolve();
    },
  };
  const editorWindow = {
    Asc: { editor: {
      asc_nativeGetFile: () => 'bin',
      getDocumentRenderer: () => ({ Save: () => new Uint8Array([1, 2, 3]) }),
    } },
    DesktopOfflineAppDocumentEndSave(error) {
      bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(error !== 0);
    },
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = editorWindow;
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._isModified = true;
  return { bridge, sent, atDialog };
}

const cases = [
  { name: 'save', docType: 'word', currentName: 'report.docx', saveName: null, param: '' },
  { name: 'Save As', docType: 'word', currentName: 'report.docx', saveName: 'bad.docx', param: 'saveas=true' },
  { name: 'PDF Save As', docType: 'pdf', currentName: 'form.pdf', saveName: 'bad.pdf', param: 'saveas=true' },
];

for (const c of cases) {
  const run = await failingBridge(c);
  await run.bridge.run(`AscDesktopEditor.LocalFileSave('${c.param}', '', null, ${DOCX}, '')`);
  run.bridge.stopTimers();
  const dialog = run.atDialog[0] || {};
  check(results, `failed ${c.name}: the error dialog is shown`, run.atDialog.length === 1, String(run.atDialog.length));
  check(results, `failed ${c.name}: modified while the dialog is open`,
    dialog.js === true && dialog.rust === true, JSON.stringify(dialog));
  check(results, `failed ${c.name}: modified after EndSave`,
    run.bridge.sandbox.AscDesktopEditor._isModified === true && run.sent[run.sent.length - 1] === true,
    run.sent.join());
}

finish(results);
