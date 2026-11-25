// Markdown: the text on screen changes while a save is being written, and the
// page is reloaded as the write ends, before any timer runs again. The reload
// must show the text that was on screen: the file holds the saved text, so
// the other one has to be in the recovery copy. The same when the write
// fails: the file holds the text from before, and the screen the one that
// was being saved.
//
// A save also never leaves a copy of text older than the file's: the copy
// the timer wrote is of AB, ABC is saved, and the process ends or the page is
// reloaded once the file is written, before the page hears of it.
//
// The page's own scripts (src/markdown) run against stand-ins for the
// webview. The fake backend keeps the file and the recovery copy, which
// outlive a reload, and drops the copy where the backend does:
// - read_markdown: opening the file ends the session and counts nothing as
//   unsaved (start_markdown_document);
// - recovery_end: the page asks (recovery::end_session). Without discard the
//   session ends and its folder stays for the next start;
// - save_markdown writes the file and keeps the session (store_markdown);
// - markdown_recovery_reload: a copy with the bytes of the file is dropped
//   and the page opens the file (reload_markdown).
// A start after the process ended offers the copies left that do not hold
// the bytes of the file (recovery::scan_candidates).
//
// A reload while the file is being written: the backend counts the write
// itself (RunningWrites), answers markdown_recovery_reload once it has ended
// and forgets the saves the page before the reload counted. The fake reads
// all three as app_quit::has_unsaved_changes does.
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
const told = ({ file, copy, released, modified, savesPending, writesRunning, closed }) =>
  JSON.stringify({ file, copy, released, modified, savesPending, writesRunning, closed });

// a.md holding `fileText`, with no recovery copy.
function makeBackend(fileText) {
  const backend = {
    file: fileText, copy: null, modified: false, savesPending: 0, write: null,
    // Writes of the file under way, and the reloads waiting for them to end.
    writesRunning: 0, reloadsWaiting: [],
    // The page closed the document.
    closed: false,
    // Copies the page let go of without deleting them.
    released: [],
    // The copies and the file, in the order they reached the disk.
    landed: [],
    // With holdCopies, a copy reaches the disk when the check lands it.
    holdCopies: false, heldCopies: [],
    // With failCopies, no copy can be written.
    failCopies: false,
  };
  const landCopy = (data) => {
    backend.copy = fromBase64(data);
    backend.landed.push('copy ' + backend.copy);
  };
  const writeEnded = () => {
    backend.writesRunning -= 1;
    backend.reloadsWaiting.splice(0).forEach((resume) => resume());
  };
  const noWriteRunning = async () => {
    while (backend.writesRunning > 0) await new Promise((resolve) => backend.reloadsWaiting.push(resolve));
  };
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
      backend.writesRunning += 1;
      backend.write = { data, answer: () => resolve(NAME), fail: () => reject(new Error('save-denied')) };
    }),
    markdown_recovery_write: ({ data }) => {
      if (backend.failCopies) throw new Error('the copy cannot be written');
      if (!backend.holdCopies) return landCopy(data);
      return new Promise((resolve) => { backend.heldCopies.push(() => { landCopy(data); resolve(); }); });
    },
    recovery_end: ({ discard }) => {
      if (!discard && backend.copy !== null) backend.released.push(backend.copy);
      backend.copy = null;
    },
    markdown_recovery_reload: async () => {
      await noWriteRunning();
      backend.savesPending = 0;
      if (backend.copy === backend.file) backend.copy = null;
      if (backend.copy === null) return null;
      return { data: toBase64(backend.copy), name: NAME, hasFile: true };
    },
    set_document_modified: ({ modified }) => { backend.modified = modified; },
    set_save_pending: ({ pending }) => { backend.savesPending += pending ? 1 : -1; },
    close_document: () => { backend.closed = true; },
  };
  // The file is written: the backend's part of a save.
  backend.endWrite = () => {
    backend.file = fromBase64(backend.write.data);
    backend.landed.push('file ' + backend.file);
    writeEnded();
  };
  // The page hears that the save ended.
  backend.answerSave = () => backend.write.answer();
  // The write ends with nothing written.
  backend.failWrite = () => writeEnded();
  // Nothing is written, and the page hears that the save failed.
  backend.failSave = () => {
    backend.failWrite();
    backend.write.fail();
  };
  backend.unsaved = () => backend.modified || backend.savesPending > 0 || backend.writesRunning > 0;
  // The held copies reach the disk, and those the page asks for meanwhile.
  backend.landHeldCopies = async () => {
    while (backend.heldCopies.length) {
      backend.heldCopies.shift()();
      await settle();
    }
  };
  // What the start screen offers once the process has ended.
  backend.offeredAtStart = () => [...backend.released, backend.copy]
    .filter((copy) => copy !== null && copy !== backend.file);
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
// run only when a check runs them. A reloaded page may still be waiting for
// its document when this returns.
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
    clickClose: async () => { listeners['md-close:click'](); await settle(); },
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

// a.md holds A. The user types AB, the copy's timer comes due, and ABC is
// typed and saved before it comes due again. The save is being written.
async function saveAbcOverACopyOfAb(backend) {
  const page = await openPage(backend);
  await page.type('AB');
  await page.runTimers();
  await page.type('ABC');
  await page.clickSave();
  return page;
}

// The process ends while ABC is being written, and once it is written,
// before the page hears of it.
async function checkProcessEndsAroundTheWrite() {
  const backend = makeBackend('A');
  await saveAbcOverACopyOfAb(backend);
  check(results, 'copy of AB, ABC being written, process ends: the next start offers ABC',
    backend.offeredAtStart().join('|') === 'ABC', told(backend));
  backend.endWrite();
  check(results, 'copy of AB, ABC written, process ends: the copy left is not older than the file',
    backend.file === 'ABC' && backend.copy === 'ABC', told(backend));
  check(results, 'copy of AB, ABC written, process ends: the next start offers nothing',
    backend.offeredAtStart().length === 0, backend.offeredAtStart().join('|'));
}

