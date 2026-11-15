// Checks what the backend would decide if the app exited at each step of a
// save: whether the document still has unsaved changes. The fake backend
// keeps the two flags the Rust side keeps (AppState.modified and
// AppState.save_pending) and reads them the way app_quit::has_unsaved_changes
// does; a save command that works clears both, as mark_document_saved does.
//
// sdkjs calls onDocumentModifiedChanged(false) about 2 ms before it calls
// LocalFileSave, so the checks follow that order: change, false, save.
//
// Run: node scripts/test-bridge-exit-during-save.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const DOCX = 65;

// A backend command the check answers when it chooses.
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((ok, fail) => { resolve = ok; reject = fail; });
  return { promise, resolve, reject };
}

async function makeRun({ docType = 'word' } = {}) {
  const backend = { modified: false, savePending: false };
  const pending = { save: null, pick: null, print: null };
  const markSaved = () => { backend.modified = false; backend.savePending = false; };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'tok-current', name: docType === 'pdf' ? 'form.pdf' : 'report.docx' }),
      pick_save_target: () => (pending.pick = deferred()).promise,
      recovery_mark_saved: () => null,
      save_file: () => (pending.save = deferred()).promise.then(markSaved),
      save_file_as: () => (pending.save = deferred()).promise.then(markSaved),
      save_pdf: () => (pending.save = deferred()).promise.then(markSaved),
      print_document: () => (pending.print = deferred()).promise,
      open_pdf_viewer: () => null,
      set_window_title: () => null,
      set_document_modified: (args) => { backend.modified = args.modified; return null; },
      set_save_pending: (args) => { backend.savePending = args.pending; return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = { message: () => Promise.resolve() };
  const run = { bridge, backend, pending, edited: false };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: {
      asc_nativeGetFile: () => 'bin',
      getDocumentRenderer: () => ({ Save: () => new Uint8Array([1, 2, 3]) }),
    } },
    // EndSave(0) reports the document saved; any other code restores the
    // history index, so sdkjs reports again whether it had changes.
    DesktopOfflineAppDocumentEndSave(error) {
      bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(error !== 0 && run.edited);
    },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  return run;
}

const unsaved = (run) => run.backend.modified || run.backend.savePending;
const settle = () => new Promise((resolve) => setImmediate(resolve));

async function settleAll() {
  for (let i = 0; i < 10; i++) await settle();
}

// The user's edit, then the "saved" report sdkjs sends as the save begins.
function editThenBeginSave(run) {
  const editor = run.bridge.sandbox.AscDesktopEditor;
  run.edited = true;
  editor.onDocumentModifiedChanged(true);
  editor.onDocumentModifiedChanged(false);
}

// Starts a bridge call and lets it run up to the backend command the check
// answers. The call's promise comes back wrapped: returned bare, the await
// would wait for the call to end.
async function startCall(run, code) {
  const done = run.bridge.run(code);
  await settleAll();
  return { done };
}

const save = `AscDesktopEditor.LocalFileSave('', '', null, ${DOCX}, '')`;
const saveAs = `AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${DOCX}, '')`;

for (const docType of ['word', 'pdf']) {
  {
    const run = await makeRun({ docType });
    editThenBeginSave(run);
    const { done } = await startCall(run, save);
    check(results, `${docType} save running: unsaved`, unsaved(run), JSON.stringify(run.backend));
    run.pending.save.resolve();
    await done;
    check(results, `${docType} save that works: saved`, !unsaved(run), JSON.stringify(run.backend));
    run.bridge.stopTimers();
  }
  {
    const run = await makeRun({ docType });
    editThenBeginSave(run);
    const { done } = await startCall(run, save);
    run.pending.save.reject(new Error('Permission denied'));
    await done;
    check(results, `${docType} save that fails: unsaved`, unsaved(run), JSON.stringify(run.backend));
    run.bridge.stopTimers();
  }
  {
    const run = await makeRun({ docType });
    editThenBeginSave(run);
    const { done } = await startCall(run, saveAs);
    check(results, `${docType} Save As dialog open: unsaved`, unsaved(run), JSON.stringify(run.backend));
    run.pending.pick.resolve(null);
    await done;
    check(results, `${docType} Save As cancelled: unsaved`, unsaved(run), JSON.stringify(run.backend));
    run.bridge.stopTimers();
  }
}

{
  const run = await makeRun();
  editThenBeginSave(run);
  const { done } = await startCall(run, 'AscDesktopEditor.Print()');
  check(results, 'print running: unsaved', unsaved(run), JSON.stringify(run.backend));
  run.pending.print.resolve();
  await done;
  check(results, 'print done: unsaved', unsaved(run), JSON.stringify(run.backend));
  run.bridge.stopTimers();
}

{
  const run = await makeRun();
  const editor = run.bridge.sandbox.AscDesktopEditor;
  editor.onDocumentModifiedChanged(true);
  editor.onDocumentModifiedChanged(false);
  await settleAll();
  check(results, 'undo back to the saved state: no unsaved changes', !unsaved(run), JSON.stringify(run.backend));
  run.bridge.stopTimers();
}

{
  const run = await makeRun();
  run.bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(false);
  const { done } = await startCall(run, save);
  check(results, 'save with no changes: nothing pending', !unsaved(run), JSON.stringify(run.backend));
  run.pending.save.resolve();
  await done;
  run.bridge.stopTimers();
}

finish(results);
