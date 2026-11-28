// Checks the marks of the end of a document open (bridge.js): the microtasks
// that run between the end of the task that sent asc_onDocumentContentReady
// and the spellcheck language list, and the first animation frame after them.
//
// Run: node scripts/test-bridge-open-tail.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const REPORT = { token: 'tok-report', name: 'report.docx' };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const phaseNames = (bridge) => bridge.logs
  .filter((line) => line.startsWith('[OPEN] phase='))
  .map((line) => /phase=(\w+)/.exec(line)[1])
  .join();

const at = (bridge, text) => bridge.logs.findIndex((line) => line.includes(text));

// The bridge with both dictionary lists answered, as they are by the time a
// document opens.
async function loadWithDictionaries() {
  const bridge = await loadBridge({
    invoke: {
      fonts_status: () => 'ready',
      open_file: () => 'Ymlu',
      list_user_dictionaries: () => ({ valid: [], skipped: [] }),
    },
    fetch: async () => ({ ok: true, json: async () => ['en_US', 'es_ES'] }),
  });
  bridge.run('window._openEditor = function() {};');
  await sleep(0);
  return bridge;
}

// The editor frame and its API. An event goes to the listeners in the order
// they were added, as in sdkjs. The bridge attaches first, as it does from
// CreateEditorApi; the editor UI adds its handlers after it, and its handler
// of asc_onDocumentContentReady queues a microtask of its own.
const EDITOR = `
  window.AscDesktopEditor._editorWindow = {
    Asc: { c_oAscAsyncAction: { Open: 0, LoadDocumentFonts: 2, LoadDocumentImages: 3 } },
    AscCommon: {
      spellcheckGetLanguages: function() { return { 1033: { name: 'en_US' }, 3082: { name: 'es_ES' } }; }
    }
  };
  window.__api = {
    callbacks: {},
    _coSpellCheckInit: function() {},
    asc_registerCallback: function(name, fn) {
      (this.callbacks[name] = this.callbacks[name] || []).push(fn);
    },
    sendEvent: function(name) {
      var list = this.callbacks[name] || [];
      var args = Array.prototype.slice.call(arguments, 1);
      for (var i = 0; i < list.length; i++) list[i].apply(this, args);
    }
  };
  _eoWatchOpenPhases(__api);
  SpellCheckBridge.attach(__api);
  __api.asc_registerCallback('asc_onDocumentContentReady', function() {
    Promise.resolve().then(function() { _eoLog('editor UI microtask'); });
  });
  __api.asc_registerCallback('asc_onSpellCheckInit', function(lcids) {
    _eoLog('editor UI took the list: ' + lcids.join());
  });
`;
const OPEN = `_eoOpenGranted(${JSON.stringify(REPORT)})`;
const CONTENT_READY = "_eoOpenPhases.mark('doc_decoded'); __api.sendEvent('asc_onEndAction', 1, 0); __api.sendEvent('asc_onDocumentContentReady');";

{
  const bridge = await loadWithDictionaries();
  bridge.run(EDITOR);
  bridge.run(OPEN);
  await sleep(10);
  bridge.run(CONTENT_READY);
  await sleep(0);
  bridge.stopTimers();

  check(results, 'the open marks microtasks between layout and spell_init',
    phaseNames(bridge) === 'request,result,doc_decoded,content_ready,ui_ready,layout,microtasks,spell_init',
    phaseNames(bridge));
  const queued = at(bridge, 'editor UI microtask');
  const microtasks = at(bridge, 'phase=microtasks');
  const list = at(bridge, 'editor UI took the list: 1033,3082');
  check(results, 'microtasks comes after the microtask the editor UI queued and before the UI takes the list',
    queued >= 0 && queued < microtasks && microtasks < list, `${queued} ${microtasks} ${list}`);
  check(results, 'spell_init comes after the editor UI took the list',
    list >= 0 && list < at(bridge, 'phase=spell_init'), `${list} ${at(bridge, 'phase=spell_init')}`);
}

