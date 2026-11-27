// Checks the phase marks of a document open (bridge.js, _eoOpenPhases): the
// line of each phase, the id the backend gets, that dt is the time since the
// previous mark of the same open, and that a failed or finished open marks
// nothing more. Also the marks taken from the events of the editor API, the
// two between app_ready and doc_request among them, and
// the sdk_min and sdk_all marks editor-patches.js takes from the globals sdkjs
// defines in the editor frame.
//
// Run: node scripts/test-bridge-open-phases.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const REPORT = { token: 'tok-report', name: 'report.docx' };
const PHASE_LINE = /^\[OPEN\] phase=(\w+) open=([0-9a-z]+-\d+) t=(\d+) dt=(\d+)(?: wall=(\d+))?$/;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function phaseLines(bridge) {
  return bridge.logs
    .filter((line) => line.startsWith('[OPEN] phase='))
    .map((line) => {
      const match = PHASE_LINE.exec(line);
      if (!match) return { malformed: line };
      const [, phase, open, t, dt, wall] = match;
      return { phase, open, t: Number(t), dt: Number(dt), wall: wall ? Number(wall) : null };
    });
}

const phaseNames = (bridge) => phaseLines(bridge).map((line) => line.phase).join();

async function loadWith(openFile) {
  const calls = [];
  const bridge = await loadBridge({
    invoke: {
      fonts_status: () => 'ready',
      open_file: (args) => { calls.push(args); return openFile(); },
    },
  });
  bridge.run('window._openEditor = function() {}; window._eoShowOpenError = function() { return Promise.resolve(); };');
  return { bridge, calls };
}

// An open that reaches the editor.
{
  const { bridge, calls } = await loadWith(() => 'Ymlu');
  const before = Date.now();
  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(20);
  bridge.run("_eoOpenPhases.mark('fonts')");
  await sleep(20);
  bridge.run("_eoOpenPhases.mark('iframe'); _eoOpenPhases.mark('iframe');");
  bridge.run("_eoOpenPhases.mark('document_ready'); _eoOpenPhases.end(); _eoOpenPhases.mark('app_ready');");
  bridge.stopTimers();
  const lines = phaseLines(bridge);

  check(results, 'every phase line has the expected form',
    lines.length > 0 && lines.every((line) => !line.malformed), JSON.stringify(lines));
  check(results, 'the phases are logged once each, in the order they were marked',
    phaseNames(bridge) === 'request,result,fonts,iframe,document_ready', phaseNames(bridge));
  const request = lines[0];
  check(results, 'the request is t=0 dt=0 and carries the wall clock',
    request.t === 0 && request.dt === 0 && request.wall >= before && request.wall <= Date.now(),
    JSON.stringify(request));
  check(results, 'only the request carries the wall clock',
    lines.slice(1).every((line) => line.wall === null));
  check(results, 'the backend gets the id the page logs under',
    calls.length === 1 && calls[0].openId === request.open && calls[0].token === REPORT.token,
    JSON.stringify(calls));
  check(results, 'every line of the open has the same id',
    lines.every((line) => line.open === request.open));
  const steps = lines.slice(1).map((line, index) => line.t - lines[index].t);
  check(results, 'dt is the time since the previous mark',
    lines.slice(1).every((line, index) => line.dt === steps[index]), JSON.stringify(lines));
  check(results, 'time went by between the marks', lines[lines.length - 1].t >= 30,
    String(lines[lines.length - 1].t));
  const found = (text) => bridge.logs.some((line) => line.includes(text));
  check(results, 'no phase line reads as a start, success, failed or ready line',
    !found('[OPEN] start') && !found('[OPEN] success') && !found('[OPEN] failed') && !found('Document ready'));
}

// Two opens of one page.
{
  const { bridge } = await loadWith(() => 'Ymlu');
  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  bridge.stopTimers();
  const ids = phaseLines(bridge).filter((line) => line.phase === 'request').map((line) => line.open);
  check(results, 'each open has its own id', ids.length === 2 && ids[0] !== ids[1], ids.join());
}

// An open the backend refuses.
{
  const { bridge } = await loadWith(() => { throw new Error('not a document'); });
  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(20);
  bridge.run("_eoOpenPhases.mark('app_ready')");
  bridge.stopTimers();
  check(results, 'a refused open logs its request and nothing after it',
    phaseNames(bridge) === 'request', phaseNames(bridge));
}

