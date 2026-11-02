// Checks the Save As that follows a recovery. The backend opens the dialog and
// decides where it starts (src-tauri/src/file_dialogs.rs, with its own tests);
// the bridge tells it the format asked for, saves with the token it gets back,
// and counts only a save to an editable format as a save of the document.
//
// Run: node scripts/test-bridge-save-suggestion.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const PDF = 513;
const DOCX = 65;

// One LocalFileSave with the backend's dialog answering `answer`.
async function saveAs({ current, fileType, answer }) {
  const calls = { picked: [], savedTo: [], markedSaved: 0 };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => current,
      pick_save_target: (args) => { calls.picked.push(args); return answer; },
      save_file_as: (args) => { calls.savedTo.push(args.token); return 'ok'; },
      recovery_mark_saved: () => { calls.markedSaved += 1; return null; },
      set_window_title: () => null,
    },
  });
  bridge.sandbox.__TAURI__.dialog = { message: () => Promise.resolve() };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: () => {},
  };
  await bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${fileType}, '')`);
  bridge.stopTimers();
  return calls;
}

let calls = await saveAs({ current: null, fileType: PDF, answer: { token: 't-pdf', name: 'report.pdf' } });
check(results, 'a PDF export after a recovery asks the backend for a PDF target',
  calls.picked.length === 1 && calls.picked[0].requestedExt === 'pdf', JSON.stringify(calls.picked));
check(results, '... saves with the token it got back',
  calls.savedTo.join() === 't-pdf', calls.savedTo.join());
check(results, '... and does not count as a save of the document',
  calls.markedSaved === 0, String(calls.markedSaved));

calls = await saveAs({ current: null, fileType: DOCX, answer: { token: 't-docx', name: 'report.docx' } });
check(results, 'a Save As after a recovery is a save of the document',
  calls.savedTo.join() === 't-docx' && calls.markedSaved === 1, calls.savedTo.join());

calls = await saveAs({
  current: { token: 't-open', name: 'report.odt' }, fileType: 0, answer: { token: 't-odt', name: 'other.odt' },
});
check(results, 'a plain Save As sends no format of its own',
  calls.picked.length === 1 && calls.picked[0].requestedExt === null, JSON.stringify(calls.picked));

calls = await saveAs({ current: null, fileType: DOCX, answer: null });
check(results, 'a cancelled dialog saves nothing',
  calls.savedTo.length === 0 && calls.markedSaved === 0, calls.savedTo.join());

finish(results);
