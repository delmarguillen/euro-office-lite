// Runs the Markdown page's own script (src/markdown/markdown.js) against
// stand-ins for the webview and checks the files it opens in a new window: a
// template the Finder hands over (newDocument) starts a new document through
// new_document_in_new_window, while any other file it hands over, and Ctrl+O,
// still go through open_in_new_window and edit the file. "Open in another
// window" is told the same way on both.
//
// Run: node scripts/test-bridge-markdown-pending-open.mjs [path/to/markdown.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { check, finish, projectRoot } from './bridge-test-harness.mjs';

const results = [];

// A stand-in for any element or object the page only touches: it keeps what
// is set on it and the listeners added to it, and anything else is another one.
function stubFactory(listeners) {
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

const settle = async () => { for (let i = 0; i < 30; i++) await new Promise((resolve) => setImmediate(resolve)); };

// The page with a .md open; `pending` is what the Finder has queued.
async function openPage({ pending = [], newWindowAnswer = 'opened', picked = null } = {}) {
  const listeners = {};
  const events = {};
  const stub = stubFactory(listeners);
  const env = { calls: [], messages: [] };
  const queue = [...pending];
  const answers = {
    read_markdown: () => ({ data: Buffer.from('A').toString('base64'), name: 'notes.md' }),
    take_pending_open_file: () => queue.shift() || null,
    pick_document: () => picked,
    open_in_new_window: () => newWindowAnswer,
    new_document_in_new_window: () => newWindowAnswer,
  };
  const invoke = async (command, args) => {
    if (command === 'open_in_new_window' || command === 'new_document_in_new_window') {
      env.calls.push({ command, token: args.token });
    }
    return answers[command] ? answers[command](args) : null;
  };
  const documentListeners = {};
  const sandbox = {
    TextDecoder, TextEncoder, URLSearchParams, atob, btoa, navigator: { language: 'en' },
    setTimeout: () => 0, clearTimeout() {},
    location: { search: '?token=t', pathname: '/markdown/markdown.html', href: '' },
    history: { replaceState() {} },
    document: {
      getElementById: stub,
      querySelectorAll: () => [],
      documentElement: stub('html'),
      addEventListener: (type, listener) => { documentListeners[type] = listener; },
    },
    __TAURI__: {
      core: { invoke },
      event: { listen: async (name, handler) => { events[name] = handler; } },
      dialog: {
        confirm: async () => true,
        message: async (text) => { env.messages.push(text); },
      },
    },
    markdownit: stub('markdownit'), markdownitFootnote: stub('footnote'), markdownitTaskLists: stub('tasks'),
    DOMPurify: stub('DOMPurify'),
    addEventListener() {},
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  const markdownJs = process.argv[2]
    ? path.resolve(process.argv[2])
    : path.join(projectRoot, 'src', 'markdown', 'markdown.js');
  for (const source of [path.join(projectRoot, 'src', 'markdown', 'markdown-text.js'), markdownJs]) {
    vm.runInContext(await readFile(source, 'utf8'), sandbox, { filename: source });
  }
  listeners['md-preview:load']();
  await settle();
  env.handOver = async (...files) => {
    queue.push(...files);
    events['pending-open-file']();
    await settle();
  };
  env.ctrlO = async () => {
    documentListeners.keydown({
      ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, key: 'o',
      preventDefault() {}, stopPropagation() {},
    });
    await settle();
  };
  return env;
}

function sent(env) {
  return env.calls.map((call) => call.command + ':' + call.token).join(' ');
}

{
  const env = await openPage();
  await env.handOver({ token: 'tok-tpl', name: 'Letter.dotx', newDocument: true });
  check(results, 'a template the Finder hands over starts a new document in a new window',
    sent(env) === 'new_document_in_new_window:tok-tpl', sent(env));
}

{
  const env = await openPage();
  await env.handOver({ token: 'tok-doc', name: 'report.docx', newDocument: false });
  check(results, 'any other file it hands over opens in a new window to be edited',
    sent(env) === 'open_in_new_window:tok-doc', sent(env));
}

{
  const env = await openPage({ pending: [{ token: 'tok-launch', name: 'Budget.xltx', newDocument: true }] });
  check(results, 'a template queued before the page listened starts a new document too',
    sent(env) === 'new_document_in_new_window:tok-launch', sent(env));
}

{
  const env = await openPage({ picked: { token: 'tok-pick', name: 'Letter.dotx' } });
  await env.ctrlO();
  check(results, 'Ctrl+O on a template edits it in a new window',
    sent(env) === 'open_in_new_window:tok-pick', sent(env));
}

for (const newDocument of [true, false]) {
  const env = await openPage({ newWindowAnswer: 'open-elsewhere' });
  await env.handOver({ token: 'tok-x', name: newDocument ? 'Letter.dotx' : 'report.docx', newDocument });
  check(results, `"open in another window" is told (${newDocument ? 'new document' : 'edit'})`,
    env.messages.length === 1, env.messages.join(' | '));
}

finish(results);
