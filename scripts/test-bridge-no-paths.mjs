// Checks that the bridge never hands the backend a path: every command that
// opens, saves or converts a file gets the token the backend gave out, the
// dialogs open in the backend, and an external reference the document names
// only sends its file name to the locate dialog. grant_pasted_image is the one
// command that still takes a path, checked by the backend against the
// clipboard; it is left out here.
//
// Run: node scripts/test-bridge-no-paths.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const HOME = '/home/u/Documents/';
const DOCX = 65;
// Every command call of every run below.
const everyCall = [];

function looksLikePath(value) {
  if (typeof value !== 'string') return false;
  return value.includes('/') || value.includes('\\');
}

async function setup(extra = {}) {
  const calls = [];
  const jsDialogs = [];
  const recorded = (command, answer) => (args) => {
    calls.push({ command, args: args || {} });
    everyCall.push({ command, args: args || {} });
    return typeof answer === 'function' ? answer(args) : answer;
  };
  const bridge = await loadBridge({
    invoke: {
      take_pending_open_file: recorded('take_pending_open_file',
        { token: 'tok-launch', name: 'launch.docx' }),
      open_file: recorded('open_file', 'Ymlu'),
      write_editor_bin: recorded('write_editor_bin', 'ok'),
      current_document: recorded('current_document', { token: 'tok-current', name: 'report.docx' }),
      pick_save_target: recorded('pick_save_target', { token: 'tok-save', name: 'copy.docx' }),
      save_file_as: recorded('save_file_as', 'ok'),
      recovery_mark_saved: recorded('recovery_mark_saved', null),
      set_window_title: recorded('set_window_title', null),
      pick_files: recorded('pick_files', (args) => (args.kind === 'cell'
        ? [{ token: 'tok-ref', name: 'book.xlsx', path: HOME + 'book.xlsx' }]
        : [{ token: 'tok-' + args.kind, name: args.kind === 'images' ? 'a.png' : 'b.docx' }])),
      locate_linked_file: recorded('locate_linked_file', { token: 'tok-located', name: 'linked.xlsx' }),
      convert_for_insert: recorded('convert_for_insert', { data: 'Ymlu', images: {} }),
      recent_files_state: recorded('recent_files_state', {
        enabled: true,
        files: [{ path: HOME + 'recent.docx', token: 'tok-recent', name: 'recent.docx', fileType: DOCX }],
      }),
      recovery_end: recorded('recovery_end', null),
      set_document_modified: recorded('set_document_modified', null),
      ...extra,
    },
  });
  bridge.sandbox.__TAURI__.dialog = {
    open: (options) => { jsDialogs.push(['open', options]); return Promise.resolve(HOME + 'x.docx'); },
    save: (options) => { jsDialogs.push(['save', options]); return Promise.resolve(HOME + 'x.docx'); },
    message: () => Promise.resolve(),
    confirm: () => Promise.resolve(true),
  };
  const urls = [];
  bridge.sandbox.__xhr = (url) => { urls.push(url); return { status: 200, body: 'image1.png' }; };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: () => {},
  };
  return { bridge, calls, jsDialogs, urls };
}

function argsOf(calls, command) {
  return calls.filter((call) => call.command === command).map((call) => call.args);
}

function pathsSent(calls) {
  return calls.filter((call) => Object.values(call.args).some(looksLikePath));
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));

// Opening the launch argument.
let run = await setup();
await settle();
check(results, 'the launch argument opens by token',
  argsOf(run.calls, 'open_file').map((args) => args.token).join() === 'tok-launch',
  JSON.stringify(argsOf(run.calls, 'open_file')));
run.bridge.stopTimers();