// No open: a new document reaches the same editor events.
{
  const { bridge } = await loadWith(() => 'Ymlu');
  bridge.run("_eoOpenPhases.mark('fonts'); _eoOpenPhases.mark('document_ready');");
  bridge.stopTimers();
  check(results, 'without an open nothing is marked', phaseLines(bridge).length === 0, phaseNames(bridge));
}

// The marks taken from the events of the editor API (_eoWatchOpenPhases). The
// API here sends an event the way sdkjs does: to the listeners in the order
// they were added, a listener added meanwhile included.
const FAKE_API = `
  window.AscDesktopEditor._editorWindow = {
    Asc: { c_oAscAsyncAction: { Open: 0, LoadDocumentFonts: 2, LoadDocumentImages: 3 } }
  };
  window.__api = {
    callbacks: {},
    asc_registerCallback: function(name, fn) {
      (this.callbacks[name] = this.callbacks[name] || []).push(fn);
    },
    sendEvent: function(name) {
      var list = this.callbacks[name] || [];
      var args = Array.prototype.slice.call(arguments, 1);
      for (var i = 0; i < list.length; i++) list[i].apply(this, args);
    }
  };
  __api.asc_registerCallback('asc_onDocumentContentReady', function() { _eoLog('editor UI handler'); });
  _eoWatchOpenPhases(__api);
  _eoWatchOpenPhases(__api);
`;
const END_FONTS = "__api.sendEvent('asc_onEndAction', 1, 2);";
const END_IMAGES = "__api.sendEvent('asc_onEndAction', 1, 3);";
const CONTENT_READY = "__api.sendEvent('asc_onEndAction', 1, 0); __api.sendEvent('asc_onDocumentContentReady'); _eoLog('task end');";
const listeners = (bridge, name) => (bridge.sandbox.__api.callbacks[name] || []).length;

{
  const { bridge } = await loadWith(() => 'Ymlu');
  bridge.run(FAKE_API);
  check(results, 'watching an API twice adds one listener',
    listeners(bridge, 'asc_onEndAction') === 1, String(listeners(bridge, 'asc_onEndAction')));
  bridge.run(END_FONTS + CONTENT_READY);
  await sleep(0);
  check(results, 'without an open the editor events mark nothing', phaseLines(bridge).length === 0, phaseNames(bridge));

  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  bridge.run(END_FONTS);
  check(results, 'a font action before the document is decoded is not the document\'s',
    phaseNames(bridge) === 'request,result', phaseNames(bridge));
  bridge.run('window.AscDesktopEditor.LocalStartOpen();');
  bridge.run("_eoOpenPhases.mark('doc_decoded'); _eoOpenPhases.mark('doc_parsed');");
  bridge.run(END_FONTS + END_FONTS + CONTENT_READY);
  await sleep(0);
  bridge.run("_eoOpenPhases.mark('spell_init'); _eoOpenPhases.mark('document_ready'); _eoOpenPhases.end();");
  check(results, 'an open without images marks every phase of the document but doc_images, in order',
    phaseNames(bridge) === 'request,result,doc_request,doc_decoded,doc_parsed,doc_fonts,content_ready,ui_ready,layout,spell_init,document_ready',
    phaseNames(bridge));
  const at = (text, from = 0) => bridge.logs.findIndex((line, index) => index >= from && line.includes(text));
  const handler = at('editor UI handler', at('phase=content_ready'));
  check(results, 'content_ready comes before the handlers of the event and ui_ready after them',
    handler === at('phase=content_ready') + 1 && at('phase=ui_ready') === handler + 1,
    `${at('phase=content_ready')} ${handler} ${at('phase=ui_ready')}`);
  check(results, 'layout is marked when the task that sent the event has ended',
    at('task end') < at('phase=layout'));
  const lines = phaseLines(bridge);
  check(results, 'the dts of the open add up to its last t',
    lines.reduce((sum, line) => sum + line.dt, 0) === lines[lines.length - 1].t, JSON.stringify(lines));

  // The next document of the same editor frame, with images.
  const first = phaseLines(bridge).length;
  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  bridge.run("_eoOpenPhases.mark('doc_decoded');");
  bridge.run(END_FONTS + END_IMAGES + CONTENT_READY + CONTENT_READY);
  await sleep(0);
  bridge.stopTimers();
  const second = phaseLines(bridge).slice(first).map((line) => line.phase).join();
  check(results, 'an open with images marks doc_images, and a second Open action marks nothing',
    second === 'request,result,doc_decoded,doc_fonts,doc_images,content_ready,ui_ready,layout', second);
  check(results, 'the listener of ui_ready is added once per API',
    listeners(bridge, 'asc_onDocumentContentReady') === 2, String(listeners(bridge, 'asc_onDocumentContentReady')));
}

