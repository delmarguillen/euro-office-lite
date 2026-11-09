// Checks that the editor waits for the font cache: with fonts_status pending
// a granted file is converted at once but the editor opens only when
// "fonts-ready" arrives; ready or failed open it straight away. The launch
// argument goes to the start screen's Open (editor-patches.js), whose
// openEditor waits for the fonts itself.
//
// Run: node scripts/test-bridge-fonts-ready.mjs [path/to/bridge.js]
import { check, finish, loadBridge, pendingQueue } from './bridge-test-harness.mjs';

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
      take_pending_open_file: pendingQueue(launchFile),
      open_file: (args) => { calls.push('open_file ' + args.token); return binBase64(); },
    },
  });
  // The bridge asks for the launch argument when it loads; the stub editor
  // is in place before that open reaches it.
  bridge.run(`
    var __editors = [];
    var __opened = [];
    window._openEditor = function(docType) { __editors.push(docType); };
    window._eoOpenPath = function(file) { __opened.push(file.token); };
  `);
  return {
    bridge,
    calls,
    editors: () => Array.from(bridge.sandbox.__editors),
    opened: () => Array.from(bridge.sandbox.__opened),
  };
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

// The launch argument, granted as a token, with the font cache still building.
start = await loadWith('pending', { token: 'tok-launch', name: 'launch.xlsx' });
await sleep(50);
check(results, 'launch argument: the start screen\'s Open gets its token once', start.opened().join() === 'tok-launch', start.opened().join());
check(results, '... and the font status is asked for', start.calls.includes('fonts_status'), start.calls.join());
start.bridge.stopTimers();

finish(results);