// The animation frames of the page and of the editor frame, run by hand. The
// editor frame already holds the request of sdkjs's paint loop.
const FRAMES = `
  window.__pageFrames = [];
  window.requestAnimationFrame = function(fn) { __pageFrames.push(fn); };
  window.__editorFrames = [function() { _eoLog('sdkjs paint loop'); }];
  window.AscDesktopEditor._editorWindow.requestAnimationFrame = function(fn) { __editorFrames.push(fn); };
  window.__runFrame = function() {
    var callbacks = __pageFrames.concat(__editorFrames);
    __pageFrames = [];
    __editorFrames = [];
    for (var i = 0; i < callbacks.length; i++) callbacks[i]();
  };
`;
const requested = (bridge) => `${bridge.sandbox.__pageFrames.length},${bridge.sandbox.__editorFrames.length}`;

{
  const bridge = await loadWithDictionaries();
  bridge.run(EDITOR + FRAMES);
  bridge.run(OPEN);
  await sleep(10);
  bridge.run(CONTENT_READY);
  check(results, 'no frame is asked for while the task that sent the event runs',
    requested(bridge) === '0,1', requested(bridge));
  await sleep(0);
  check(results, 'once the task has ended one frame is asked of the page and one of the editor frame',
    requested(bridge) === '1,2', requested(bridge));
  bridge.run('__runFrame();');
  bridge.run("_eoOpenPhases.mark('document_ready'); _eoOpenPhases.end();");
  bridge.stopTimers();

  check(results, 'the open marks frame and doc_drawn between spell_init and document_ready',
    phaseNames(bridge) === 'request,result,doc_decoded,content_ready,ui_ready,layout,microtasks,spell_init,frame,doc_drawn,document_ready',
    phaseNames(bridge));
  const frame = at(bridge, 'phase=frame');
  const paint = at(bridge, 'sdkjs paint loop');
  const drawn = at(bridge, 'phase=doc_drawn');
  check(results, 'frame comes before the paint loop of sdkjs and doc_drawn after it',
    frame >= 0 && frame < paint && paint < drawn, `${frame} ${paint} ${drawn}`);
  check(results, 'the marks ask for no further frame', requested(bridge) === '0,0', requested(bridge));
}

// The frame comes after onDocumentReady: the open has ended.
{
  const bridge = await loadWithDictionaries();
  bridge.run(EDITOR + FRAMES);
  bridge.run(OPEN);
  await sleep(10);
  bridge.run(CONTENT_READY);
  await sleep(0);
  bridge.run("_eoOpenPhases.mark('document_ready'); _eoOpenPhases.end();");
  bridge.run('__runFrame();');
  bridge.stopTimers();
  check(results, 'a frame after document_ready marks nothing',
    phaseNames(bridge) === 'request,result,doc_decoded,content_ready,ui_ready,layout,microtasks,spell_init,document_ready',
    phaseNames(bridge));
}

// The editor frame has no requestAnimationFrame yet.
{
  const bridge = await loadWithDictionaries();
  bridge.run(EDITOR + FRAMES);
  bridge.run('delete window.AscDesktopEditor._editorWindow.requestAnimationFrame;');
  bridge.run(OPEN);
  await sleep(10);
  bridge.run(CONTENT_READY);
  await sleep(0);
  bridge.run('__runFrame();');
  bridge.stopTimers();
  check(results, 'without a frame of the editor only the page\'s frame is marked',
    phaseNames(bridge) === 'request,result,doc_decoded,content_ready,ui_ready,layout,microtasks,spell_init,frame',
    phaseNames(bridge));
}

// No open in progress: a new document sends the same events.
{
  const bridge = await loadWithDictionaries();
  bridge.run(EDITOR);
  bridge.run("__api.sendEvent('asc_onEndAction', 1, 0); __api.sendEvent('asc_onDocumentContentReady');");
  await sleep(0);
  bridge.stopTimers();
  check(results, 'without an open the list is sent and nothing is marked',
    phaseNames(bridge) === '' && at(bridge, 'editor UI took the list: 1033,3082') >= 0, phaseNames(bridge));
}

finish(results);
