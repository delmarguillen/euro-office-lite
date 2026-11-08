// Checks a file that is open in another window: the backend answers or
// rejects with 'open-elsewhere', and the bridge shows its own warning instead
// of opening it. With a document, the document stays as it is; on the start
// screen (Open, Edit PDF), the start screen stays. A launch argument takes the
// start screen's Open (editor-patches.js, not loaded here).
//
// Run: node scripts/test-bridge-open-elsewhere.mjs [path/to/bridge.js]
import { check, finish, loadBridge, pendingQueue } from './bridge-test-harness.mjs';

const results = [];
const OTHER = { token: 'tok-other', name: 'other.docx' };
const LANGUAGES = ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'uk', 'zh', 'ja'];

function openElsewhere() {
  throw 'open-elsewhere';
}

async function setup({ docType = 'word', pending = null } = {}) {
  const calls = { newWindow: 0, open: 0 };
  const bridge = await loadBridge({
    invoke: {
      pick_document: () => OTHER,
      open_in_new_window: () => { calls.newWindow += 1; return 'open-elsewhere'; },
      open_file: () => { calls.open += 1; return openElsewhere(); },
      take_pending_open_file: pendingQueue(pending),
      recovery_end: () => null,
      set_document_modified: () => null,
    },
  });
  const env = { bridge, calls, messages: [], reloads: 0, editors: [], sameWindow: [] };
  bridge.sandbox.__TAURI__.dialog = {
    confirm: () => Promise.resolve(true),
    message: (text, options) => {
      env.messages.push({ text, title: options && options.title });
      return Promise.resolve();
    },
  };
  bridge.sandbox.location = { reload: () => { env.reloads += 1; } };
  bridge.sandbox._eoOpenPath = (file) => { env.sameWindow.push(file); return Promise.resolve(); };
  bridge.sandbox._openEditor = (type) => { env.editors.push(type); };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  env.t = (key) => bridge.run(`_t(${JSON.stringify(key)})`);
  env.pendingOpen = () => bridge.sandbox.localStorage.getItem('eo-pending-open');
  env.pendingData = () => bridge.sandbox._pendingFileData;
  return env;
}

async function settle(env) {
  await new Promise((resolve) => setTimeout(resolve, 20));
  env.bridge.stopTimers();
}

function warnedOnce(env) {
  return env.messages.length === 1
    && env.messages[0].text === env.t('openElsewhereMsg')
    && env.messages[0].title === env.t('openFailed');
}

{
  const env = await setup();
  await env.bridge.run('AscDesktopEditor.LocalFileOpen()');
  await settle(env);
  check(results, 'Open with a document, file open elsewhere: warning, no reload',
    env.calls.newWindow === 1 && warnedOnce(env) && env.reloads === 0
      && env.pendingOpen() === null && env.sameWindow.length === 0,
    `newWindow=${env.calls.newWindow} messages=${JSON.stringify(env.messages)} reloads=${env.reloads}`);
}

{
  const env = await setup({ docType: null });
  await env.bridge.run(`_eoOpenGranted(${JSON.stringify(OTHER)})`);
  await settle(env);
  check(results, 'Start screen, file open elsewhere: warning, stays on the start screen',
    env.calls.open === 1 && warnedOnce(env) && env.editors.length === 0
      && env.pendingData() === undefined,
    `open=${env.calls.open} editors=${env.editors.length} messages=${JSON.stringify(env.messages)}`);
}

{
  // The bridge asks for the launch argument by itself when it loads.
  const env = await setup({ docType: null, pending: OTHER });
  await settle(env);
  check(results, 'Launch argument (double click): the start screen\'s Open, once',
    env.sameWindow.length === 1 && env.sameWindow[0].token === OTHER.token
      && env.calls.newWindow === 0,
    `sameWindow=${JSON.stringify(env.sameWindow)} newWindow=${env.calls.newWindow}`);
}

{
  const env = await setup({ docType: null });
  await env.bridge.run('_eoShowOpenError("x2t failed")');
  check(results, 'Any other open error keeps the damaged-file message',
    env.messages.length === 1 && env.messages[0].text === env.t('openErrorMsg'),
    JSON.stringify(env.messages));
}

{
  const env = await setup();
  const missing = LANGUAGES.filter((code) =>
    !env.bridge.run(`_UI_STRINGS[${JSON.stringify(code)}].openElsewhereMsg`));
  check(results, 'The open-elsewhere text exists in every language',
    missing.length === 0, missing.join(' '));
}

finish(results);
