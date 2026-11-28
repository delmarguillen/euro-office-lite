// Exercise the real preview hook and bridge print flow without a printer.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { loadBridge, projectRoot } from './bridge-test-harness.mjs';

const source = await readFile(path.join(projectRoot, 'src/editor-patches.js'), 'utf8');
const start = source.indexOf('// Supply printer list to print preview panel');
const end = source.indexOf('var fontPatched = false;', start);
assert.ok(start >= 0 && end > start);
const hook = source.slice(start, end);
const settle = () => new Promise(resolve => setImmediate(resolve));

async function preview(platform, response, fail = false, editorApp = 'DE') {
  const calls = [], messages = [], logs = [], intervals = [];
  let opened = 0;
  const lm = { mode: {}, leftMenu: { showMenu() { opened++; return 'opened'; } } };
  const sandbox = {
    navigator: { platform },
    setInterval(fn) { intervals.push(fn); return fn; },
    clearInterval() {}, setTimeout() {},
    _eoLog(message) { logs.push(message); },
    __TAURI__: { core: { invoke(command) {
      calls.push(command);
      return fail ? Promise.reject(new Error('CUPS unavailable')) : Promise.resolve(response);
    } } },
    win: {
      [editorApp]: { getController() { return lm; } },
      _eoLog(message) { logs.push(message); },
      on_native_message(type, json) { messages.push([type, JSON.parse(json)]); },
    },
  };
  sandbox.window = sandbox;
  vm.runInNewContext(hook, sandbox);
  intervals[0]();
  assert.equal(lm.leftMenu.showMenu('file:printpreview'), 'opened');
  await settle();
  assert.equal(opened, 1);
  return { calls, messages, logs };
}

const linux = await preview('Linux x86_64', JSON.stringify([
  { name: 'Office', is_default: false, color_supported: false, duplex_supported: false },
  { name: 'Office/Duplex', is_default: true, color_supported: true, duplex_supported: true },
]));
assert.deepEqual(linux.calls, ['plugin:printer|get_printers']);
assert.deepEqual(linux.messages, [['printer:config', {
  printers: [
    { name: 'Office', color_supported: false, duplex_supported: false },
    { name: 'Office/Duplex', color_supported: true, duplex_supported: true },
  ], current_printer: 'Office/Duplex',
}]]);
const arm = await preview('Linux aarch64', '[]');
assert.deepEqual(arm.messages[0][1], { printers: [], current_printer: null });
for (const app of ['SSE', 'PE', 'PDFE']) {
  const editor = await preview('Linux x86_64', '[{"name":"Office"}]', false, app);
  assert.equal(editor.messages[0][1].current_printer, 'Office');
}
const windows = await preview('Win32', '{"Name":"Windows printer"}');
assert.equal(windows.messages[0][1].current_printer, 'Windows printer');
const mac = await preview('MacIntel', '[]');
assert.deepEqual(mac.calls, []);
const unavailable = await preview('Linux x86_64', null, true);
assert.ok(unavailable.logs.some(log => log.includes('CUPS unavailable')));
console.log('PASS Linux/ARM discovery, capabilities, defaults, no queues, errors, Windows and macOS preview');

for (const type of ['word', 'cell', 'slide', 'pdf']) {
  const calls = [], completions = [], errors = [];
  let rejectSubmission = false;
  const bridge = await loadBridge({ invoke: {
    print_document(args) { calls.push(['generate', args]); },
    print_to_printer(args) {
      calls.push(['submit', args]);
      if (rejectSubmission) throw new Error('CUPS rejected the job');
    },
    open_pdf_viewer(args) { calls.push(['viewer', args]); },
  } });
  try {
    Object.assign(bridge.sandbox, {
      _getEditor: () => ({ editor: {}, ew: { DesktopOfflineAppDocumentEndSave(code) { completions.push(code); } } }),
      _eoBeginSave: () => ({}), _eoEndSave() {}, _eoRestoreModified() {},
      _eoWriteEditorBin: async () => calls.push(['write']),
      _eoWithSheetLocale: (_ref, args) => args,
      _eoPdfChanges: () => 'pdf-changes',
    });
    bridge.sandbox.__TAURI__.dialog = { message: async text => errors.push(text) };
    const desktop = bridge.sandbox.AscDesktopEditor;
    desktop._currentDocType = type;
    await desktop.Print(JSON.stringify({ nativeOptions: { printer: 'Office/Duplex' } }));
    assert.deepEqual(calls.map(call => call[0]), type === 'pdf' ? ['generate', 'submit'] : ['write', 'generate', 'submit']);
    assert.equal(calls.at(-1)[1].printer, 'Office/Duplex');
    assert.deepEqual(Object.keys(calls.at(-1)[1]), ['printer']);
    if (type === 'pdf') assert.equal(calls[0][1].pdfChanges, 'pdf-changes');
    assert.equal(completions.at(-1), 0);

    calls.length = 0;
    await desktop.Print();
    assert.equal(calls.at(-1)[0], 'viewer');

    rejectSubmission = true;
    await desktop.Print(JSON.stringify({ nativeOptions: { printer: 'Office' } }));
    assert.equal(completions.at(-1), 1);
    assert.equal(errors.length, 1);
    assert.equal(desktop._isPrinting, false);
    console.log(`PASS ${type}: named printer, generated PDF only, viewer fallback and submission failure`);
  } finally {
    bridge.stopTimers();
  }
}
