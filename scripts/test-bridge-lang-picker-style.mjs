// Checks that the style that hides the unsized spellcheck indicators of the
// language pickers (editor-patches.js) ends up in the editor document, not in
// the about:blank the frame starts with: the window object is kept across
// that navigation, so a style put into about:blank is lost with it and the
// "already styled" flag must not be set.
//
// Run: node scripts/test-bridge-lang-picker-style.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const bridge = await loadBridge();

const intervals = new Set();
bridge.sandbox.setInterval = (fn, ms) => {
  const timer = setInterval(fn, ms);
  intervals.add(timer);
  return timer;
};
bridge.sandbox.clearInterval = (timer) => { intervals.delete(timer); clearInterval(timer); };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function loadStyleBlock() {
  const marker = '          if (!win._eoLangPickerStyled) {';
  const start = source.indexOf(marker);
  const end = source.indexOf('\n          }\n', start);
  if (start < 0 || end < 0) throw new Error('editor-patches.js: cannot find the language picker style block');
  bridge.run('window._installLangPickerStyle = function(win) {\n' + source.slice(start, end + 12) + '};');
  return bridge.sandbox._installLangPickerStyle;
}

class FakeDocument {
  constructor() {
    this.styles = [];
    this.head = { appendChild: (node) => this.styles.push(node) };
  }
  createElement(tag) { return { tagName: tag, textContent: '' }; }
  hasRule(selector) { return this.styles.some((style) => style.textContent.includes(selector)); }
}

class FakeWindow {
  constructor() {
    this.document = new FakeDocument();
    this._eoLog = (...args) => bridge.logs.push(args.join(' '));
  }
  // The frame leaves about:blank for the editor page: same window object,
  // new document, and the SDK comes up on it.
  navigateToEditor() {
    this.document = new FakeDocument();
    this.Asc = { editor: {} };
  }
}

const source = (await readFile(path.join(projectRoot, 'src/editor-patches.js'), 'utf8')).replace(/\r\n/g, '\n');
const install = loadStyleBlock();

const win = new FakeWindow();
const blank = win.document;
install(win);
// Long enough for several ticks of the 50 ms timer while about:blank is up.
await sleep(200);
win.navigateToEditor();
const editor = win.document;
await sleep(200);

check(results, 'the editor document gets the status bar menu rule', editor.hasRule('.lang-menu .icon'),
  editor.styles.length + ' style(s) in the editor, ' + blank.styles.length + ' in about:blank');
check(results, '... and the language dialog rule in the same style',
  editor.styles.length === 1 && editor.hasRule('.combo-langs .input-icon'), String(editor.styles.length));
check(results, 'about:blank gets no style', blank.styles.length === 0, String(blank.styles.length));
check(results, 'the flag is set once the style is in', win._eoLangPickerStyled === true);

const installedLines = bridge.logs.filter((line) => line.startsWith('[EO] Language picker style installed'));
check(results, 'the install is logged once', installedLines.length === 1, String(installedLines.length));

// A second pass on the same window adds nothing.
install(win);
await sleep(200);
check(results, 'a second pass adds no style', editor.styles.length === 1, String(editor.styles.length));

for (const timer of intervals) clearInterval(timer);
bridge.stopTimers();
finish(results);
