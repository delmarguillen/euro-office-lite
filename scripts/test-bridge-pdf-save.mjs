// Checks that Save and Save As do nothing to disk while a PDF is open: the
// user is told saving PDF comes later, and the editor's save ends as not done
// so the document stays modified. A .docx still saves as before.
//
// Run: node scripts/test-bridge-pdf-save.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];

async function save(docType, param) {
  const calls = [];
  const record = (name) => () => { calls.push(name); return 'ok'; };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: record('write_editor_bin'),
      save_file: record('save_file'),
      save_file_as: record('save_file_as'),
      get_current_path: () => 'C:/docs/report.' + (docType === 'pdf' ? 'pdf' : 'docx'),
      recovery_mark_saved: () => null,
    },
  });
  const messages = [];
  const endSaves = [];
  bridge.sandbox.__TAURI__.dialog = {
    save: () => Promise.resolve('C:/docs/copy.docx'),
    message: (text) => { messages.push(text); return Promise.resolve(); },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: (code) => endSaves.push(code),
  };
  await bridge.run(`AscDesktopEditor.LocalFileSave(${JSON.stringify(param)}, '', null, 0, '{}')`);
  bridge.stopTimers();
  return { calls, messages, endSaves, t: (key) => bridge.run(`_t(${JSON.stringify(key)})`) };
}

for (const [label, param] of [['Save', ''], ['Save As', 'saveas=true;']]) {
  const env = await save('pdf', param);
  check(results, `${label} with a PDF open writes nothing`, env.calls.length === 0, env.calls.join());
  check(results, `... says saving PDF comes later`,
    env.messages.length === 1 && env.messages[0] === env.t('pdfSaveLater'), JSON.stringify(env.messages));
  check(results, `... and ends the save as not done`, env.endSaves.join() === '1', env.endSaves.join());
}

const env = await save('word', '');
check(results, 'Save with a .docx open still writes the file',
  env.calls.includes('save_file') && env.messages.length === 0, env.calls.join());

finish(results);