// The marks between app_ready and doc_request. sdkjs sends
// asc_onGetEditorPermissions once it has the document's info, and the editor
// UI, which listens after the bridge, ends its work there with the Open action.
{
  const PERMISSIONS = "__api.sendEvent('asc_onGetEditorPermissions', {});";
  const START_OPEN = "__api.sendEvent('asc_onStartAction', 1, 0);";
  const START_FONTS = "__api.sendEvent('asc_onStartAction', 1, 2);";
  const { bridge } = await loadWith(() => 'Ymlu');
  bridge.run(FAKE_API);
  bridge.run("__api.asc_registerCallback('asc_onGetEditorPermissions', function() { _eoLog('editor UI permissions'); });");
  check(results, 'watching an API twice adds one listener of each of the two events',
    listeners(bridge, 'asc_onGetEditorPermissions') === 2 && listeners(bridge, 'asc_onStartAction') === 1,
    `${listeners(bridge, 'asc_onGetEditorPermissions')} ${listeners(bridge, 'asc_onStartAction')}`);
  bridge.run(PERMISSIONS + START_OPEN);
  check(results, 'without an open the permissions and the Open action mark nothing',
    phaseLines(bridge).length === 0, phaseNames(bridge));

  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  bridge.run("_eoOpenPhases.mark('app_ready');");
  bridge.run(START_FONTS);
  check(results, 'an action that is not Open is not the document being loaded',
    phaseNames(bridge) === 'request,result,app_ready', phaseNames(bridge));
  bridge.run(PERMISSIONS + START_FONTS + START_OPEN + START_OPEN + PERMISSIONS);
  bridge.run('window.AscDesktopEditor.LocalStartOpen();');
  bridge.stopTimers();
  check(results, 'doc_info and doc_load are marked once each, between app_ready and doc_request',
    phaseNames(bridge) === 'request,result,app_ready,doc_info,doc_load,doc_request', phaseNames(bridge));
  const info = bridge.logs.findIndex((line) => line.includes('phase=doc_info'));
  check(results, 'doc_info comes right before the editor UI takes the permissions',
    info >= 0 && bridge.logs[info + 1] === 'editor UI permissions', `${info} ${bridge.logs[info + 1]}`);
}

// The font engine can be ready before the editor UI is: sdkjs then asks for
// the document while it takes the document's info.
{
  const { bridge } = await loadWith(() => 'Ymlu');
  bridge.run(FAKE_API);
  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  bridge.run('window.AscDesktopEditor.LocalStartOpen();');
  bridge.run("__api.sendEvent('asc_onGetEditorPermissions', {}); __api.sendEvent('asc_onStartAction', 1, 0);");
  bridge.stopTimers();
  check(results, 'a doc_request that comes first leaves doc_info and doc_load after it',
    phaseNames(bridge) === 'request,result,doc_request,doc_info,doc_load', phaseNames(bridge));
}

// The sdk_min and sdk_all marks editor-patches.js takes from the globals the
// editor frame's sdkjs defines, from SDK_PHASE_GLOBALS to the end of
// _installSdkPhaseMarks.
const patches = (await readFile(path.join(projectRoot, 'src/editor-patches.js'), 'utf8')).replace(/\r\n/g, '\n');

function loadSdkPhaseMarks(bridge) {
  const start = patches.indexOf('    var SDK_PHASE_GLOBALS = {');
  const last = patches.indexOf('    function _installSdkPhaseMarks(win) {', start);
  if (start < 0 || last < 0) throw new Error('editor-patches.js: cannot find the sdk phase marks');
  const end = patches.indexOf('\n    }\n', last);
  bridge.run(patches.slice(start, end + 6) + '\nwindow._installSdkPhaseMarks = _installSdkPhaseMarks;');
  return bridge.sandbox._installSdkPhaseMarks;
}

// The harness has no setInterval; these are tracked so a check can see the
// poll stop, and cleared at the end.
function withIntervals(bridge) {
  const intervals = new Set();
  bridge.sandbox.setInterval = (fn, ms) => {
    const timer = setInterval(fn, ms);
    intervals.add(timer);
    return timer;
  };
  bridge.sandbox.clearInterval = (timer) => { intervals.delete(timer); clearInterval(timer); };
  return intervals;
}

function clearIntervals(intervals) {
  for (const timer of intervals) clearInterval(timer);
  intervals.clear();
}

