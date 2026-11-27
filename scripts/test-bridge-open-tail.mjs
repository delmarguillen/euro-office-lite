// Checks the marks of the end of a document open (bridge.js): the microtasks
// that run between the end of the task that sent asc_onDocumentContentReady
// and the spellcheck language list.
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
