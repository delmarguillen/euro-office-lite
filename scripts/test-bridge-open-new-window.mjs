// Checks File > Open and Recent with a document already open: the file opens
// in a new window through open_in_new_window, and the open document is neither
// reloaded nor asked about. Without a document, Open keeps the start screen's
// path in the same window.
//
// Run: node scripts/test-bridge-open-new-window.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const OTHER = { token: 'tok-other', name: 'other.docx' };
const RECENT = { path: '/home/u/Documents/recent.docx', token: 'tok-recent', name: 'recent.docx' };

async function setup({ docType = 'word', modified = false, newWindow = () => 'opened' } = {}) {
  const calls = [];
  const bridge = await loadBridge({
    invoke: {
      pick_document: () => OTHER,
      recent_files_state: () => ({ enabled: true, files: [RECENT] }),
      open_in_new_window: (args) => { calls.push(args); return newWindow(args); },
      recovery_end: () => null,
      set_document_modified: () => null,
    },
  });
  const env = { bridge, calls, confirms: 0, messages: [], reloads: 0, sameWindow: [] };
  bridge.sandbox.__TAURI__.dialog = {
    confirm: () => { env.confirms += 1; return Promise.resolve(true); },
    message: (text, options) => {
      env.messages.push({ text, title: options && options.title });
      return Promise.resolve();
    },
  };
  bridge.sandbox.location = { reload: () => { env.reloads += 1; } };
  bridge.sandbox._eoOpenPath = (file) => { env.sameWindow.push(file); return Promise.resolve(); };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._isModified = modified;
  env.t = (key) => bridge.run(`_t(${JSON.stringify(key)})`);
  env.pendingOpen = () => bridge.sandbox.localStorage.getItem('eo-pending-open');
  return env;
}

// The reload waits on its invoke round trips; let them settle before looking.
async function settle(env) {
  await new Promise((resolve) => setTimeout(resolve, 20));
  env.bridge.stopTimers();
}

async function open(env) {
  await env.bridge.run('AscDesktopEditor.LocalFileOpen()');
  await settle(env);
  return env;
}

function openedInNewWindow(env, token) {
  return env.calls.length === 1 && env.calls[0].token === token;
}

function leftDocumentAlone(env) {
  return env.reloads === 0 && env.confirms === 0 && env.pendingOpen() === null
    && env.sameWindow.length === 0;
}

{
  const env = await open(await setup());
  check(results, 'Open with a document: new window, no reload, no question',
    openedInNewWindow(env, OTHER.token) && leftDocumentAlone(env),
    `calls=${env.calls.length} reloads=${env.reloads} confirms=${env.confirms}`);
}

{
  const env = await open(await setup({ modified: true }));
  check(results, 'Open with unsaved changes: new window, no discard question',
    openedInNewWindow(env, OTHER.token) && leftDocumentAlone(env),
    `calls=${env.calls.length} reloads=${env.reloads} confirms=${env.confirms}`);
}

{
  const env = await open(await setup({ docType: null }));
  check(results, 'Open on the start screen: same window, as before',
    env.calls.length === 0 && env.sameWindow.length === 1
      && env.sameWindow[0].token === OTHER.token && env.reloads === 0,
    `calls=${env.calls.length} sameWindow=${env.sameWindow.length}`);
}

{
  const env = await setup({ modified: true });
  await env.bridge.run(`_eoOpenRecent(${JSON.stringify(RECENT.path)})`);
  await settle(env);
  check(results, 'Recent with a document: new window',
    openedInNewWindow(env, RECENT.token) && leftDocumentAlone(env),
    `calls=${env.calls.length} reloads=${env.reloads} confirms=${env.confirms}`);
}

{
  const env = await open(await setup({ newWindow: () => 'already-open' }));
  const logged = env.bridge.logs.some((line) => line.includes('is the open document'));
  check(results, 'The open document itself: nothing happens, only a log line',
    openedInNewWindow(env, OTHER.token) && leftDocumentAlone(env)
      && env.messages.length === 0 && logged,
    `messages=${env.messages.length} logged=${logged}`);
}

{
  const env = await open(await setup({ newWindow: () => { throw new Error('spawn: denied'); } }));
  const shown = env.messages.length === 1
    && env.messages[0].text === env.t('openNewWindowFailedMsg')
    && env.messages[0].title === env.t('openFailed');
  check(results, 'A failed launch: error dialog, document left alone',
    shown && leftDocumentAlone(env),
    `messages=${JSON.stringify(env.messages)} reloads=${env.reloads}`);
}

{
  const env = await setup();
  const languages = ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'uk', 'zh', 'ja'];
  const missing = languages.filter((code) =>
    !env.bridge.run(`_UI_STRINGS[${JSON.stringify(code)}].openNewWindowFailedMsg`));
  check(results, 'The launch error text exists in every language',
    missing.length === 0, missing.join(' '));
}

finish(results);
