// Drives the two paste paths of src/bridge.js that never see a paste event,
// AscDesktopEditor.Paste() (the Paste button and context menu, every platform)
// and Ctrl+V with a non-Latin layout on Linux, with an editor copy that only
// reaches the system clipboard as HTML (a slide, an image, a chart: no text,
// no image flavor). Checks that:
//
//   - the editor copy goes to sdkjs's HTML paste once, with the sequence of
//     its own paste handler (flag, long-action counter, pastedFrom, iframe
//     paste), and leaves the counter where it was;
//   - HTML that is not an editor copy (a browser's "Copy Image") does not;
//   - text still wins when there is text, and an empty clipboard stays empty;
//   - the decision itself (_eoHtmlIsEditorCopy) on the shapes sdkjs writes.
//
// The fake CommonIframe_PasteStart ends in Paste_End, as sdkjs's does.
//
// Run: node scripts/test-bridge-internal-html-paste.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];

const slideCopy = '<img width="1284" height="724" src="data:image/png;base64,iVBORw0KGgo=" class="pptData;20600;RE9DWQ==">';
const docCopy = '<span style="font-size:11pt" class="docData;1675;RE9DWQ==">x</span>';
const cellCopy = '<html><body><span class="xslData;7067;RE9DWQ==">1</span></body></html>\0\0';
const browserImage = '<meta charset="utf-8"><img src="http://127.0.0.1:8765/img.png" alt="">';
const webText = '<p class="MsoNormal" style="margin:0">Hello</p>';

// Clipboard contents for the next reads; each read is counted.
let clip = {};
let reads = {};
function reader(name, key) {
  return () => { reads[name] = (reads[name] || 0) + 1; return clip[key] === undefined ? null : clip[key]; };
}
const invoke = {
  read_clipboard_text: reader('text', 'text'),
  read_clipboard_image: reader('image', 'image'),
  read_clipboard_html: reader('html', 'html'),
};

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// An editor window as _loadEditorBin finds it, with the parts of sdkjs's
// g_clipboardBase and api that the HTML paste uses, recording each step.
const editorWindowSource = `
  var __listeners = {};
  var __editorDoc = {
    addEventListener: function(type, fn) { (__listeners[type] = __listeners[type] || []).push(fn); },
  };
  var __steps = [];
  function __makeEditorWindow() {
    var editor = { inserted: [], pasted: [], counter: 0, openDocument: function() {} };
    editor.AddImageUrl = function(list) { editor.inserted.push(list[0]); };
    editor.asc_PasteData = function(format, data) { editor.pasted.push([format, data]); };
    editor.isLongAction = function() { return editor.counter > 0; };
    editor.incrementCounterLongAction = function() { __steps.push('increment'); editor.counter++; };
    editor.decrementCounterLongAction = function() { __steps.push('decrement'); editor.counter--; };
    var cb = {
      Api: editor, PasteFlag: false, pastedFrom: 'stale',
      isUseNewCopy: function() { return false; },
      CommonIframe_PasteStart: function(html, text) {
        __steps.push({ start: { flag: cb.PasteFlag, counter: editor.counter, pastedFrom: cb.pastedFrom, html: html, text: text } });
        if (cb.__throw) throw new Error('boom');
        cb.Paste_End();
      },
      Paste_End: function() { __steps.push('end'); editor.decrementCounterLongAction(); cb.PasteFlag = false; },
    };
    return {
      Asc: { editor: editor },
      document: __editorDoc,
      AscCommon: {
        g_oDocumentUrls: { documentUrl: '', getImageUrl: function() {}, getUrl: function() {} },
        OpenFileResult: function() {},
        History: {},
        c_oAscClipboardDataFormat: { Text: 1, Html: 2, Internal: 4 },
        g_clipboardBase: cb,
      },
    };
  }
  window.AscDesktopEditor._editorWindow = __makeEditorWindow();
`;

async function makeBridge(platform) {
  const bridge = await loadBridge({ platform, invoke });
  bridge.sandbox.__xhr = (url) => (url.endsWith('doc-key') ? { status: 200, body: '1000-1/2' } : null);
  bridge.run(editorWindowSource);
  return bridge;
}

function reset(bridge) {
  const sb = bridge.sandbox;
  sb.__steps.length = 0;
  const ew = sb.AscDesktopEditor._editorWindow;
  ew.Asc.editor.inserted.length = 0;
  ew.Asc.editor.pasted.length = 0;
  ew.Asc.editor.counter = 0;
  ew.AscCommon.g_clipboardBase.PasteFlag = false;
  ew.AscCommon.g_clipboardBase.pastedFrom = 'stale';
  ew.AscCommon.g_clipboardBase.__throw = false;
  bridge.logs.length = 0;
  reads = {};
}

