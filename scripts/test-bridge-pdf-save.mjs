// Checks that Save and Save As with a PDF open hand the PDF editor's changes
// to save_pdf, that a cancelled or failed save ends as not done, and that
// Print with a PDF open prints it with its changes. A .docx still saves and
// prints as before.
//
// Run: node scripts/test-bridge-pdf-save.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
// "%PDF" and one byte: what the PDF editor's Save() returns, in small.
const CHANGES = new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x01]);
const CHANGES_B64 = Buffer.from(CHANGES).toString('base64');

async function setup(docType, { picked = null, saveFails = false } = {}) {
  const calls = [];
  const dialogs = [];
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
      print_document: record('print_document', '/tmp/print_output.pdf'),
      open_pdf_viewer: record('open_pdf_viewer'),
      current_document: () => ({ token: 'tok-current', name: 'report.' + (docType === 'pdf' ? 'pdf' : 'docx') }),
      pick_save_target: (args) => { dialogs.push(args); return picked; },
      recovery_mark_saved: () => null,
    },
  });
  const messages = [];
  const endSaves = [];
  bridge.sandbox.__TAURI__.dialog = {
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

async function print(docType) {
  const env = await setup(docType);
  await env.bridge.run("AscDesktopEditor.Print('')");
  env.bridge.stopTimers();
  return env;
}

let env = await save('pdf', '');
const saved = env.calls.find((call) => call.name === 'save_pdf');
check(results, 'Save with a PDF open calls save_pdf and nothing else', env.names() === 'save_pdf', env.names());
check(results, '... with the editor\'s changes and no token',
  saved && saved.args.data === CHANGES_B64 && saved.args.token === null, JSON.stringify(saved && saved.args));
check(results, '... and ends the save as done', env.endSaves.join() === '0' && env.messages.length === 0,
  env.endSaves.join() + ' ' + JSON.stringify(env.messages));

env = await save('pdf', 'saveas=true;', { picked: { token: 'tok-copy', name: 'copy.pdf' } });
const savedAs = env.calls.find((call) => call.name === 'save_pdf');
check(results, 'Save As with a PDF open writes where the picked token points',
  env.names() === 'save_pdf' && savedAs.args.token === 'tok-copy' && savedAs.args.data === CHANGES_B64
    && !('path' in savedAs.args),
  JSON.stringify(savedAs && savedAs.args));
check(results, '... the dialog offers PDF only',
  env.dialogs.length === 1
    && JSON.stringify(env.dialogs[0].filters) === JSON.stringify([{ name: 'PDF', extensions: ['pdf'] }])
    && env.dialogs[0].requestedExt === 'pdf',
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

env = await print('pdf');
const printed = env.calls.find((call) => call.name === 'print_document');
check(results, 'Print with a PDF open prints it with the editor\'s changes',
  env.names() === 'print_document,open_pdf_viewer' && printed && (printed.args || {}).pdfChanges === CHANGES_B64,
  env.names() + ' ' + JSON.stringify(printed && printed.args));

env = await print('word');
const printedDocx = env.calls.find((call) => call.name === 'print_document');
check(results, 'Print with a .docx open still goes through the Editor.bin',
  env.names() === 'write_editor_bin,print_document,open_pdf_viewer' && printedDocx && !(printedDocx.args || {}).pdfChanges,
  env.names());

finish(results);
