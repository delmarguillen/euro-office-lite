// Markdown: the text on screen changes while a save is being written, and the
// page is reloaded as the write ends, before any timer runs again. The reload
// must show the text that was on screen: the file holds the saved text, so
// the other one has to be in the recovery copy. The same when the write
// fails: the file holds the text from before, and the screen the one that
// was being saved.
//
// The page's own scripts (src/markdown) run against stand-ins for the
// webview. The fake backend keeps the file and the recovery copy, which
// outlive a reload, and drops the copy where the backend does:
// - read_markdown: opening the file ends the session and counts nothing as
//   unsaved (start_markdown_document);
// - recovery_end with discard: the page asks (recovery::end_session);
// - save_markdown writes the file and keeps the session (store_markdown).
//
// Run: node scripts/test-bridge-markdown-save-reload.mjs
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { check, finish, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const markdownDir = path.join(projectRoot, 'src', 'markdown');
const NAME = 'a.md';

const toBase64 = (text) => Buffer.from(text).toString('base64');
const fromBase64 = (data) => Buffer.from(data, 'base64').toString();
// What a failed check prints of the backend.
const told = ({ file, copy, modified, savesPending }) => JSON.stringify({ file, copy, modified, savesPending });

// a.md holding `fileText`, with no recovery copy.
function makeBackend(fileText) {
  const backend = { file: fileText, copy: null, modified: false, savesPending: 0, write: null };
  backend.answers = {
    current_document: () => ({ token: 't', name: NAME }),
    read_markdown: () => {
      backend.copy = null;
      backend.modified = false;
      backend.savesPending = 0;
      return { data: toBase64(backend.file), name: NAME };
    },
    // The write waits until the check ends it.
    save_markdown: ({ data }) => new Promise((resolve, reject) => {
      backend.write = { data, answer: () => resolve(NAME), fail: () => reject(new Error('save-denied')) };
    }),
    markdown_recovery_write: ({ data }) => { backend.copy = fromBase64(data); },
    recovery_end: ({ discard }) => { if (discard) backend.copy = null; },
    markdown_recovery_reload: () => (backend.copy === null
      ? null
      : { data: toBase64(backend.copy), name: NAME, hasFile: true }),
    set_document_modified: ({ modified }) => { backend.modified = modified; },
    set_save_pending: ({ pending }) => { backend.savesPending += pending ? 1 : -1; },
  };
  // The file is written: the backend's part of a save.
  backend.endWrite = () => { backend.file = fromBase64(backend.write.data); };
  // The page hears that the save ended.
  backend.answerSave = () => backend.write.answer();
  // Nothing is written, and the page hears that the save failed.
  backend.failSave = () => backend.write.fail();
  return backend;
}

// A stand-in for any element or object the page only touches: it keeps what
// is set on it and the listeners added to it, and anything else is another one.
function makeStubs(listeners) {
  const stubs = new Map();
  function stub(name) {
    if (stubs.has(name)) return stubs.get(name);
    const store = {};
    const proxy = new Proxy(function() {}, {
      get(target, prop) {
        if (prop === 'then') return undefined;
        if (prop === Symbol.toPrimitive) return () => 0;
        if (prop === 'addEventListener') return (type, listener) => { listeners[name + ':' + type] = listener; };
        if (prop in store) return store[prop];
        return stub(name + '.' + String(prop));
      },
      set(target, prop, value) { store[prop] = value; return true; },
      apply() { return stub(name + '()'); },
    });
    stubs.set(name, proxy);
    return proxy;
  }
  return stub;
}

const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((resolve) => setImmediate(resolve)); };

