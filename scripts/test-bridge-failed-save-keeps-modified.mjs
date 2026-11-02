// Checks that a failed save never leaves the title without its modified mark:
// when the error dialog opens, the JS flag and the last flag sent to Rust (the
// one that draws the mark) already say modified, and they still do after the
// editor's EndSave.
//
// sdkjs calls onDocumentModifiedChanged(false) about 2 ms before it calls
// LocalFileSave (measured on Windows over CDP). The "before" cases follow that
// order; the "during" cases report saved while the backend command runs.
// Also checks that a save that works ends without the mark, that a document
// with no changes does not gain it, and that printing keeps it.
//
// Run: node scripts/test-bridge-failed-save-keeps-modified.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const DOCX = 65;

async function makeBridge({ docType, currentName, saveName, saveFails, savedDuringSave }) {
  const sent = [];
  const atDialog = [];
  let bridge;
  const runSave = () => {
    if (savedDuringSave) bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(false);
    if (saveFails) throw new Error('Permission denied');
    return null;
  };
  bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'tok-current', name: currentName }),
      pick_save_target: () => (saveName ? { token: 'tok-copy', name: saveName } : null),
      recovery_mark_saved: () => null,
      save_file: runSave,
      save_file_as: runSave,
      save_pdf: runSave,
      print_document: () => null,
      open_pdf_viewer: () => null,
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
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: {
      asc_nativeGetFile: () => 'bin',
      getDocumentRenderer: () => ({ Save: () => new Uint8Array([1, 2, 3]) }),
    } },
    // A failed EndSave restores the history index, so sdkjs reports again
    // whether the document had changes; EndSave(0) reports it saved.
    DesktopOfflineAppDocumentEndSave(error) {
      bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(error !== 0 && run.edited);
    },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  const run = { bridge, sent, atDialog, edited: false };
  return run;
}

// What sdkjs does around a save: the user's edit, then the "saved" report
// that comes before LocalFileSave unless the case reports it during the save.
async function startSave(run, { edited, savedDuringSave }, call) {
  const editor = run.bridge.sandbox.AscDesktopEditor;
  run.edited = edited;
  if (edited) editor.onDocumentModifiedChanged(true);
  if (!savedDuringSave) editor.onDocumentModifiedChanged(false);
  await run.bridge.run(call);
  run.bridge.stopTimers();
}

const isModifiedNow = (run) => run.bridge.sandbox.AscDesktopEditor._isModified === true
  && run.sent[run.sent.length - 1] === true;
const isSavedNow = (run) => run.bridge.sandbox.AscDesktopEditor._isModified === false
  && run.sent[run.sent.length - 1] === false;

const paths = [
  { name: 'save', docType: 'word', currentName: 'report.docx', saveName: null, param: '' },
  { name: 'Save As', docType: 'word', currentName: 'report.docx', saveName: 'bad.docx', param: 'saveas=true' },
  { name: 'PDF save', docType: 'pdf', currentName: 'form.pdf', saveName: null, param: '' },
  { name: 'PDF Save As', docType: 'pdf', currentName: 'form.pdf', saveName: 'bad.pdf', param: 'saveas=true' },
];
const saveCall = (p) => `AscDesktopEditor.LocalFileSave('${p.param}', '', null, ${DOCX}, '')`;

for (const order of ['before', 'during']) {
  const savedDuringSave = order === 'during';
  for (const p of paths) {
    const label = `failed ${p.name} (saved ${order})`;
    const run = await makeBridge({ ...p, saveFails: true, savedDuringSave });
    await startSave(run, { edited: true, savedDuringSave }, saveCall(p));
    const dialog = run.atDialog[0] || {};
    check(results, `${label}: the error dialog is shown`, run.atDialog.length === 1, String(run.atDialog.length));
    check(results, `${label}: modified while the dialog is open`,
      dialog.js === true && dialog.rust === true, JSON.stringify(dialog));
    check(results, `${label}: modified after EndSave`, isModifiedNow(run), run.sent.join());
  }
}

for (const p of paths) {
  const run = await makeBridge({ ...p, saveFails: false, savedDuringSave: false });
  await startSave(run, { edited: true, savedDuringSave: false }, saveCall(p));
  check(results, `good ${p.name}: no error dialog`, run.atDialog.length === 0, String(run.atDialog.length));
  check(results, `good ${p.name}: saved after EndSave`, isSavedNow(run), run.sent.join());

  // A later save that fails with no edit in between has nothing to restore.
  const before = run.sent.length;
  run.edited = false;
  run.bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(false);
  await run.bridge.run(saveCall(p));
  run.bridge.stopTimers();
  check(results, `good ${p.name}: no mark gained by saving again`,
    run.bridge.sandbox.AscDesktopEditor._isModified === false && !run.sent.slice(before).includes(true),
    run.sent.slice(before).join());
}

{
  const run = await makeBridge({ docType: 'word', currentName: 'report.docx', saveName: null,
    saveFails: true, savedDuringSave: false });
  await startSave(run, { edited: false, savedDuringSave: false }, saveCall(paths[0]));
  check(results, 'unchanged document: a failed save adds no mark',
    run.bridge.sandbox.AscDesktopEditor._isModified === false && !run.sent.includes(true), run.sent.join());
}

{
  const run = await makeBridge({ docType: 'word', currentName: 'report.docx', saveName: null,
    saveFails: false, savedDuringSave: false });
  await startSave(run, { edited: true, savedDuringSave: false }, 'AscDesktopEditor.Print()');
  check(results, 'print (saved before): modified after EndSave', isModifiedNow(run), run.sent.join());
}

finish(results);