// The docs API sets only the name of the editor's iframe. The frame element
// stays while its window changes.
function editorFrame(url) {
  const frame = { id: '', name: 'frameEditor', contentWindow: null };
  return { frame, win: frameWindow(frame, url) };
}

function frameWindow(frame, url) {
  const win = { frameElement: frame, document: { URL: url } };
  frame.contentWindow = win;
  return win;
}

const EDITOR_URLS = {
  word: 'tauri://localhost/web-apps/apps/documenteditor/main/index.html',
  cell: 'tauri://localhost/web-apps/apps/spreadsheeteditor/main/index.html',
  slide: 'tauri://localhost/web-apps/apps/presentationeditor/main/index.html',
  pdf: 'tauri://localhost/web-apps/apps/pdfeditor/main/index.html',
};

// What each editor's sdkjs defines, as SDK_PHASE_GLOBALS reads it.
const MIN_PART = {
  word: (win) => { win.Asc = { asc_docs_api: function() {} }; },
  cell: (win) => { win.Asc = { spreadsheet_api: function() {} }; },
  slide: (win) => { win.Asc = { asc_docs_api: function() {} }; },
  pdf: (win) => { win.Asc = { asc_docs_api: function() {} }; },
};

function restPart(win) {
  win.AscCommon = Object.assign(win.AscCommon || {}, { openXml: {} });
}

async function startOpen(docType) {
  const { bridge } = await loadWith(() => 'Ymlu');
  const intervals = withIntervals(bridge);
  const install = loadSdkPhaseMarks(bridge);
  bridge.run(`window.AscDesktopEditor._currentDocType = ${JSON.stringify(docType)}`);
  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  const done = () => { bridge.stopTimers(); clearIntervals(intervals); };
  return { bridge, intervals, install, done };
}

// Each editor, first reached on about:blank and then on its own document.
for (const docType of ['word', 'cell', 'slide', 'pdf']) {
  const { bridge, intervals, install, done } = await startOpen(docType);
  const { frame, win: blank } = editorFrame('about:blank');
  install(blank);
  check(results, `${docType}: the editor frame on about:blank is polled`,
    intervals.size === 1, String(intervals.size));
  const editor = frameWindow(frame, EDITOR_URLS[docType]);
  install(editor);
  check(results, `${docType}: its own document starts no second poll`,
    intervals.size === 1, String(intervals.size));
  await sleep(30);
  check(results, `${docType}: nothing is marked before sdkjs defines its globals`,
    phaseNames(bridge) === 'request,result', phaseNames(bridge));
  MIN_PART[docType](editor);
  await sleep(30);
  check(results, `${docType}: sdk_min is marked once the min part is evaluated`,
    phaseNames(bridge) === 'request,result,sdk_min', phaseNames(bridge));
  restPart(editor);
  await sleep(30);
  check(results, `${docType}: sdk_all follows once the rest is evaluated`,
    phaseNames(bridge) === 'request,result,sdk_min,sdk_all', phaseNames(bridge));
  check(results, `${docType}: the poll stops with both phases marked`,
    intervals.size === 0, String(intervals.size));
  done();
}

// The first contact is already the editor's document, with sdkjs evaluated.
{
  const { bridge, intervals, install, done } = await startOpen('word');
  const { win } = editorFrame(EDITOR_URLS.word);
  MIN_PART.word(win);
  restPart(win);
  install(win);
  install(win);
  await sleep(30);
  check(results, 'a first contact on the editor document marks both phases in order',
    phaseNames(bridge) === 'request,result,sdk_min,sdk_all', phaseNames(bridge));
  await sleep(30);
  check(results, 'each phase is marked once',
    phaseNames(bridge) === 'request,result,sdk_min,sdk_all', phaseNames(bridge));
  check(results, 'and the poll has stopped', intervals.size === 0, String(intervals.size));
  done();
}

// The rest of sdkjs, or another editor's API, does not end the min part.
{
  const { bridge, install, done } = await startOpen('cell');
  const { win } = editorFrame(EDITOR_URLS.cell);
  install(win);
  restPart(win);
  win.Asc = { asc_docs_api: function() {} };
  await sleep(30);
  check(results, 'sdk_all waits for sdk_min, which needs this editor\'s own API',
    phaseNames(bridge) === 'request,result', phaseNames(bridge));
  MIN_PART.cell(win);
  await sleep(30);
  check(results, 'then both are marked, min first',
    phaseNames(bridge) === 'request,result,sdk_min,sdk_all', phaseNames(bridge));
  done();
}