// The page is reloaded once ABC is written, before it hears of it. Saving
// from the reloaded page must not put AB over ABC.
async function checkReloadAsAbcIsWritten() {
  const backend = makeBackend('A');
  await saveAbcOverACopyOfAb(backend);
  backend.endWrite();

  const reloaded = await reloadPage(backend);
  check(results, 'copy of AB, ABC written, reload: the page shows ABC', reloaded.text() === 'ABC', String(reloaded.text()));
  check(results, 'copy of AB, ABC written, reload: ABC is not marked modified', !backend.modified, told(backend));
  await reloaded.clickSave();
  if (backend.write) backend.endWrite();
  check(results, 'copy of AB, ABC written, reload, Save: the file still holds ABC', backend.file === 'ABC', told(backend));
}

// The copy of AB is still on its way to the disk when ABC is saved. The
// backend takes the writes it has been asked for in any order: here the file
// goes first whenever it has been asked for.
async function checkCopyOnItsWayWhenTheSaveBegins() {
  const backend = makeBackend('A');
  backend.holdCopies = true;
  await saveAbcOverACopyOfAb(backend);
  const fileAskedFor = backend.write !== null;
  check(results, 'copy of AB on its way, Save: the file is not asked for yet', !fileAskedFor, told(backend));
  if (fileAskedFor) backend.endWrite();
  await backend.landHeldCopies();
  if (!fileAskedFor) backend.endWrite();
  check(results, 'copy of AB on its way, Save: AB, then ABC, reach the disk before the file',
    backend.landed.join(', ') === 'copy AB, copy ABC, file ABC', backend.landed.join(', '));
  check(results, 'copy of AB on its way, Save: the next start offers nothing',
    backend.offeredAtStart().length === 0, backend.offeredAtStart().join('|'));
}

// The copy of ABC cannot be written before the save: AB, the copy on disk,
// would be older than the file. The save is written all the same.
async function checkCopyThatCannotBeReplaced() {
  const backend = makeBackend('A');
  const page = await openPage(backend);
  await page.type('AB');
  await page.runTimers();
  backend.failCopies = true;
  await page.type('ABC');
  await page.clickSave();
  check(results, 'copy not replaced: the save is written all the same', backend.write !== null, told(backend));
  check(results, 'copy not replaced: AB is let go of, not deleted',
    backend.copy === null && backend.released.join('|') === 'AB', told(backend));
  backend.endWrite();

  const reloaded = await reloadPage(backend);
  check(results, 'copy not replaced, reload: the page shows the file, unmodified',
    reloaded.text() === 'ABC' && !backend.modified, String(reloaded.text()) + ' ' + told(backend));
}

// The page is reloaded while B is being written, and Close is pressed on the
// new page before the write ends.
async function checkCloseAfterAReloadDuringTheWrite() {
  const { backend } = await saveBOverA();
  const reloaded = await reloadPage(backend);
  check(results, 'reload during the write: the backend counts the document as unsaved', backend.unsaved(), told(backend));
  await reloaded.clickClose();
  check(results, 'reload during the write, Close: the document stays open and keeps its copy while the file is written',
    !backend.closed && backend.copy === 'B', told(backend));
  backend.endWrite();
  await settle();
  check(results, 'reload during the write, Close: the document closes once the file is written',
    backend.closed && backend.file === 'B', told(backend));
}

// The write of B ends after the reload, and the new page saves C.
async function checkSaveAfterAReloadDuringTheWrite() {
  const { backend } = await saveBOverA();
  const reloaded = await reloadPage(backend);
  backend.endWrite();
  await settle();
  check(results, 'reload during the write, write ended: the page shows B and nothing is unsaved',
    reloaded.text() === 'B' && !backend.unsaved(), String(reloaded.text()) + ' ' + told(backend));
  await reloaded.type('C');
  await reloaded.clickSave();
  backend.endWrite();
  backend.answerSave();
  await settle();
  check(results, 'reload during the write, then C saved: the file holds C', backend.file === 'C', told(backend));
  check(results, 'reload during the write, then C saved: no save is pending and nothing is unsaved',
    backend.savesPending === 0 && backend.writesRunning === 0 && !backend.unsaved(), told(backend));
}

// The write of B fails after the reload: B is in no file.
async function checkFailedWriteAfterAReload() {
  const { backend } = await saveBOverA();
  const reloaded = await reloadPage(backend);
  backend.failWrite();
  await settle();
  check(results, 'reload during the write, write failed: the page shows B, marked modified',
    reloaded.text() === 'B' && backend.modified, String(reloaded.text()) + ' ' + told(backend));
  check(results, 'reload during the write, write failed: no save is pending',
    backend.savesPending === 0 && backend.writesRunning === 0, told(backend));
}

await checkUndoThenReloadAfterTheSave();
await checkUndoThenReloadAsTheWriteEnds();
await checkTypingOnThenReload();
await checkSaveThenReload();
await checkFailedSaveThenReload();
await checkFailedSaveAfterUndo();
await checkProcessEndsAroundTheWrite();
await checkReloadAsAbcIsWritten();
await checkCopyOnItsWayWhenTheSaveBegins();
await checkCopyThatCannotBeReplaced();
await checkCloseAfterAReloadDuringTheWrite();
await checkSaveAfterAReloadDuringTheWrite();
await checkFailedWriteAfterAReload();
finish(results);