// The page against `backend`: opened on the file (`search` with its token),
// or reloaded (no `search`, as takePending leaves the address). Its timers
// run only when a check runs them.
async function loadPage(backend, search) {
  const listeners = {};
  const stub = makeStubs(listeners);
  const timers = [];
  const invoke = async (command, args) => (backend.answers[command] ? backend.answers[command](args) : null);
  // CodeMirror does not load here: the page edits in its textarea.
  const head = { appendChild: (script) => script.onerror() };
  const sandbox = {
    TextDecoder, TextEncoder, URLSearchParams, atob, btoa, navigator: { language: 'en' },
    setTimeout: (callback) => timers.push(callback), clearTimeout() {},
    location: { search, pathname: '/markdown/markdown.html', href: '' },
    history: { replaceState() {} },
    document: {
      getElementById: stub, querySelectorAll: () => [], documentElement: stub('html'), addEventListener() {},
      createElement: () => ({}), head, body: stub('body'),
    },
    __TAURI__: { core: { invoke }, event: { listen: async () => {} }, dialog: { confirm: async () => true, message: async () => {} } },
    markdownit: stub('markdownit'), markdownitFootnote: stub('footnote'), markdownitTaskLists: stub('tasks'), DOMPurify: stub('DOMPurify'),
    addEventListener() {},
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  for (const file of ['markdown-text.js', 'markdown.js']) {
    const source = path.join(markdownDir, file);
    vm.runInContext(await readFile(source, 'utf8'), sandbox, { filename: source });
  }
  listeners['md-preview:load']();
  await settle();
  return {
    text: () => stub('md-source').value,
    type: async (text) => { stub('md-source').value = text; listeners['md-source:input'](); await settle(); },
    runTimers: async () => { timers.splice(0).forEach((callback) => callback()); await settle(); },
    clickSave: async () => { listeners['md-save:click'](); await settle(); },
  };
}

const openPage = (backend) => loadPage(backend, '?token=t');
// The page that was open is gone, with its timers; the backend stays.
const reloadPage = (backend) => loadPage(backend, '');

// a.md holds A. The user changes it to B, the copy's timer comes due, and
// Save begins to write B.
async function saveBOverA() {
  const backend = makeBackend('A');
  const page = await openPage(backend);
  await page.type('B');
  await page.runTimers();
  await page.clickSave();
  return { backend, page };
}

// The user undoes back to A while B is written, and no timer runs before the
// page is reloaded. The reload comes once the page has ended the save.
async function checkUndoThenReloadAfterTheSave() {
  const { backend, page } = await saveBOverA();
  check(results, 'B got its recovery copy and its save is writing',
    backend.copy === 'B' && backend.write !== null && backend.savesPending === 1, told(backend));
  await page.type('A');
  backend.endWrite();
  backend.answerSave();
  await settle();
  check(results, 'undo, save ended: the file holds B and the recovery copy A',
    backend.file === 'B' && backend.copy === 'A', told(backend));
  check(results, 'undo, save ended: the backend counts the document as modified, no save pending',
    backend.modified && backend.savesPending === 0, told(backend));

  const reloaded = await reloadPage(backend);
  check(results, 'undo, reload after the save: the page shows A', reloaded.text() === 'A', String(reloaded.text()));
  check(results, 'undo, reload after the save: A is marked modified', backend.modified, told(backend));
}

// The copy's timer comes due while B is written, and the page is reloaded as
// the write ends, before it hears of it.
async function checkUndoThenReloadAsTheWriteEnds() {
  const { backend, page } = await saveBOverA();
  await page.type('A');
  await page.runTimers();
  backend.endWrite();

  const reloaded = await reloadPage(backend);
  check(results, 'undo, reload as the write ends: the page shows A', reloaded.text() === 'A', String(reloaded.text()));
  check(results, 'undo, reload as the write ends: A is marked modified', backend.modified, told(backend));
}

// The user types on while B is written: the same with text never saved.
async function checkTypingOnThenReload() {
  const { backend, page } = await saveBOverA();
  await page.type('BC');
  backend.endWrite();
  backend.answerSave();
  await settle();

  const reloaded = await reloadPage(backend);
  check(results, 'typing on, reload after the save: the page shows BC', reloaded.text() === 'BC', String(reloaded.text()));
}

// Nothing changes while B is written: its copy is dropped, and a reload
// opens the file.
async function checkSaveThenReload() {
  const { backend } = await saveBOverA();
  backend.endWrite();
  backend.answerSave();
  await settle();
  check(results, 'no change, save ended: no recovery copy of saved text', backend.copy === null, told(backend));

  const reloaded = await reloadPage(backend);
  check(results, 'no change, reload after the save: the page shows the file, unmodified',
    reloaded.text() === 'B' && !backend.modified, String(reloaded.text()) + ' ' + told(backend));
}

// The user changes A to B and saves before the copy's timer comes due. The
// timer comes due while B is written, when B needs no copy, and the write
// fails: B is in no file.
async function checkFailedSaveThenReload() {
  const backend = makeBackend('A');
  const page = await openPage(backend);
  await page.type('B');
  await page.clickSave();
  await page.runTimers();
  backend.failSave();
  await settle();
  check(results, 'save failed: the file holds A and the recovery copy B',
    backend.file === 'A' && backend.copy === 'B', told(backend));
  check(results, 'save failed: the backend counts the document as modified, no save pending',
    backend.modified && backend.savesPending === 0, told(backend));

  const reloaded = await reloadPage(backend);
  check(results, 'save failed, reload: the page shows B', reloaded.text() === 'B', String(reloaded.text()));
  check(results, 'save failed, reload: B is marked modified', backend.modified, told(backend));
}

// The user undoes back to A while a write that fails runs: the screen shows
// the file again, and the copy of B stays.
async function checkFailedSaveAfterUndo() {
  const { backend, page } = await saveBOverA();
  await page.type('A');
  backend.failSave();
  await settle();
  check(results, 'undo, save failed: the copy on disk is not dropped', backend.copy === 'B', told(backend));
  check(results, 'undo, save failed: the backend counts nothing as unsaved',
    !backend.modified && backend.savesPending === 0, told(backend));
}

await checkUndoThenReloadAfterTheSave();
await checkUndoThenReloadAsTheWriteEnds();
await checkTypingOnThenReload();
await checkSaveThenReload();
await checkFailedSaveThenReload();
await checkFailedSaveAfterUndo();
finish(results);
