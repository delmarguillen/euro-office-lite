// Drives the real Linux Ctrl+V path of src/bridge.js (the keydown that reads
// the native clipboard image and the paste listener that guards sdkjs's own
// paste) with simulated events and clipboard contents. It checks what does not
// depend on parsing HTML: the editor's copy is given here by its text/x-custom
// tag. Telling the copy apart by its HTML runs on the webview's own DOMParser
// and is checked in a real webview through window._eoIsEditorCopy.
//
//   - the editor's own copy reaches sdkjs, and the native image of the same
//     keystroke (a picture of the selection) is never inserted, whichever of
//     the two finishes first;
//   - an image copied elsewhere (Issue #24) is inserted once, by the native
//     read, and sdkjs's paste of it is blocked;
//   - each paste event goes with its own keystroke, also when two overlap;
//   - the image goes into the document the key was pressed in, or nowhere;
//   - with no paste event at all, the native image still goes in once.
//
// Run: node scripts/test-bridge-linux-paste-guard.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
let nextRead = null;
const bridge = await loadBridge({
  invoke: { read_clipboard_image: () => nextRead() },
});
bridge.sandbox.__xhr = (url) => (url.endsWith('doc-key') ? { status: 200, body: '1000-1/2' } : null);

// An editor window as _loadEditorBin finds it; each has its own document.
bridge.run(`
  var __listeners = {};
  var __editorDoc = {
    addEventListener: function(type, fn) { (__listeners[type] = __listeners[type] || []).push(fn); },
  };
  function __makeEditorWindow(documentUrl) {
    var editor = { inserted: [], openDocument: function() {} };
    editor.AddImageUrl = function(list) { editor.inserted.push(list[0]); };
    return {
      Asc: { editor: editor },
      document: __editorDoc,
      AscCommon: {
        g_oDocumentUrls: { documentUrl: documentUrl, getImageUrl: function() {}, getUrl: function() {} },
        OpenFileResult: function() {},
        History: {},
        c_oAscClipboardDataFormat: { Text: 1, Html: 2, Internal: 4 },
        g_clipboardBase: { isUseNewCopy: function() { return false; } },
      },
    };
  }
  window.AscDesktopEditor._editorWindow = __makeEditorWindow('');
  _loadEditorBin('', 'test.pptx', { skipRecovery: true });
`);
const sb = bridge.sandbox;
if (!(sb.__listeners.keydown || []).length || !(sb.__listeners.paste || []).length) {
  throw new Error('the bridge registered no keydown/paste listeners on the editor document');
}
const firstEditorWindow = sb.AscDesktopEditor._editorWindow;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function dispatch(type, event) {
  for (const listener of sb.__listeners[type] || []) {
    if (event.stopped) break;
    listener(event);
  }
}

function keydown() {
  dispatch('keydown', {
    ctrlKey: true, metaKey: false, shiftKey: false, altKey: false, key: 'v', code: 'KeyV',
    getModifierState: () => false, preventDefault() {}, stopPropagation() {},
  });
}

// Returns true when the paste event reached sdkjs (nothing stopped it).
function pasteEvent(clip) {
  const paste = {
    clipboardData: { getData: (type) => clip[type] || '', files: [] },
    defaultPrevented: false,
    stopped: false,
    preventDefault() { this.defaultPrevented = true; },
    stopPropagation() { this.stopped = true; },
  };
  dispatch('paste', paste);
  return !paste.defaultPrevented && !paste.stopped;
}

// Native reads resolve in order, when `release` is called or right away.
function nativeReads(image, immediate) {
  const waiting = [];
  nextRead = () => (immediate ? image : new Promise((resolve) => waiting.push(() => resolve(image))));
  return () => { while (waiting.length) waiting.shift()(); };
}

function reset() {
  sb.AscDesktopEditor._editorWindow = firstEditorWindow;
  firstEditorWindow.Asc.editor.inserted.length = 0;
  bridge.logs.length = 0;
}