function state(bridge) {
  const sb = bridge.sandbox;
  const ew = sb.AscDesktopEditor._editorWindow;
  const starts = sb.__steps.filter((step) => typeof step === 'object').map((step) => step.start);
  return {
    steps: sb.__steps.map((step) => (typeof step === 'object' ? 'start' : step)).join(','),
    starts,
    counter: ew.Asc.editor.counter,
    flag: ew.AscCommon.g_clipboardBase.PasteFlag,
    inserted: ew.Asc.editor.inserted.length,
    pasted: ew.Asc.editor.pasted.length,
  };
}

function resultLine(bridge, prefix) {
  const line = bridge.logs.find((entry) => entry.includes(prefix));
  return line ? line.slice(line.indexOf(prefix) + prefix.length) : '(none)';
}

// The full sdkjs sequence, once, with the counter back where it was.
function checkSequence(label, bridge, html) {
  const s = state(bridge);
  const start = s.starts[0];
  check(results, `${label}: one CommonIframe_PasteStart`, s.starts.length === 1, `calls=${s.starts.length}`);
  check(results, `${label}: flag set, counter raised, pastedFrom cleared before it`,
    !!start && start.flag === true && start.counter === 1 && start.pastedFrom === null,
    start ? `flag=${start.flag}, counter=${start.counter}, pastedFrom=${start.pastedFrom}` : '');
  check(results, `${label}: given the clipboard HTML`, !!start && start.html === html && start.text === '');
  check(results, `${label}: Paste_End balanced it`,
    s.steps === 'increment,start,end,decrement' && s.counter === 0 && s.flag === false, s.steps);
}

// --- AscDesktopEditor.Paste() on each platform ------------------------------

for (const platform of ['Linux x86_64', 'Win32', 'MacIntel']) {
  const bridge = await makeBridge(platform);
  const sb = bridge.sandbox;
  const paste = () => sb.AscDesktopEditor.Paste();
  const name = platform.split(' ')[0];

  // (i) No text, no image, an editor copy as HTML.
  reset(bridge);
  clip = { text: null, image: null, html: slideCopy };
  await paste();
  checkSequence(`(i) Paste() ${name}, slide copy`, bridge, slideCopy);
  check(results, `(i) Paste() ${name}: result=internal-html`,
    resultLine(bridge, '[CLIPBOARD] paste result=') === 'internal-html', resultLine(bridge, '[CLIPBOARD] paste result='));

  // The cell copy as Linux hands it over: whole document plus trailing bytes,
  // cut after </html> as sdkjs cuts an event's HTML.
  reset(bridge);
  clip = { text: '', image: null, html: cellCopy };
  await paste();
  const cut = cellCopy.slice(0, cellCopy.indexOf('</html>') + 7);
  check(results, `(i) Paste() ${name}, cell copy with empty text: pasted, cut after </html>`,
    state(bridge).starts.length === 1 && state(bridge).starts[0].html === cut);

  // (iii) A browser's "Copy Image" HTML (no binary tag).
  reset(bridge);
  clip = { text: null, image: null, html: browserImage };
  await paste();
  check(results, `(iii) Paste() ${name}, browser image HTML: not pasted as HTML`,
    state(bridge).starts.length === 0 && state(bridge).counter === 0);
  check(results, `(iii) Paste() ${name}: result=empty`,
    resultLine(bridge, '[CLIPBOARD] paste result=') === 'empty', resultLine(bridge, '[CLIPBOARD] paste result='));

  // (iv) Text present: text wins, the HTML is not even read.
  reset(bridge);
  clip = { text: 'Hello', image: null, html: slideCopy };
  await paste();
  check(results, `(iv) Paste() ${name}, text present: pasted as text`,
    resultLine(bridge, '[CLIPBOARD] paste result=') === 'text' && state(bridge).pasted === 1 &&
    state(bridge).starts.length === 0 && !reads.html,
    `result=${resultLine(bridge, '[CLIPBOARD] paste result=')}, html reads=${reads.html || 0}`);

  // An image present: the image wins, as today.
  reset(bridge);
  clip = { text: null, image: 'clipboard_1.png', html: slideCopy };
  await paste();
  check(results, `Paste() ${name}, image present: pasted as image`,
    resultLine(bridge, '[CLIPBOARD] paste result=') === 'image' && state(bridge).inserted === 1 &&
    state(bridge).starts.length === 0);

  // (v) Nothing at all.
  reset(bridge);
  clip = { text: null, image: null, html: null };
  await paste();
  check(results, `(v) Paste() ${name}, empty clipboard: result=empty`,
    resultLine(bridge, '[CLIPBOARD] paste result=') === 'empty' && state(bridge).starts.length === 0);

  bridge.stopTimers();
}

