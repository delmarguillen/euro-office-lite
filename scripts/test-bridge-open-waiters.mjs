// Checks the files that wait in LocalFileOpen for the open in progress in this
// window, all woken at once when its indicator goes away. Only the first opens
// here: the others see its indicator and wait again, then go to a new window
// once it is open. Two opens at once in one window could leave the editor with
// one document and the backend saving to the other's file. The open here is the
// real _openPath of editor-patches.js; the editor is a small fake of openEditor.
//
// Run: node scripts/test-bridge-open-waiters.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const A = { token: 'tok-a', name: 'a.docx' };
const B = { token: 'tok-b', name: 'b.docx' };
const C = { token: 'tok-c', name: 'c.docx' };
const D = { token: 'tok-d', name: 'd.docx' };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// Longer than the 100 ms LocalStartOpen waits before it opens the document.
const AFTER_START_OPEN = 160;

const START = '    async function _openPath(file, silent) {';
const END = '\n    }\n';

// The start screen's open in this window, cut from editor-patches.js.
async function openPathSource() {
  const raw = await readFile(path.join(projectRoot, 'src', 'editor-patches.js'), 'utf8');
  const source = raw.replace(/\r\n/g, '\n');
  const start = source.indexOf(START);
  const end = source.indexOf(END, start);
  if (start < 0 || end < 0) throw new Error('editor-patches.js: cannot find _openPath');
  return source.slice(start, end + END.length);
}

function element() {
  const names = new Set();
  return {
    classList: {
      add: (name) => names.add(name),
      remove: (name) => names.delete(name),
      contains: (name) => names.has(name),
      toggle: (name, on) => (on ? names.add(name) : names.delete(name)),
    },
    inert: false,
    textContent: '',
  };
}

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

// `fonts`: 'ready'; 'pending', the cache being built, with its notice over the
// start screen until 'fonts-ready'; or 'unanswered', no editor created until
// the check calls fontsAnswer().
async function loadPage({ fonts }) {
  const page = { opens: [], newWindow: [], editors: [], loaded: [], saveTarget: null, messages: [] };
  const fontsStatus = deferred();
  const bridge = await loadBridge({
    invoke: {
      fonts_status: () => (fonts === 'unanswered' ? fontsStatus.promise : fonts),
      open_file: (args) => new Promise((resolve, reject) => {
        page.opens.push({ token: args.token, resolve, reject });
      }),
      open_in_new_window: (args) => { page.newWindow.push(args.token); return 'opened'; },
    },
  });
  const elements = {
    'start-screen': element(),
    opening: element(),
    'opening-text': element(),
    placeholder: element(),
  };
  bridge.sandbox.document.body = element();
  bridge.sandbox.document.getElementById = (id) => elements[id] || null;
  bridge.sandbox.__TAURI__.dialog = {
    message: (text) => { page.messages.push(text); return Promise.resolve(); },
  };
  // What openEditor does with the document it is created for.
  bridge.sandbox._openEditor = (docType, awaited) => {
    if (awaited && awaited.state === 'failed') return;
    bridge.sandbox.AscDesktopEditor._currentDocType = docType;
    if (awaited && awaited.state === 'converting') {
      bridge.sandbox._eoSetEditorBehind(true);
    } else {
      elements['start-screen'].classList.add('hidden');
      bridge.sandbox._eoHideOpening();
    }
    if (awaited) awaited.editorCreated = true;
    page.editors.push(awaited && awaited.name);
    // sdkjs asks for its document once the editor has loaded.
    bridge.sandbox.AscDesktopEditor.LocalStartOpen();
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { openDocument: () => {} } },
    AscCommon: { getEmpty: () => new Uint8Array(4), OpenFileResult: function() {}, History: {} },
  };
  bridge.sandbox._loadEditorBin = (data, fileName) => { page.loaded.push(fileName); };
  bridge.run(`${await openPathSource()}\nwindow._eoOpenPath = _openPath;`);
  page.bridge = bridge;
  page.fontsAnswer = () => fontsStatus.resolve('ready');
  page.open = (file) => bridge.run(`AscDesktopEditor.LocalFileOpen(${JSON.stringify(file)})`);
  page.requested = () => page.opens.map((open) => open.token).join();
  // The backend makes a converted file the one it saves to (open_file).
  page.answer = (file) => {
    const open = page.opens.find((entry) => entry.token === file.token);
    if (!open) return;
    page.saveTarget = file.name;
    open.resolve('bytes-' + file.token);
  };
  page.fail = (file) => page.opens.find((entry) => entry.token === file.token).reject('x2t exit code 80');
  page.state = () => `converted here=${page.requested()} newWindow=${page.newWindow} `
    + `editors=${page.editors} editor shows=${page.loaded} backend saves to=${page.saveTarget}`;
  return page;
}

// A fails before its editor is created; B and C were waiting for it, and their
// conversions would end in reverse order.
{
  const page = await loadPage({ fonts: 'unanswered' });
  page.open(A);
  await sleep(20);
  page.open(B);
  page.open(C);
  await sleep(20);
  const waited = page.requested() === A.token;
  page.fail(A);
  await sleep(20);
  page.fontsAnswer();
  await sleep(20);
  page.answer(C);
  await sleep(AFTER_START_OPEN);
  page.answer(B);
  await sleep(AFTER_START_OPEN);
  check(results, 'A failing: B and C wait while A converts', waited, page.state());
  check(results, '... only one of them is converted here', page.requested() === `${A.token},${B.token}`, page.state());
  check(results, '... the other goes to a new window once it is open', page.newWindow.join() === C.token, page.state());
  check(results, '... one editor, showing the document the backend saves to',
    page.editors.length === 1 && page.loaded.length === 1 && page.loaded[0] === page.saveTarget, page.state());
  page.bridge.stopTimers();
}

// A opens; B, C and D were waiting for it. Its editor is created while it
// converts, or once it has arrived.
for (const fonts of ['ready', 'unanswered']) {
  const page = await loadPage({ fonts });
  page.open(A);
  await sleep(20);
  page.open(B);
  page.open(C);
  page.open(D);
  await sleep(20);
  const waited = page.requested() === A.token && page.newWindow.length === 0;
  page.answer(A);
  await sleep(20);
  if (fonts === 'unanswered') page.fontsAnswer();
  await sleep(AFTER_START_OPEN);
  const label = fonts === 'ready' ? 'editor created during the conversion' : 'editor created after it';
  check(results, `A opening, ${label}: B, C and D wait`, waited, page.state());
  check(results, '... and all go to new windows, none here',
    page.requested() === A.token && page.newWindow.join() === [B.token, C.token, D.token].join(), page.state());
  check(results, '... the editor shows A, which the backend saves to',
    page.loaded.join() === A.name && page.saveTarget === A.name, page.state());
  page.bridge.stopTimers();
}

// The same wake-up when the font cache notice goes away with files waiting.
{
  const page = await loadPage({ fonts: 'pending' });
  await sleep(20);
  page.open(B);
  page.open(C);
  page.open(D);
  await sleep(20);
  const waited = page.opens.length === 0;
  page.bridge.emit('fonts-ready', 'ready');
  await sleep(20);
  page.answer(B);
  await sleep(AFTER_START_OPEN);
  check(results, 'font cache notice: B, C and D wait for it', waited, page.state());
  check(results, '... then only B is converted here, C and D go to new windows',
    page.requested() === B.token && page.newWindow.join() === [C.token, D.token].join(), page.state());
  check(results, '... the editor shows B, which the backend saves to',
    page.loaded.join() === B.name && page.saveTarget === B.name, page.state());
  page.bridge.stopTimers();
}

finish(results);
