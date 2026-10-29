// Checks the dialog a save shows when the backend may not write the file (a
// read-only file or folder): Save, Save As and a PDF save say so, and any
// other failure keeps the generic save error.
//
// Run: node scripts/test-bridge-save-denied.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const DENIED = 'save-denied: /home/user/docs/report.docx: the file is read-only';
const OTHER = 'x2t conversion failed (exit code 80): ';

async function setup(docType, failure) {
  const fail = () => { throw failure; };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'tok-current', name: 'report.' + (docType === 'pdf' ? 'pdf' : 'docx') }),
      save_file: fail,
      save_file_as: fail,
      save_pdf: fail,
      pick_save_target: () => ({ token: 'tok-copy', name: 'copy.docx' }),
      recovery_mark_saved: () => null,
    },
  });
  const messages = [];
  const endSaves = [];
  bridge.sandbox.__TAURI__.dialog = {
    save: () => Promise.resolve('/home/user/docs/copy.' + (docType === 'pdf' ? 'pdf' : 'docx')),
    message: (text, options) => { messages.push({ text, title: options && options.title }); return Promise.resolve(); },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: {
      editor: {
        asc_nativeGetFile: () => 'bin',
        getDocumentRenderer: () => ({ Save: () => new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x01]) }),
      },
    },
    DesktopOfflineAppDocumentEndSave: (code) => endSaves.push(code),
  };
  return { bridge, messages, endSaves, t: (key) => bridge.run(`_t(${JSON.stringify(key)})`) };
}

async function save(docType, param, failure) {
  const env = await setup(docType, failure);
  await env.bridge.run(`AscDesktopEditor.LocalFileSave(${JSON.stringify(param)}, '', null, 65, '{}')`);
  env.bridge.stopTimers();
  return env;
}

function showsOnly(env, key) {
  return env.messages.length === 1 && env.messages[0].text === env.t(key)
    && env.messages[0].title === env.t('saveError');
}

const cases = [
  ['Save', 'word', ''],
  ['Save As', 'word', 'saveas=true;'],
  ['a PDF save', 'pdf', ''],
  ['a PDF Save As', 'pdf', 'saveas=true;'],
];

for (const [name, docType, param] of cases) {
  let env = await save(docType, param, DENIED);
  check(results, `${name} refused for permissions says so`, showsOnly(env, 'saveDeniedMsg'),
    JSON.stringify(env.messages));
  check(results, '... and ends the save as failed', env.endSaves.join() === '1', env.endSaves.join());

  env = await save(docType, param, new Error(DENIED));
  check(results, `${name} refused for permissions, thrown as an Error, says so too`,
    showsOnly(env, 'saveDeniedMsg'), JSON.stringify(env.messages));

  env = await save(docType, param, OTHER);
  check(results, `${name} failing for another reason keeps the generic message`,
    showsOnly(env, 'saveErrorMsg'), JSON.stringify(env.messages));
}

const languages = ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'uk', 'zh', 'ja'];
const env = await setup('word', OTHER);
const missing = languages.filter((lang) => {
  const text = env.bridge.run(`_UI_STRINGS[${JSON.stringify(lang)}].saveDeniedMsg`);
  return typeof text !== 'string' || text.length === 0;
});
check(results, 'every language has the permission message', missing.length === 0, missing.join());
env.bridge.stopTimers();

finish(results);