// --- Busy editor and a throwing paste (Linux bridge, Paste()) ---------------
{
  const bridge = await makeBridge('Linux x86_64');
  const sb = bridge.sandbox;
  const cb = sb.AscDesktopEditor._editorWindow.AscCommon.g_clipboardBase;

  reset(bridge);
  sb.AscDesktopEditor._editorWindow.Asc.editor.counter = 1;
  clip = { text: null, image: null, html: slideCopy };
  await sb.AscDesktopEditor.Paste();
  check(results, 'editor in a long action: nothing pasted, counter untouched',
    state(bridge).starts.length === 0 && state(bridge).counter === 1 &&
    resultLine(bridge, '[CLIPBOARD] paste result=') === 'internal-html-busy',
    resultLine(bridge, '[CLIPBOARD] paste result='));

  reset(bridge);
  cb.__throw = true;
  await sb.AscDesktopEditor.Paste();
  check(results, 'a throw inside the iframe paste still ends the paste (no locked editor)',
    state(bridge).counter === 0 && state(bridge).flag === false &&
    resultLine(bridge, '[CLIPBOARD] paste result=') === 'internal-html-error',
    `counter=${state(bridge).counter}, ${resultLine(bridge, '[CLIPBOARD] paste result=')}`);
  bridge.stopTimers();
}

// --- Ctrl+V with a non-Latin layout (Linux) ---------------------------------
{
  const bridge = await makeBridge('Linux x86_64');
  const sb = bridge.sandbox;
  bridge.run("_loadEditorBin('', 'test.pptx', { skipRecovery: true });");
  if (!(sb.__listeners.keydown || []).length) throw new Error('the bridge registered no keydown listener');

  // Ctrl + the physical V key on a Russian layout: key is "м".
  async function cyrillicCtrlV() {
    const event = {
      ctrlKey: true, metaKey: false, shiftKey: false, altKey: false, key: 'м', code: 'KeyV', keyCode: 77,
      getModifierState: () => false, preventDefault() {}, stopPropagation() {},
    };
    for (const listener of sb.__listeners.keydown) listener(event);
    await sleep(50);
  }
  const keyResult = () => resultLine(bridge, '[EO] KeyShim v: result=');

  // (ii) Editor copy as HTML only.
  reset(bridge);
  clip = { text: null, image: null, html: slideCopy };
  await cyrillicCtrlV();
  checkSequence('(ii) KeyShim v, slide copy', bridge, slideCopy);
  check(results, '(ii) KeyShim v: result=internal-html', keyResult() === 'internal-html', keyResult());

  reset(bridge);
  clip = { text: null, image: null, html: docCopy };
  await cyrillicCtrlV();
  check(results, '(ii) KeyShim v, document copy: pasted as HTML', state(bridge).starts.length === 1, keyResult());

  // (iii) Browser image HTML.
  reset(bridge);
  clip = { text: null, image: null, html: browserImage };
  await cyrillicCtrlV();
  check(results, '(iii) KeyShim v, browser image HTML: not pasted as HTML, result=empty',
    state(bridge).starts.length === 0 && keyResult() === 'empty', keyResult());

  // (iv) Text wins.
  reset(bridge);
  clip = { text: 'Hello', image: null, html: slideCopy };
  await cyrillicCtrlV();
  check(results, '(iv) KeyShim v, text present: result=text, HTML not read',
    keyResult() === 'text' && state(bridge).starts.length === 0 && !reads.html, keyResult());

  // Image wins.
  reset(bridge);
  clip = { text: null, image: 'clipboard_2.png', html: slideCopy };
  await cyrillicCtrlV();
  check(results, 'KeyShim v, image present: result=image',
    keyResult() === 'image' && state(bridge).inserted === 1 && state(bridge).starts.length === 0, keyResult());

  // (v) Empty.
  reset(bridge);
  clip = { text: null, image: null, html: null };
  await cyrillicCtrlV();
  check(results, '(v) KeyShim v, empty clipboard: result=empty',
    keyResult() === 'empty' && state(bridge).starts.length === 0, keyResult());

  bridge.stopTimers();
}

// --- The decision on its own -------------------------------------------------
{
  const bridge = await makeBridge('Linux x86_64');
  const isCopy = bridge.sandbox._eoHtmlIsEditorCopy;
  check(results, '_eoHtmlIsEditorCopy exists', typeof isCopy === 'function');
  if (typeof isCopy === 'function') {
    const cases = [
      ['slide (pptData, <img>)', slideCopy, true],
      ['document (docData, <span>)', docCopy, true],
      ['spreadsheet (xslData, whole document)', cellCopy, true],
      ['PDF (pdfData, single quotes)', "<div class='pdfData;10;AAA'></div>", true],
      ['browser "Copy Image"', browserImage, false],
      ['web text with a class', webText, false],
      ['tag name outside a class', '<p>pptData;20600;AAA</p>', false],
      ['tag word without its separator', '<img class="pptData" src="x.png">', false],
      ['empty', '', false],
      ['null', null, false],
    ];
    for (const [label, html, expected] of cases) {
      check(results, `decision: ${label} -> ${expected}`, isCopy(html) === expected);
    }
  }
  bridge.stopTimers();
}

finish(results);
