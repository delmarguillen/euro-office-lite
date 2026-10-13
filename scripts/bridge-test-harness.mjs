// Runs the real src/bridge.js in a Node vm with the webview, Tauri and editor
// pieces it touches replaced by small fakes, so its clipboard paths can be
// exercised without a browser. Used by the scripts/test-bridge-*.mjs checks.
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
export const projectRoot = path.resolve(scriptDir, '..');

// The bridge to test: the working copy, or another file given on the command
// line (for instance one extracted with git show, to see a check fail on it).
export function bridgePathFromArgs() {
  const arg = process.argv[2];
  return arg ? path.resolve(arg) : path.join(projectRoot, 'src', 'bridge.js');
}

export async function loadBridge({ platform = 'Linux x86_64', invoke = {} } = {}) {
  const logs = [];
  const timers = new Set();
  const handlers = {
    js_log: () => null,
    take_pending_open_file: () => null,
    ...invoke,
  };
  const sandbox = {
    console: {
      log: () => {},
      warn: () => {},
      error: (...args) => logs.push('[console.error] ' + args.join(' ')),
    },
    navigator: { platform, language: 'en-US' },
    localStorage: {
      store: new Map(),
      getItem(key) { return this.store.has(key) ? this.store.get(key) : null; },
      setItem(key, value) { this.store.set(key, String(value)); },
      removeItem(key) { this.store.delete(key); },
    },
    document: {
      readyState: 'complete',
      addEventListener() {},
      getElementById() { return null; },
      querySelector() { return null; },
      querySelectorAll() { return []; },
    },
    addEventListener() {},
    setTimeout(fn, ms) {
      const timer = setTimeout(() => { timers.delete(timer); fn(); }, ms);
      timers.add(timer);
      return timer;
    },
    clearTimeout(timer) { timers.delete(timer); clearTimeout(timer); },
    // The spellcheck setup asks for its manifest at load; nothing here needs it.
    fetch: () => new Promise(() => {}),
    atob: (text) => Buffer.from(text, 'base64').toString('binary'),
    btoa: (text) => Buffer.from(text, 'binary').toString('base64'),
    XMLHttpRequest: class {
      open(method, url) { this.url = url; }
      send() {
        const answer = sandbox.__xhr ? sandbox.__xhr(this.url) : null;
        this.status = answer ? answer.status : 404;
        this.responseText = answer ? answer.body : '';
      }
    },
    __TAURI__: {
      core: {
        invoke: (command, args) => {
          const handler = handlers[command];
          try {
            return Promise.resolve(handler ? handler(args) : null);
          } catch (error) {
            return Promise.reject(error);
          }
        },
      },
      event: { listen: () => Promise.resolve(() => {}) },
    },
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  const source = await readFile(bridgePathFromArgs(), 'utf8');
  vm.runInContext(source, sandbox, { filename: 'bridge.js' });
  // Everything the bridge logs, from _eoLog and console.error alike.
  sandbox._eoLog = (...args) => logs.push(args.join(' '));
  return {
    sandbox,
    logs,
    run: (code) => vm.runInContext(code, sandbox),
    // Pending bridge timers must not keep a finished check alive.
    stopTimers() { for (const timer of timers) clearTimeout(timer); timers.clear(); },
  };
}

// A script of sdkjs evaluated in the same context, so it sees the same
// window.AscDesktopEditor and AscCommon the bridge does.
export async function loadSdkjsSnippet(bridge, relativePath, startMarker, endMarker) {
  // Checkouts may have CRLF line ends; the markers are written with LF.
  const raw = await readFile(path.join(projectRoot, relativePath), 'utf8');
  const source = raw.replace(/\r\n/g, '\n');
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start);
  if (start < 0 || end < 0) {
    throw new Error(`${relativePath}: cannot find ${start < 0 ? startMarker : endMarker}`);
  }
  bridge.run(source.slice(start, end + endMarker.length));
}

export function check(results, label, ok, detail = '') {
  results.push({ label, ok });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${detail ? '  (' + detail + ')' : ''}`);
}

export function finish(results) {
  const failed = results.filter((result) => !result.ok).length;
  console.log(`${results.length - failed} passed, ${failed} failed`);
  process.exitCode = failed ? 1 : 0;
}