// The rest of sdkjs may arrive after app_ready; document_ready ends the poll.
{
  const { bridge, intervals, install, done } = await startOpen('slide');
  const { win } = editorFrame(EDITOR_URLS.slide);
  install(win);
  MIN_PART.slide(win);
  await sleep(30);
  bridge.run("_eoOpenPhases.mark('app_ready')");
  await sleep(30);
  check(results, 'the poll goes on after app_ready', intervals.size === 1, String(intervals.size));
  restPart(win);
  await sleep(30);
  check(results, 'and still marks an sdk_all that comes after it',
    phaseNames(bridge) === 'request,result,sdk_min,app_ready,sdk_all', phaseNames(bridge));
  done();
}
{
  const { bridge, intervals, install, done } = await startOpen('cell');
  const { win } = editorFrame(EDITOR_URLS.cell);
  install(win);
  MIN_PART.cell(win);
  await sleep(30);
  bridge.run("_eoOpenPhases.mark('document_ready'); _eoOpenPhases.end();");
  await sleep(30);
  check(results, 'the poll stops when document_ready ends the open',
    intervals.size === 0, String(intervals.size));
  restPart(win);
  await sleep(30);
  check(results, 'and marks nothing after it',
    phaseNames(bridge) === 'request,result,sdk_min,document_ready', phaseNames(bridge));
  done();
}

// A new document has no open to mark: the poll ends on its first turn.
{
  const { bridge } = await loadWith(() => 'Ymlu');
  const intervals = withIntervals(bridge);
  const install = loadSdkPhaseMarks(bridge);
  bridge.run("window.AscDesktopEditor._currentDocType = 'word'");
  install(editorFrame(EDITOR_URLS.word).win);
  await sleep(30);
  check(results, 'without an open the poll stops at once', intervals.size === 0, String(intervals.size));
  bridge.stopTimers();
  clearIntervals(intervals);
}

// The marks can never stop the bridge being injected: they do not throw, and
// injectBridgeDeep calls them after the bridge is in the frame.
{
  const { install, done } = await startOpen('word');
  const unreadable = {
    get frameElement() { throw new Error('blocked'); },
    document: { URL: 'about:blank' },
  };
  let thrown = null;
  try { install(unreadable); } catch (error) { thrown = error; }
  check(results, 'a frame element that throws does not reach the caller', thrown === null, String(thrown));
  done();
}
{
  const { bridge, install, done } = await startOpen('word');
  const editor = bridge.sandbox.AscDesktopEditor;
  delete bridge.sandbox.AscDesktopEditor;
  let thrown = null;
  try { install(editorFrame(EDITOR_URLS.word).win); } catch (error) { thrown = error; }
  bridge.sandbox.AscDesktopEditor = editor;
  check(results, 'a missing window.AscDesktopEditor does not reach the caller', thrown === null, String(thrown));
  done();
}
{
  const start = patches.indexOf('    function injectBridgeDeep(win) {');
  const end = patches.indexOf('\n    }\n', start);
  const body = patches.slice(start, end);
  const bridgeBlock = body.indexOf('        if (win && !hasADE) {');
  const blockEnd = body.indexOf('\n        }\n', body.indexOf('END TABS FIX', bridgeBlock));
  const call = body.indexOf('        _installSdkPhaseMarks(win);');
  check(results, 'injectBridgeDeep sets up the marks after it injects the bridge',
    bridgeBlock > 0 && blockEnd > bridgeBlock && call > blockEnd, `${bridgeBlock} ${blockEnd} ${call}`);
}

// Other frames and a frame that cannot be read.
{
  const { intervals, install, done } = await startOpen('word');
  const helper = { frameElement: { id: 'asc_pasteFrame', name: '' }, document: { URL: 'about:blank' } };
  const unnamed = { frameElement: { id: '', name: '' }, document: { URL: 'about:blank' } };
  const topLevel = { frameElement: null, document: { URL: 'about:blank' } };
  const unreadable = {
    get frameElement() { throw new Error('blocked'); },
    document: { URL: 'about:blank' },
  };
  for (const win of [helper, unnamed, topLevel, unreadable]) install(win);
  check(results, 'a frame that is not the editor frame starts no poll',
    intervals.size === 0, String(intervals.size));
  done();
}

// No document type to read the globals for.
{
  const { intervals, install, done } = await startOpen(null);
  install(editorFrame('about:blank').win);
  check(results, 'without a document type no poll starts', intervals.size === 0, String(intervals.size));
  done();
}

finish(results);
