// Checks that the editor waits for the font cache: with fonts_status pending
// a granted file is converted at once but the editor opens only when
// "fonts-ready" arrives; ready or failed open it straight away. The launch
// argument, also a token, waits the same way.
//
// Run: node scripts/test-bridge-fonts-ready.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const BIN_BYTES = 'DOCY;v5;0;';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const REPORT = { token: 'tok-report', name: 'report.docx' };

function binBase64() {
  return Buffer.from(BIN_BYTES, 'binary').toString('base64');
}

async function loadWith(status, launchFile) {
  const calls = [];
  const bridge = await loadBridge({
    invoke: {
      fonts_status: () => { calls.push('fonts_status'); return status; },
      take_pending_open_file: () => launchFile,
      open_file: (args) => { calls.push('open_file ' + args.token); return binBase64(); },
    },
  });
  // The bridge asks for the launch argument when it loads; the stub editor
  // is in place before that open reaches it.
  bridge.run(`
    var __editors = [];
    window._openEditor = function(docType) { __editors.push(docType); };
  `);
  return { bridge, calls, editors: () => Array.from(bridge.sandbox.__editors) };
}

async function startWith(status) {
  const start = await loadWith(status, null);
  start.bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(50);
  return start;
}

let start = await startWith('pending');
check(results, 'pending: the file is converted at once', start.calls.includes('open_file tok-report'), start.calls.join());
check(results, '... but the editor does not open yet', start.editors().length === 0, start.editors().join());
start.bridge.emit('fonts-ready', 'ready');
await sleep(50);
check(results, '... and opens when fonts-ready arrives', start.editors().join() === 'word', start.editors().join());
start.bridge.stopTimers();

start = await startWith('pending');
start.bridge.emit('fonts-ready', 'failed');
await sleep(50);
check(results, 'pending then failed: the editor opens anyway', start.editors().join() === 'word', start.editors().join());
start.bridge.stopTimers();

start = await startWith('ready');
check(results, 'ready: the editor opens straight away', start.editors().join() === 'word', start.editors().join());
start.bridge.stopTimers();

start = await startWith('failed');
check(results, 'failed: the editor opens straight away', start.editors().join() === 'word', start.editors().join());
start.bridge.stopTimers();

start = await startWith('ready');
start.bridge.run(`_eoWaitForFonts(); _eoWaitForFonts();`);
await sleep(20);
const asked = start.calls.filter((call) => call === 'fonts_status').length;
check(results, 'the status is asked once for every caller', asked === 1, String(asked));
start.bridge.stopTimers();

// The launch argument, granted as a token.
start = await loadWith('pending', { token: 'tok-launch', name: 'launch.xlsx' });
await sleep(50);
check(results, 'launch argument, pending: converted at once by token', start.calls.includes('open_file tok-launch'), start.calls.join());
check(results, '... but the editor does not open yet', start.editors().length === 0, start.editors().join());
start.bridge.emit('fonts-ready', 'ready');
await sleep(50);
check(results, '... and opens when fonts-ready arrives', start.editors().join() === 'cell', start.editors().join());
start.bridge.stopTimers();

start = await loadWith('ready', { token: 'tok-launch', name: 'launch.xlsx' });
await sleep(50);
check(results, 'launch argument, ready: the editor opens straight away', start.editors().join() === 'cell', start.editors().join());
start.bridge.stopTimers();

finish(results);
