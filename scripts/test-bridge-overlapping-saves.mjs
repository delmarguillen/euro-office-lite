// Two saves running at once: save A begins, the user edits and opens Save As
// (B), and A ends while B's dialog is still open. The edit is in no file, so
// the page must still ask before closing, and the backend must still count the
// document as unsaved.
//
// The fake backend keeps what the Rust side keeps (AppState.modified and
// AppState.saves_pending) and reads them as app_quit::has_unsaved_changes
// does. sdkjs calls onDocumentModifiedChanged(false) as each save begins.
//
// Run: node scripts/test-bridge-overlapping-saves.mjs [path/to/bridge.js]
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

async function makeRun() {
  const backend = { modified: false, savesPending: 0 };
  const commands = [];
  const confirms = [];
  // Each command's answers, taken in order; with none left it answers at once.
  const held = { write_editor_bin: [], save_file: [], pick_save_target: [], save_file_as: [] };
  const answer = (command, value) => () => {
    commands.push(command);
    const next = held[command].shift();
    return next ? next.promise : value;
  };
  const bridge = await loadBridge({
    invoke: {
      set_document_modified: ({ modified }) => { backend.modified = modified; },
      set_save_pending: ({ pending }) => { backend.savesPending += pending ? 1 : -1; },
      write_editor_bin: answer('write_editor_bin', 'ok'),
      current_document: () => ({ token: 'tok-current', name: 'report.docx' }),
      save_file: answer('save_file', 'ok'),
      pick_save_target: answer('pick_save_target', null),
      save_file_as: answer('save_file_as', 'ok'),
      force_close: () => { commands.push('force_close'); return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = {
    message: () => Promise.resolve(),
    confirm: (text) => { confirms.push(text); return Promise.resolve(false); },
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
  const backendUnsaved = () => backend.modified || backend.savesPending > 0;
  const pageUnsaved = () => !!bridge.run('_eoHasUnsavedChanges()');
  return { bridge, backend, commands, confirms, held, backendUnsaved, pageUnsaved };
}

async function settleAll() {
  for (let i = 0; i < 10; i++) await new Promise((resolve) => setImmediate(resolve));
}

// The user's edit, then the "saved" report sdkjs sends as the save begins.
async function editThenSave(run, param) {
  const editor = run.bridge.sandbox.AscDesktopEditor;
  editor.onDocumentModifiedChanged(true);
  editor.onDocumentModifiedChanged(false);
  run.bridge.run(`AscDesktopEditor.LocalFileSave('${param}', '', null, ${DOCX}, '')`);
  await settleAll();
}

async function quit(run) {
  run.bridge.emit('confirm-close');
  await settleAll();
}

function checkBothUnsaved(run, when) {
  check(results, when + ': the page counts the document as unsaved', run.pageUnsaved());
  check(results, when + ': the backend counts the document as unsaved', run.backendUnsaved(),
    JSON.stringify(run.backend));
}

// A is written while B's dialog is open.
{
  const run = await makeRun();
  const saveA = deferred();
  run.held.save_file.push(saveA);
  run.held.pick_save_target.push(deferred());
  await editThenSave(run, '');
  await editThenSave(run, 'saveas=true');
  check(results, 'A is being written and the Save As dialog of B is open',
    run.commands.includes('save_file') && run.commands.includes('pick_save_target'), run.commands.join(' '));
  saveA.resolve('ok');
  await settleAll();
  checkBothUnsaved(run, 'A ended, B open');
  await quit(run);
  check(results, 'A ended, B open: Quit asks before closing', run.confirms.length === 1, run.confirms.length + ' dialogs');
  check(results, 'A ended, B open: answering no keeps the app open', !run.commands.includes('force_close'), run.commands.join(' '));
  run.bridge.stopTimers();
}

// A is written before B reaches its dialog, and B is then cancelled: B began
// with the edit unsaved, so cancelling it leaves the document modified.
{
  const run = await makeRun();
  const saveA = deferred();
  const writeB = deferred();
  run.held.save_file.push(saveA);
  await editThenSave(run, '');
  run.held.write_editor_bin.push(writeB);
  await editThenSave(run, 'saveas=true');
  saveA.resolve('ok');
  await settleAll();
  checkBothUnsaved(run, 'A ended, B not at its dialog yet');
  writeB.resolve('ok');
  await settleAll();
  check(results, 'B reached its dialog and was cancelled', run.commands.includes('pick_save_target'), run.commands.join(' '));
  checkBothUnsaved(run, 'A ended, B cancelled');
  await quit(run);
  check(results, 'A ended, B cancelled: Quit asks before closing', run.confirms.length === 1, run.confirms.length + ' dialogs');
  run.bridge.stopTimers();
}

// A fails while B's dialog is open.
{
  const run = await makeRun();
  const saveA = deferred();
  run.held.save_file.push(saveA);
  run.held.pick_save_target.push(deferred());
  await editThenSave(run, '');
  await editThenSave(run, 'saveas=true');
  saveA.reject(new Error('disk full'));
  await settleAll();
  checkBothUnsaved(run, 'A failed, B open');
  run.bridge.stopTimers();
}

// Both saves end well: nothing is left unsaved on either side.
{
  const run = await makeRun();
  const saveA = deferred();
  const pickB = deferred();
  run.held.save_file.push(saveA);
  run.held.pick_save_target.push(pickB);
  await editThenSave(run, '');
  await editThenSave(run, 'saveas=true');
  saveA.resolve('ok');
  await settleAll();
  pickB.resolve({ token: 'tok-b', name: 'copy.docx' });
  await settleAll();
  check(results, 'A and B ended well: B was written', run.commands.includes('save_file_as'), run.commands.join(' '));
  check(results, 'A and B ended well: the page counts nothing as unsaved', !run.pageUnsaved());
  check(results, 'A and B ended well: the backend counts nothing as unsaved', !run.backendUnsaved(),
    JSON.stringify(run.backend));
  await quit(run);
  check(results, 'A and B ended well: Quit closes without asking',
    run.confirms.length === 0 && run.commands.includes('force_close'), run.confirms.length + ' dialogs; ' + run.commands.join(' '));
  run.bridge.stopTimers();
}

finish(results);
