// Runs the Markdown page's own script (src/markdown/markdown.js) against
// stand-ins for the webview, with the recovery copy and the save held open:
// edit, Save, undo back to the saved text and Close must not close the
// document before the save is written.
//
// Run: node scripts/test-bridge-markdown-save-race.mjs
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { check, finish, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const listeners = {};
const stubs = new Map();

// A stand-in for any element or object the page only touches: it keeps what
// is set on it and the listeners added to it, and anything else is another one.
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

// The commands the page sends, in order; the recovery copy and the save wait
// until the test lets them go.
const commands = [];
const held = {};
const hold = (key) => new Promise((resolve) => { held[key] = resolve; });
const answers = {
  read_markdown: () => ({ data: Buffer.from('A').toString('base64'), name: 'a.md' }),
  markdown_recovery_write: () => hold('recovery'),
  save_markdown: () => hold('save'),
};
const invoke = async (command, args) => {
  commands.push(command);
  return answers[command] ? answers[command](args) : null;
};

const timers = [];
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
for (const file of ['markdown-text.js', 'markdown.js']) {
  const source = path.join(projectRoot, 'src', 'markdown', file);
  vm.runInContext(await readFile(source, 'utf8'), sandbox, { filename: source });
}

const settle = async () => { for (let i = 0; i < 20; i++) await new Promise((resolve) => setImmediate(resolve)); };
const type = async (text) => { stub('md-source').value = text; listeners['md-source:input'](); await settle(); };
const runTimers = () => timers.splice(0).forEach((callback) => callback());

listeners['md-preview:load']();
await settle();
await type('B');
runTimers();
listeners['md-save:click']();
await type('A');
listeners['md-close:click']();
await settle();
held.recovery();
await settle();
const closedEarly = commands.includes('close_document');
check(results, 'Close waits for the save that is being written', !closedEarly, commands.join(' '));
check(results, 'the save went on to be written', 'save' in held, commands.join(' '));
if (held.save) held.save('a.md');
await settle();
// The copy of the text on screen, written as the save ends.
held.recovery();
await settle();
check(results, 'once the save is written, Close closes the document', commands.includes('close_document'), commands.join(' '));
finish(results);
