// Checks that Save and Save As with a PDF open hand the PDF editor's changes
// to save_pdf, and that a cancelled or failed save ends as not done. A .docx
// still saves as before.
//
// Run: node scripts/test-bridge-pdf-save.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
// "%PDF" and one byte: what the PDF editor's Save() returns, in small.
const CHANGES = new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x01]);
const CHANGES_B64 = Buffer.from(CHANGES).toString('base64');

async function setup(docType, { picked = null, saveFails = false } = {}) {
  const calls = [];
  const record = (name, result = 'ok') => (args) => {
    calls.push({ name, args });
    if (name === 'save_pdf' && saveFails) throw new Error('x2t conversion failed (exit code 89): ');
    return result;
  };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: record('write_editor_bin'),
      save_file: record('save_file'),
      save_file_as: record('save_file_as'),
      save_pdf: record('save_pdf'),
      get_current_path: () => 'C:/docs/report.' + (docType === 'pdf' ? 'pdf' : 'docx'),
      recovery_mark_saved: () => null,
    },
  });
  const messages = [];
  const endSaves = [];
  const dialogs = [];
  bridge.sandbox.__TAURI__.dialog = {
    save: (options) => { dialogs.push(options); return Promise.resolve(picked); },
    message: (text) => { messages.push(text); return Promise.resolve(); },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: {
      editor: {
        asc_nativeGetFile: () => 'bin',
        getDocumentRenderer: () => ({ Save: () => CHANGES }),
      },
    },
    DesktopOfflineAppDocumentEndSave: (code) => endSaves.push(code),
  };
  const names = () => calls.map((call) => call.name).join();
  const t = (key) => bridge.run(`_t(${JSON.stringify(key)})`);
  return { bridge, calls, names, messages, endSaves, dialogs, t };
}

async function save(docType, param, options) {
  const env = await setup(docType, options);
  await env.bridge.run(`AscDesktopEditor.LocalFileSave(${JSON.stringify(param)}, '', null, 0, '{}')`);
  env.bridge.stopTimers();
  return env;
}

let env = await save('pdf', '');
const saved = env.calls.find((call) => call.name === 'save_pdf');
check(results, 'Save with a PDF open calls save_pdf and nothing else', env.names() === 'save_pdf', env.names());
check(results, '... with the editor\'s changes and no path',
  saved && saved.args.data === CHANGES_B64 && saved.args.path === null, JSON.stringify(saved && saved.args));
check(results, '... and ends the save as done', env.endSaves.join() === '0' && env.messages.length === 0,
  env.endSaves.join() + ' ' + JSON.stringify(env.messages));

env = await save('pdf', 'saveas=true;', { picked: 'C:/docs/copy' });
const savedAs = env.calls.find((call) => call.name === 'save_pdf');
check(results, 'Save As with a PDF open writes to the picked path as a .pdf',
  env.names() === 'save_pdf' && savedAs.args.path === 'C:/docs/copy.pdf' && savedAs.args.data === CHANGES_B64,
  JSON.stringify(savedAs && savedAs.args));
check(results, '... the dialog offers PDF only',
  env.dialogs.length === 1
    && JSON.stringify(env.dialogs[0].filters) === JSON.stringify([{ name: 'PDF', extensions: ['pdf'] }]),
  JSON.stringify(env.dialogs));
check(results, '... and ends the save as done', env.endSaves.join() === '0', env.endSaves.join());

env = await save('pdf', 'saveas=true;', { picked: null });
check(results, 'a cancelled Save As with a PDF open writes nothing', env.calls.length === 0, env.names());
check(results, '... and ends the save as not done', env.endSaves.join() === '1', env.endSaves.join());

env = await save('pdf', '', { saveFails: true });
check(results, 'a failed PDF save says so',
  env.messages.length === 1 && env.messages[0] === env.t('saveErrorMsg'), JSON.stringify(env.messages));
check(results, '... and ends the save as not done', env.endSaves.join() === '1', env.endSaves.join());

env = await save('word', '');
check(results, 'Save with a .docx open still writes the Editor.bin and saves it',
  env.names() === 'write_editor_bin,save_file' && env.messages.length === 0, env.names());

finish(results);
