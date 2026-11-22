// Checks the phase marks of a document open (bridge.js, _eoOpenPhases): the
// line of each phase, the id the backend gets, that dt is the time since the
// previous mark of the same open, and that a failed or finished open marks
// nothing more. Also the marks editor-patches.js takes from the load events
// of the editor frame's scripts.
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

// The scripts of the editor that end a phase.
{
  const { bridge } = await loadWith(() => 'Ymlu');
  const phaseFor = bridge.sandbox._eoSdkPhaseForScript;
  bridge.stopTimers();
  check(results, 'sdk-all-min.js ends sdk_min',
    phaseFor('tauri://localhost/sdkjs/word/sdk-all-min.js') === 'sdk_min');
  check(results, 'sdk-all.js ends sdk_all, with a query or a fragment too',
    phaseFor('tauri://localhost/sdkjs/cell/sdk-all.js?v=1#x') === 'sdk_all');
  check(results, 'any other script ends no phase',
    phaseFor('tauri://localhost/web-apps/vendor/requirejs/require.js') === null
      && phaseFor('tauri://localhost/sdkjs/word/not-sdk-all.js') === null
      && phaseFor('') === null);
}

// The marks taken from the editor frame (editor-patches.js).
{
  const { bridge } = await loadWith(() => 'Ymlu');
  const source = (await readFile(path.join(projectRoot, 'src/editor-patches.js'), 'utf8')).replace(/\r\n/g, '\n');
  const start = source.indexOf('    function _installSdkLoadMarks(win) {');
  const last = source.indexOf('    function _markSdkScriptLoaded(event) {', start);
  const end = source.indexOf('\n    }\n', last);
  bridge.run(source.slice(start, end + 6) + '\nwindow._installSdkLoadMarks = _installSdkLoadMarks;');
  const install = bridge.sandbox._installSdkLoadMarks;

  class FakeDocument {
    constructor() { this.listeners = []; }
    addEventListener(type, fn, capture) { this.listeners.push({ type, fn, capture }); }
    load(target) { this.listeners.filter((l) => l.type === 'load').forEach((l) => l.fn({ target })); }
  }
  const script = (src) => ({ tagName: 'SCRIPT', src });
  const frame = { document: new FakeDocument() };

  bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(10);
  install(frame);
  install(frame);
  const blank = frame.document;
  check(results, 'one capturing load listener per document',
    blank.listeners.length === 1 && blank.listeners[0].type === 'load' && blank.listeners[0].capture === true,
    JSON.stringify(blank.listeners.length));

  // The frame leaves its initial about:blank for the editor.
  frame.document = new FakeDocument();
  install(frame);
  check(results, 'the document the frame navigates to gets its own listener',
    frame.document.listeners.length === 1);

  frame.document.load({ tagName: 'IMG', src: 'tauri://localhost/sdkjs/word/sdk-all-min.js' });
  frame.document.load(script('tauri://localhost/web-apps/vendor/requirejs/require.js'));
  frame.document.load(script('tauri://localhost/sdkjs/word/sdk-all-min.js'));
  frame.document.load(script('tauri://localhost/sdkjs/word/sdk-all.js'));
  frame.document.load(script('tauri://localhost/sdkjs/word/sdk-all.js'));
  bridge.stopTimers();
  check(results, 'the two sdk scripts are marked once each when they load',
    phaseNames(bridge) === 'request,result,sdk_min,sdk_all', phaseNames(bridge));
}

finish(results);
