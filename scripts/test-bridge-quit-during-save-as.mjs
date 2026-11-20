// Quit while the Save As dialog is open: sdkjs has reported the document as
// saved (it calls onDocumentModifiedChanged(false) as the save begins), the
// backend still counts it as unsaved because the save is pending, and emits
// confirm-close. The page must ask before it calls force_close.
//
// Run: node scripts/test-bridge-quit-during-save-as.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const DOCX = 65;

async function makeRun({ discard }) {
  const commands = [];
  const confirms = [];
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'tok-current', name: 'report.docx' }),
      // The Save As dialog stays open for the whole check.
      pick_save_target: () => { commands.push('pick_save_target'); return new Promise(() => {}); },
      force_close: () => { commands.push('force_close'); return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = {
    message: () => Promise.resolve(),
    confirm: (text) => { confirms.push(text); return Promise.resolve(discard); },
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
  return { bridge, commands, confirms };
}

async function settleAll() {
  for (let i = 0; i < 10; i++) await new Promise((resolve) => setImmediate(resolve));
}

// The user's edit, the "saved" report sdkjs sends as the save begins, then
// Save As, left waiting on its dialog.
async function editThenOpenSaveAs(run) {
  const editor = run.bridge.sandbox.AscDesktopEditor;
  editor.onDocumentModifiedChanged(true);
  editor.onDocumentModifiedChanged(false);
  run.bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${DOCX}, '')`);
  await settleAll();
}

async function quit(run) {
  run.bridge.emit('confirm-close');
  await settleAll();
}

{
  const run = await makeRun({ discard: false });
  await editThenOpenSaveAs(run);
  check(results, 'the Save As dialog is open', run.commands.includes('pick_save_target'), run.commands.join(' '));
  await quit(run);
  check(results, 'Quit with Save As open asks before closing', run.confirms.length === 1, run.confirms.length + ' dialogs');
  check(results, 'answering no keeps the app open', !run.commands.includes('force_close'), run.commands.join(' '));
  run.bridge.stopTimers();
}

{
  const run = await makeRun({ discard: true });
  await editThenOpenSaveAs(run);
  await quit(run);
  check(results, 'answering yes closes', run.commands.includes('force_close'), run.commands.join(' '));
  run.bridge.stopTimers();
}

{
  const run = await makeRun({ discard: false });
  const editor = run.bridge.sandbox.AscDesktopEditor;
  editor.onDocumentModifiedChanged(true);
  editor.onDocumentModifiedChanged(false);
  await quit(run);
  check(results, 'undo back to the saved state, no save running: closes without asking',
    run.confirms.length === 0 && run.commands.includes('force_close'), run.confirms.length + ' dialogs; ' + run.commands.join(' '));
  run.bridge.stopTimers();
}

finish(results);
