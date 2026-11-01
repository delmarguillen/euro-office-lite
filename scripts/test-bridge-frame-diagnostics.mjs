// Checks that _installFrameDiagnostics (editor-patches.js) is live on the
// editor document after the frame navigates from its initial about:blank,
// whether the browser keeps the window object or gives a new one, and that a
// kept window logs each error once.
//
// Run: node scripts/test-bridge-frame-diagnostics.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const bridge = await loadBridge();

const source = (await readFile(path.join(projectRoot, 'src/editor-patches.js'), 'utf8')).replace(/\r\n/g, '\n');
const start = source.indexOf('    function _installFrameDiagnostics(win) {');
const end = source.indexOf('\n    }\n', start);
bridge.run('var _log = window._eoLog;\n' + source.slice(start, end + 6) + '\nwindow._installFrameDiagnostics = _installFrameDiagnostics;');
const install = bridge.sandbox._installFrameDiagnostics;

class FakeWindow {
  constructor(href) {
    this.listeners = [];
    this.navigate(href);
  }
  navigate(href) {
    this.document = {};
    this.location = { href, pathname: href };
  }
  addEventListener(type, fn, capture) { this.listeners.push({ type, fn, capture }); }
  removeEventListener(type, fn) { this.listeners = this.listeners.filter((l) => !(l.type === type && l.fn === fn)); }
  fire(type, event) { this.listeners.filter((l) => l.type === type).forEach((l) => l.fn(event)); }
}

const errorEvent = { type: 'error', message: 'boom', filename: 'file:///app/sdkjs/word/sdk-all.js', lineno: 1, colno: 2 };
const countErrors = () => bridge.logs.filter((line) => line.startsWith('[IFRAME-ERROR]')).length;

// Same window object, new document (initial about:blank reused).
const kept = new FakeWindow('about:blank');
install(kept);
kept.navigate('file:///app/web-apps/apps/documenteditor/main/index.html');
install(kept);
let before = countErrors();
kept.fire('error', errorEvent);
check(results, 'kept window logs an error after navigation exactly once', countErrors() - before === 1,
  String(countErrors() - before));
check(results, 'kept window has one error and one rejection listener',
  kept.listeners.length === 2, String(kept.listeners.length));

// A second install on the same document changes nothing.
install(kept);
check(results, 'reinstall on the same document adds no listener', kept.listeners.length === 2);

// New window object after navigation.
const fresh = new FakeWindow('file:///app/web-apps/apps/documenteditor/main/index.html');
install(fresh);
before = countErrors();
fresh.fire('error', errorEvent);
check(results, 'new window logs an error', countErrors() - before === 1);

const installedLines = bridge.logs.filter((line) => line.startsWith('[EO] iframe diagnostics installed'));
check(results, 'install is logged once per document', installedLines.length === 3, String(installedLines.length));

finish(results);