const editorCopy = {
  'text/html': '<img src="data:image/png;base64,iVBORw0KGgo=" class="pptData;20600;RE9DWQ==">',
  'text/x-custom': 'asc_internalData2;RE9DWQ==',
};
const externalImage = { 'text/html': '<img src="http://127.0.0.1:8765/img.png">' };
const inserted = () => firstEditorWindow.Asc.editor.inserted.length;

check(results, 'window._eoIsEditorCopy is there for the inspector',
  typeof sb._eoIsEditorCopy === 'function');

// The editor's own copy, with a native image in the clipboard too.
for (const nativeFirst of [false, true]) {
  reset();
  const release = nativeReads('clipboard_1.png', nativeFirst);
  keydown();
  await sleep(15);
  const reached = pasteEvent(editorCopy);
  release();
  await sleep(500);
  const when = nativeFirst ? 'native read first' : 'paste event first';
  check(results, `editor copy reaches sdkjs (${when})`, reached);
  check(results, `... and no native picture of it goes in (${when})`, inserted() === 0, `inserted=${inserted()}`);
}

// Issue #24.
for (const nativeFirst of [false, true]) {
  reset();
  const release = nativeReads('clipboard_2.png', nativeFirst);
  keydown();
  await sleep(15);
  const reached = pasteEvent(externalImage);
  release();
  await sleep(500);
  const when = nativeFirst ? 'native read first' : 'paste event first';
  const line = bridge.logs.find((entry) => entry.includes('clipboard image inserted'));
  check(results, `external image: sdkjs paste blocked (${when})`, !reached);
  check(results, `... inserted once by the native read (${when})`, inserted() === 1,
    `inserted=${inserted()}${line ? '; ' + line.replace(/^.*\(/, '(') : ''}`);
}

// (i) The document changes while the keystroke waits for its paste event.
{
  reset();
  nativeReads('clipboard_3.png', true);
  keydown();
  await sleep(15);
  const next = sb.__makeEditorWindow('ascdesktop://docmedia/1000-1/3');
  sb.AscDesktopEditor._editorWindow = next;
  pasteEvent(externalImage);
  await sleep(500);
  check(results, '(i) document changed while waiting: nothing goes into the new one',
    next.Asc.editor.inserted.length === 0 && inserted() === 0,
    `new=${next.Asc.editor.inserted.length}, old=${inserted()}`);
}

// (ii) Two overlapping keystrokes (20 ms apart), two editor copies.
{
  reset();
  const release = nativeReads('clipboard_4.png', false);
  keydown();
  await sleep(20);
  keydown();
  await sleep(15);
  const first = pasteEvent(editorCopy);
  const second = pasteEvent(editorCopy);
  release();
  await sleep(500);
  check(results, '(ii) two overlapping editor copies both reach sdkjs', first && second);
  check(results, '(ii) ... and no native image goes in', inserted() === 0, `inserted=${inserted()}`);
}

// (iii) Two overlapping keystrokes, two external images: two insertions in
// all, counting any external paste that reaches sdkjs as one.
{
  reset();
  const release = nativeReads('clipboard_5.png', false);
  keydown();
  await sleep(20);
  keydown();
  await sleep(15);
  const viaSdk = [pasteEvent(externalImage), pasteEvent(externalImage)].filter(Boolean).length;
  release();
  await sleep(500);
  check(results, '(iii) two overlapping external images: exactly two insertions', inserted() + viaSdk === 2,
    `native=${inserted()}, through sdkjs=${viaSdk}`);
}

// (iv) No paste event at all: the native image still goes in, once, at the end
// of the wait.
{
  reset();
  nativeReads('clipboard_6.png', true);
  keydown();
  await sleep(600);
  const line = bridge.logs.find((entry) => entry.includes('clipboard image inserted'));
  check(results, '(iv) no paste event: the native image goes in once', inserted() === 1,
    line ? line.replace(/^.*\(/, '(') : `inserted=${inserted()}`);
}

bridge.stopTimers();
finish(results);
