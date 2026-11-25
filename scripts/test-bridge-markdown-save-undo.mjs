// Markdown: the user saves and, while the file is being written, undoes back
// to the text the file had. Once the save ends the file holds the saved text
// and the screen another one: the document is modified and needs a recovery
// copy of the text on screen.
//
// First the page's pure helpers (src/markdown/markdown-text.js), then the
// page's own script (src/markdown/markdown.js) against stand-ins for the
// webview. The fake backend keeps AppState.modified and AppState.saves_pending
// and reads them as app_quit::has_unsaved_changes does.
//
// Run: node scripts/test-bridge-markdown-save-undo.mjs
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { check, finish, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const markdownDir = path.join(projectRoot, 'src', 'markdown');

async function runPageScript(sandbox, file) {
  const source = path.join(markdownDir, file);
  vm.runInContext(await readFile(source, 'utf8'), sandbox, { filename: source });
}

// ── The pure helpers ──
async function checkHelpers() {
  const sandbox = { TextDecoder, TextEncoder, decodeURIComponent };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  await runPageScript(sandbox, 'markdown-text.js');
  const Text = sandbox.EoMarkdownText;

  const hasHelper = typeof Text.needsRecoveryCopy === 'function';
  check(results, 'markdown-text.js has needsRecoveryCopy', hasHelper);
  if (!hasHelper) return;

  // The file holds A, the user typed AB, saved, and undid back to A.
  const writing = Text.afterSaveEvent(Text.savedState('A'), { type: 'write-start', text: 'AB' });
  check(results, 'undo during the write: a save is pending', Text.unsavedChanges(writing, 'A').savePending);
  check(results, 'undo during the write: the text on screen needs a recovery copy', Text.needsRecoveryCopy(writing, 'A'));
  check(results, 'no undo during the write: the text being written needs no copy', !Text.needsRecoveryCopy(writing, 'AB'));

  const written = Text.afterSaveEvent(writing, { type: 'write-end', ok: true });
  check(results, 'undo, write ended well: the document is modified', Text.unsavedChanges(written, 'A').modified);
  check(results, 'undo, write ended well: the text on screen needs a recovery copy', Text.needsRecoveryCopy(written, 'A'));
  check(results, 'undo, write ended well: Close asks', Text.closeAction(written, 'A') === 'ask');
  check(results, 'no undo, write ended well: nothing unsaved, no copy needed',
    !Text.unsavedChanges(written, 'AB').modified && !Text.needsRecoveryCopy(written, 'AB'));

  const failed = Text.afterSaveEvent(writing, { type: 'write-end', ok: false });
  check(results, 'undo, write failed: the screen shows the file again, no copy needed',
    !Text.unsavedChanges(failed, 'A').modified && !Text.needsRecoveryCopy(failed, 'A'));
  check(results, 'no undo, write failed: the text still needs its copy', Text.needsRecoveryCopy(failed, 'AB'));
}

// ── The page ──
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

// The page with a.md (text A) open. The save waits until the check lets it go.
async function openPage() {
  const listeners = {};
  const stub = makeStubs(listeners);
  const backend = { modified: false, savesPending: 0 };
  const commands = [];
  const copies = [];
  const timers = [];
  let endSave = null;
  const answers = {
    read_markdown: () => ({ data: Buffer.from('A').toString('base64'), name: 'a.md' }),
    set_document_modified: ({ modified }) => { backend.modified = modified; },
    set_save_pending: ({ pending }) => { backend.savesPending += pending ? 1 : -1; },
    markdown_recovery_write: ({ data }) => { copies.push(Buffer.from(data, 'base64').toString()); },
    save_markdown: () => new Promise((resolve) => { endSave = () => resolve('a.md'); }),
  };
  const invoke = async (command, args) => {
    commands.push(command);
    return answers[command] ? answers[command](args) : null;
  };
  const sandbox = {
    TextDecoder, TextEncoder, URLSearchParams, atob, btoa, navigator: { language: 'en' },
    setTimeout: (callback) => timers.push(callback), clearTimeout() {},
    location: { search: '?token=t', pathname: '/markdown/markdown.html', href: '' },
    history: { replaceState() {} },
    document: { getElementById: stub, querySelectorAll: () => [], documentElement: stub('html'), addEventListener() {} },
    __TAURI__: { core: { invoke }, event: { listen: async () => {} }, dialog: { confirm: async () => true, message: async () => {} } },
    markdownit: stub('markdownit'), markdownitFootnote: stub('footnote'), markdownitTaskLists: stub('tasks'), DOMPurify: stub('DOMPurify'),
    addEventListener() {},
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  await runPageScript(sandbox, 'markdown-text.js');
  await runPageScript(sandbox, 'markdown.js');

  const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((resolve) => setImmediate(resolve)); };
  const page = {
    backend, commands, copies, settle,
    backendUnsaved: () => backend.modified || backend.savesPending > 0,
    type: async (text) => { stub('md-source').value = text; listeners['md-source:input'](); await settle(); },
    runTimers: async () => { timers.splice(0).forEach((callback) => callback()); await settle(); },
    clickSave: async () => { listeners['md-save:click'](); await settle(); },
    endSave: async () => { endSave(); await settle(); },
  };
  listeners['md-preview:load']();
  await settle();
  return page;
}

// Edit, wait for the recovery copy, and start a save that stays writing.
async function editAndStartSave(page) {
  await page.type('AB');
  await page.runTimers();
  await page.clickSave();
}

async function checkUndoDuringTheWrite() {
  const page = await openPage();
  await editAndStartSave(page);
  check(results, 'page: the edit got its recovery copy and the save is writing',
    page.copies.join('|') === 'AB' && page.commands.includes('save_markdown'), page.commands.join(' '));
  await page.type('A');
  // The copy's timer comes due while the file is still being written.
  await page.runTimers();
  check(results, 'page, undo during the write: the backend counts the document as unsaved', page.backendUnsaved(),
    JSON.stringify(page.backend));
  await page.endSave();
  await page.runTimers();
  check(results, 'page, undo, write ended: the backend counts the document as modified, no save pending',
    page.backend.modified && page.backend.savesPending === 0, JSON.stringify(page.backend));
  // Copied when its timer came due during the write, and again as the write ended.
  check(results, 'page, undo, write ended: the text on screen got a recovery copy',
    page.copies.join('|') === 'AB|A|A', page.copies.join('|'));
}

async function checkSaveWithoutUndo() {
  const page = await openPage();
  await editAndStartSave(page);
  await page.endSave();
  await page.runTimers();
  check(results, 'page, no undo, write ended: the backend counts nothing as unsaved',
    !page.backend.modified && page.backend.savesPending === 0, JSON.stringify(page.backend));
  check(results, 'page, no undo, write ended: no recovery copy of saved text',
    page.copies.join('|') === 'AB', page.copies.join('|'));
}

await checkHelpers();
await checkUndoDuringTheWrite();
await checkSaveWithoutUndo();
finish(results);
