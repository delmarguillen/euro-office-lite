// Checks that the changes an editor has not sent yet reach the recovery
// folder when an unhandled exception puts it in view mode. The exception
// handler and the autosave are the ones of sdkjs (common/apiBase.js, run here
// as they are): the handler sends EditingError and sets view mode, and the
// autosave skips an editor in view mode. The editor around them is a fake
// whose asc_Save hands the pending changes to LocalFileSaveChanges, as
// word/Local/api.js and common/Local/common.js do.
//
// Run: node scripts/test-bridge-editor-error-pending-changes.mjs [path/to/bridge.js]
import { check, finish, loadBridge, loadSdkjsSnippet } from './bridge-test-harness.mjs';

const results = [];
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const EDITOR = `
  function baseEditorsApi() {}
  var pending = [];
  var saveCalls = 0;
  var c_oAscError = {
    ID: { EditingError: -25, ConvertationOpenError: -1, ConvertationSaveError: -2 },
    Level: { Critical: -1, NoCritical: 0 },
  };
  var Asc = { c_oAscError: c_oAscError };
  var AscCommon = {
    History: { Have_Changes: function() { return pending.length > 0; } },
    sendClientLog: function() {},
    getMemoryInfo: function() { return ''; },
  };
  var t = new baseEditorsApi();
  t.callbacks = {};
  t.asc_registerCallback = function(name, callback) {
    if (!this.callbacks[name]) this.callbacks[name] = [];
    this.callbacks[name].push(callback);
  };
  t.sendEvent = function(name, id, level) {
    (this.callbacks[name] || []).forEach(function(callback) { callback(id, level); });
  };
  t.isLoadFullApi = true;
  t.isDocumentLoadComplete = true;
  t.isCoAuthoringEnable = false;
  t.isViewMode = false;
  t.canSave = true;
  t.canUnlockDocument = false;
  t.autoSaveGap = 2000;
  t.isLiveViewer = function() { return false; };
  t.getEditorErrorInfo = function() { return ''; };
  t.asc_setViewMode = function(isViewMode) { this.isViewMode = !!isViewMode; };
  t._autoSaveInner = function() {
    if (AscCommon.History.Have_Changes(true)) this.asc_Save(true);
  };
  t.asc_Save = function() {
    saveCalls += 1;
    if (!this.canSave) return;
    var changes = pending.splice(0);
    AscDesktopEditor.LocalFileSaveChanges(changes.join('","'), null, changes.length);
  };
`;

async function setup(docType = 'word') {
  const recorded = [];
  const bridge = await loadBridge({
    invoke: { save_changes: (args) => { recorded.push(args); return 'ok'; } },
  });
  bridge.run(EDITOR);
  await loadSdkjsSnippet(bridge, 'src/sdkjs/common/apiBase.js',
    'const sendUnhandledError  = function(errorMsg, url, lineNumber, column, stack) {',
    'c_oAscError.Level.Critical);\n\t\t\t\t}\n\t\t\t}\n\t\t}');
  await loadSdkjsSnippet(bridge, 'src/sdkjs/common/apiBase.js',
    'baseEditorsApi.prototype._autoSave = function () {', '\n\t};');
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.run('AscDesktopEditor.CreateEditorApi(t);');
  return { bridge, recorded };
}

const throwInTheEditor = (bridge) => bridge.run("sendUnhandledError('boom', '', 1, 1, '')");
// The interval of sdkjs calls _autoSave every 40 ms; a few calls stand for it.
const runAutoSave = (bridge) => bridge.run('for (var i = 0; i < 5; i++) t._autoSave();');
const changesOf = (recorded) => recorded.flatMap((args) => args.changes).join();

// Without an error, the autosave sends the changes.
{
  const { bridge, recorded } = await setup();
  bridge.run("pending = ['c1', 'c2'];");
  runAutoSave(bridge);
  check(results, 'the autosave of an editor in edit mode records its changes',
    changesOf(recorded) === 'c1,c2', JSON.stringify(recorded));
  bridge.stopTimers();
}

