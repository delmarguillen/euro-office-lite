// Checks the dialog a save shows when the backend refuses to write over a file
// that is open in another window: Save As, a PDF export and a PDF Save As say
// so, the document stays open and modified, and any other failure keeps its
// own message. Also checks every language has the text, and that the German
// and French "open elsewhere" text points at the other window.
//
// Run: node scripts/test-bridge-save-elsewhere.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const ELSEWHERE = 'open-elsewhere';
const DENIED = 'save-denied: /home/user/docs/copy.docx: the file is read-only';
const OTHER = 'x2t conversion failed (exit code 80): ';
const DOCX = 65;
const PDF = 513;

async function setup({ docType, currentName, saveName, failure }) {
  const sent = [];
  const calls = [];
  const messages = [];
  let bridge;
  const fail = (name) => () => {
    calls.push(name);
    // sdkjs reports the document as saved while the save runs.
    bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(false);
    throw failure;
  };
  bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'tok-current', name: currentName }),
      pick_save_target: () => ({ token: 'tok-copy', name: saveName }),
      save_file: fail('save_file'),
      save_file_as: fail('save_file_as'),
      save_pdf: fail('save_pdf'),
      recovery_mark_saved: () => { calls.push('recovery_mark_saved'); return null; },
      set_window_title: (args) => { calls.push('set_window_title ' + JSON.stringify(args)); return null; },
      set_document_modified: (args) => { sent.push(args.modified); return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = {
    message: (text, options) => { messages.push({ text, title: options && options.title }); return Promise.resolve(); },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: {
      asc_nativeGetFile: () => 'bin',
      getDocumentRenderer: () => ({ Save: () => new Uint8Array([0x25, 0x50, 0x44, 0x46, 0x01]) }),
    } },
    // A failed EndSave restores the history index, so sdkjs reports the edit again.
    DesktopOfflineAppDocumentEndSave: (code) => {
      bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(code !== 0);
    },
  };
  return { bridge, sent, calls, messages, t: (key) => bridge.run(`_t(${JSON.stringify(key)})`) };
}

async function saveAs(path, failure) {
  const env = await setup({ ...path, failure });
  env.bridge.sandbox.AscDesktopEditor.onDocumentModifiedChanged(true);
  await env.bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true;', '', null, ${path.fileType}, '{}')`);
  env.bridge.stopTimers();
  return env;
}

function showsOnly(env, key) {
  return env.messages.length === 1 && env.messages[0].text === env.t(key)
    && env.messages[0].title === env.t('saveError');
}

function isModifiedNow(env) {
  return env.bridge.sandbox.AscDesktopEditor._isModified === true
    && env.sent[env.sent.length - 1] === true;
}

// The document only moves after a save that worked: the recovery session
// follows it and the title shows the new name.
function documentStayed(env) {
  return !env.calls.includes('recovery_mark_saved')
    && !env.calls.some((call) => call.indexOf('copy.') !== -1 && call.indexOf('set_window_title') === 0);
}

const paths = [
  { name: 'Save As', docType: 'word', currentName: 'report.docx', saveName: 'copy.docx', fileType: DOCX, command: 'save_file_as' },
  { name: 'a PDF export', docType: 'word', currentName: 'report.docx', saveName: 'copy.pdf', fileType: PDF, command: 'save_file_as' },
  { name: 'a PDF Save As', docType: 'pdf', currentName: 'form.pdf', saveName: 'copy.pdf', fileType: PDF, command: 'save_pdf' },
];

for (const path of paths) {
  let env = await saveAs(path, ELSEWHERE);
  check(results, `${path.name} over a file open in another window reaches ${path.command}`,
    env.calls.includes(path.command), env.calls.join());
  check(results, `${path.name} over a file open in another window says so`,
    showsOnly(env, 'saveElsewhereMsg'), JSON.stringify(env.messages));
  check(results, '... and leaves the document modified', isModifiedNow(env), env.sent.join());
  check(results, '... and open where it was', documentStayed(env), env.calls.join());

  env = await saveAs(path, DENIED);
  check(results, `${path.name} refused for permissions keeps its message`,
    showsOnly(env, 'saveDeniedMsg'), JSON.stringify(env.messages));

  env = await saveAs(path, OTHER);
  check(results, `${path.name} failing for another reason keeps the generic message`,
    showsOnly(env, 'saveErrorMsg'), JSON.stringify(env.messages));
}

const languages = ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'uk', 'zh', 'ja'];
const env = await setup({ docType: 'word', currentName: 'report.docx', saveName: 'copy.docx', failure: OTHER });
const text = (lang, key) => env.bridge.run(`_UI_STRINGS[${JSON.stringify(lang)}][${JSON.stringify(key)}]`);
const missing = languages.filter((lang) => {
  const message = text(lang, 'saveElsewhereMsg');
  return typeof message !== 'string' || message.length === 0;
});
check(results, 'every language has the open-elsewhere save message', missing.length === 0, missing.join());

const english = text('en', 'saveElsewhereMsg');
const untranslated = languages.filter((lang) => lang !== 'en' && text(lang, 'saveElsewhereMsg') === english);
check(results, '... translated, not the English text', untranslated.length === 0, untranslated.join());

check(results, 'German open-elsewhere text names the other window, not this one',
  text('de', 'openElsewhereMsg').indexOf('diesem Fenster') === -1, text('de', 'openElsewhereMsg'));
check(results, 'French open-elsewhere text names the other window, not this one',
  text('fr', 'openElsewhereMsg').indexOf('cette fenêtre') === -1, text('fr', 'openElsewhereMsg'));
env.bridge.stopTimers();

finish(results);
