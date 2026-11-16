// Checks that the style hiding the PDF form controls that need document
// scripts (editor-patches.js) reaches the PDF editor document. The frame
// starts on about:blank and keeps its window object when it navigates, so
// the check for the PDF editor has to wait for the live document, and the
// "already hidden" flag must only be set once the style is in.
//
// Run: node scripts/test-bridge-pdf-form-controls.mjs [path/to/bridge.js]
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

const source = (await readFile(path.join(projectRoot, 'src/editor-patches.js'), 'utf8')).replace(/\r\n/g, '\n');

function slice(startMarker, endMarker, endLength) {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start);
  if (start < 0 || end < 0) throw new Error('editor-patches.js: cannot find ' + startMarker.trim());
  return source.slice(start, end + endLength);
}

// The helpers between _isPdfEditorFrame and injectBridgeDeep, whatever
// their names, and the block that uses them.
function loadPdfControlsBlock() {
  const helpers = slice('    function _isPdfEditorFrame(win) {', '    function injectBridgeDeep(win) {', 0);
  const block = slice('          if (!win._eoPdfScriptFeaturesHidden', '\n          }\n', 12);
  bridge.run(helpers + '\nwindow._hidePdfFormControls = function(win) {\n' + block + '};');
  return bridge.sandbox._hidePdfFormControls;
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
    this.location = { href: 'about:blank', pathname: 'blank' };
    this._eoLog = (...args) => bridge.logs.push(args.join(' '));
  }
  // Same window object, new document, and the SDK comes up on it.
  navigateTo(editor) {
    const page = '/app/web-apps/apps/' + editor + '/main/index.html';
    this.document = new FakeDocument();
    this.location = { href: 'file://' + page, pathname: page };
    this.Asc = { editor: {} };
  }
}

const hide = loadPdfControlsBlock();

// Runs the block on a frame still on about:blank, then navigates it.
async function openFrame(editor) {
  const win = new FakeWindow();
  const blank = win.document;
  hide(win);
  // Several ticks of the 50 ms timer while about:blank is up.
  await sleep(200);
  const flagOnBlank = win._eoPdfScriptFeaturesHidden === true;
  win.navigateTo(editor);
  await sleep(200);
  return { win, blank, editor: win.document, flagOnBlank };
}

const hiddenLogs = () => bridge.logs.filter((line) => line.startsWith('[EO] PDF form controls hidden')).length;

let logsBefore = hiddenLogs();
const pdf = await openFrame('pdfeditor');
const pdfLogs = hiddenLogs() - logsBefore;
check(results, 'PDF editor: the style lands in the editor document',
  pdf.editor.hasRule('#slot-btn-form-datetime') && pdf.editor.hasRule('#form-txt-pholder'),
  pdf.editor.styles.length + ' style(s) in the editor, ' + pdf.blank.styles.length + ' in about:blank');
check(results, '... and not in about:blank', pdf.blank.styles.length === 0, String(pdf.blank.styles.length));
check(results, '... the flag is not set while on about:blank', !pdf.flagOnBlank);
check(results, '... and is set once the style is in', pdf.win._eoPdfScriptFeaturesHidden === true);
check(results, '... and it is logged once', pdfLogs === 1, String(pdfLogs));

logsBefore = hiddenLogs();
const word = await openFrame('documenteditor');
const wordLogs = hiddenLogs() - logsBefore;
check(results, 'Word editor: no style', word.editor.styles.length === 0 && word.blank.styles.length === 0,
  word.editor.styles.length + ' in the editor, ' + word.blank.styles.length + ' in about:blank');
check(results, '... and no flag', word.win._eoPdfScriptFeaturesHidden !== true);
check(results, '... and nothing logged', wordLogs === 0, String(wordLogs));

for (const timer of intervals) clearInterval(timer);
bridge.stopTimers();
finish(results);