// An exception with changes pending.
{
  const { bridge, recorded } = await setup();
  bridge.run("pending = ['c1', 'c2'];");
  throwInTheEditor(bridge);
  check(results, 'sdkjs puts the editor in view mode after the exception',
    bridge.run('t.isViewMode') === true);
  const atTheError = recorded.length;
  await sleep(30);
  check(results, 'the pending changes are recorded after the exception',
    changesOf(recorded) === 'c1,c2', JSON.stringify(recorded));
  check(results, '... once the task of the exception has ended, in one batch with no index',
    atTheError === 0 && recorded.length === 1 && recorded[0].deleteIndex === null && recorded[0].count === 2,
    JSON.stringify(recorded));
  check(results, '... and the log says so',
    bridge.logs.some((line) => line.includes('[RECOVER] pending changes sent after the editor error')),
    bridge.logs.join(' | '));
  bridge.stopTimers();
}

// The cause: the autosave of sdkjs does nothing for an editor in view mode.
{
  const { bridge, recorded } = await setup();
  throwInTheEditor(bridge);
  await sleep(30);
  bridge.run("pending = ['late'];");
  runAutoSave(bridge);
  check(results, 'the autosave of sdkjs skips an editor in view mode',
    recorded.length === 0 && bridge.run('saveCalls') === 0, JSON.stringify(recorded));
  bridge.stopTimers();
}

// A save is running when the exception arrives: the changes are sent later.
{
  const { bridge, recorded } = await setup();
  bridge.run("_EO_PENDING_CHANGES_RETRY_MS = 20; pending = ['c1']; t.canSave = false;");
  throwInTheEditor(bridge);
  await sleep(30);
  check(results, 'nothing is recorded while sdkjs refuses the save', recorded.length === 0,
    JSON.stringify(recorded));
  bridge.run('t.canSave = true;');
  await sleep(60);
  check(results, 'the changes are recorded once sdkjs takes the save',
    changesOf(recorded) === 'c1', JSON.stringify(recorded));
  bridge.stopTimers();
}

// sdkjs never takes the save: the tries end.
{
  const { bridge, recorded } = await setup();
  bridge.run("_EO_PENDING_CHANGES_TRIES = 3; _EO_PENDING_CHANGES_RETRY_MS = 10; pending = ['c1']; t.canSave = false;");
  throwInTheEditor(bridge);
  await sleep(120);
  check(results, 'the save is tried the given number of times and no more',
    bridge.run('saveCalls') === 3 && recorded.length === 0, String(bridge.run('saveCalls')));
  check(results, '... and the log says the changes were not sent',
    bridge.logs.some((line) => line.includes('[RECOVER] pending changes not sent after the editor error')),
    bridge.logs.join(' | '));
  bridge.stopTimers();
}

// Nothing pending, another error, a PDF: no save is asked for.
{
  const { bridge, recorded } = await setup();
  throwInTheEditor(bridge);
  await sleep(30);
  check(results, 'an exception with nothing pending asks for no save',
    bridge.run('saveCalls') === 0 && recorded.length === 0);
  bridge.stopTimers();
}
{
  const { bridge, recorded } = await setup();
  bridge.run("pending = ['c1']; t.sendEvent('asc_onError', c_oAscError.ID.ConvertationSaveError, c_oAscError.Level.NoCritical);");
  await sleep(30);
  check(results, 'another error of the editor asks for no save',
    bridge.run('saveCalls') === 0 && recorded.length === 0);
  bridge.stopTimers();
}
{
  const { bridge, recorded } = await setup('pdf');
  bridge.run("pending = ['c1'];");
  throwInTheEditor(bridge);
  await sleep(30);
  check(results, 'a PDF, which has no recovery session, asks for no save',
    bridge.run('saveCalls') === 0 && recorded.length === 0);
  bridge.stopTimers();
}

// The same API handed over twice is watched once.
{
  const { bridge, recorded } = await setup();
  bridge.run("AscDesktopEditor.CreateEditorApi(t); pending = ['c1'];");
  throwInTheEditor(bridge);
  await sleep(30);
  check(results, 'an API handed over twice sends its changes once',
    bridge.run('saveCalls') === 1 && changesOf(recorded) === 'c1', JSON.stringify(recorded));
  bridge.stopTimers();
}

finish(results);