// Save As.
run = await setup();
await run.bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${DOCX}, '')`);
await settle();
check(results, 'Save As asks the backend for the target and saves by token',
  argsOf(run.calls, 'save_file_as').map((args) => args.token).join() === 'tok-save',
  JSON.stringify(argsOf(run.calls, 'save_file_as')));
check(results, '... and opens no dialog of its own', run.jsDialogs.length === 0,
  JSON.stringify(run.jsDialogs));
run.bridge.stopTimers();

// Insert a document, then convert it.
run = await setup();
let picked = await new Promise((resolve) => {
  run.bridge.sandbox.__picked = resolve;
  run.bridge.run(`AscDesktopEditor.OpenFilenameDialog('word', false, function(f) { window.__picked(f); })`);
});
await new Promise((resolve) => {
  run.bridge.sandbox.__converted = resolve;
  run.bridge.run(`AscDesktopEditor.convertFile(${JSON.stringify(picked)}, 0x2002, function(f) { window.__converted(f); })`);
});
check(results, 'a document picked to insert is converted by token',
  argsOf(run.calls, 'convert_for_insert').map((args) => args.token).join() === 'tok-word',
  JSON.stringify(argsOf(run.calls, 'convert_for_insert')));
check(results, '... and the editor holds no path for it', !looksLikePath(picked.replace(/^eo-grant:[^/]+\//, '')),
  String(picked));
run.bridge.stopTimers();

// An external reference picked in the dialog: the editor gets the path, the
// backend still only the token.
run = await setup();
picked = await new Promise((resolve) => {
  run.bridge.sandbox.__picked = resolve;
  run.bridge.run(`AscDesktopEditor.OpenFilenameDialog('cell', false, function(f) { window.__picked(f); })`);
});
await new Promise((resolve) => {
  run.bridge.sandbox.__converted = resolve;
  run.bridge.run(`AscDesktopEditor.convertFile(${JSON.stringify(picked)}, 0x2002, function(f) { window.__converted(f); })`);
});
check(results, 'a reference keeps its path in the editor', picked === HOME + 'book.xlsx', String(picked));
check(results, '... and is converted with the token it was granted under',
  argsOf(run.calls, 'convert_for_insert').map((args) => args.token).join() === 'tok-ref',
  JSON.stringify(argsOf(run.calls, 'convert_for_insert')));
run.bridge.stopTimers();

// A reference saved in the document, never picked in this session.
run = await setup();
await new Promise((resolve) => {
  run.bridge.sandbox.__converted = resolve;
  run.bridge.run(`AscDesktopEditor.convertFile(${JSON.stringify(HOME + 'linked.xlsx')}, 0x2002, function(f) { window.__converted(f); })`);
});
check(results, 'an unknown path goes to the locate dialog by name only',
  JSON.stringify(argsOf(run.calls, 'locate_linked_file').map((args) => args.name)) === '["linked.xlsx"]',
  JSON.stringify(argsOf(run.calls, 'locate_linked_file')));
check(results, '... and converts what the user located',
  argsOf(run.calls, 'convert_for_insert').map((args) => args.token).join() === 'tok-located',
  JSON.stringify(argsOf(run.calls, 'convert_for_insert')));
run.bridge.stopTimers();

// A cancelled locate converts nothing.
run = await setup({ locate_linked_file: () => null });
let converted = await new Promise((resolve) => {
  run.bridge.sandbox.__converted = resolve;
  run.bridge.run(`AscDesktopEditor.convertFile(${JSON.stringify(HOME + 'linked.xlsx')}, 0x2002, function(f) { window.__converted(f); })`);
});
check(results, 'a cancelled locate answers the editor with no file',
  converted === null && argsOf(run.calls, 'convert_for_insert').length === 0, String(converted));
run.bridge.stopTimers();

// Insert an image.
run = await setup();
picked = await new Promise((resolve) => {
  run.bridge.sandbox.__picked = resolve;
  run.bridge.run(`AscDesktopEditor.OpenFilenameDialog('images', false, function(f) { window.__picked(f); })`);
});
run.bridge.run(`AscDesktopEditor.LocalFileGetImageUrl(${JSON.stringify(picked)})`);
const copyUrl = decodeURIComponent(run.urls.find((url) => url.includes('copy-to-media')) || '');
check(results, 'an inserted image is copied by token',
  copyUrl.endsWith('copy-to-media/eo-grant:tok-images/a.png'), copyUrl);
run.bridge.stopTimers();

// Reopening from the editor's recent list.
run = await setup();
run.bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
run.bridge.sandbox.location = { reload: () => {} };
run.bridge.run(`AscDesktopEditor.execCommand('open:recent', ${JSON.stringify(JSON.stringify({ id: 0, path: HOME + 'recent.docx' }))})`);
await settle();
const pending = run.bridge.sandbox.localStorage.getItem('eo-pending-open');
check(results, 'a recent entry is reopened by its token', pending === JSON.stringify({ token: 'tok-recent', name: 'recent.docx' }),
  String(pending));
run.bridge.stopTimers();

// Across every run above: no command got a path.
check(results, 'no command receives a path', pathsSent(everyCall).length === 0,
  JSON.stringify(pathsSent(everyCall)));

finish(results);
