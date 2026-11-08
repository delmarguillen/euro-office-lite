// Checks the files waiting in the backend (pending_open.rs): the launch
// argument and, on macOS, the files the Finder hands over later with a
// "pending-open-file" event. Each goes through LocalFileOpen: a new window
// with a document mounted, this window on the start screen. An open in
// progress is waited for, and no file opens twice or gets lost.
//
// Run: node scripts/test-bridge-pending-open.mjs [path/to/bridge.js]
import { check, finish, loadBridge, pendingQueue } from './bridge-test-harness.mjs';

const results = [];
const LAUNCH = { token: 'tok-launch', name: 'launch.docx' };
const A = { token: 'tok-a', name: 'a.docx' };
const B = { token: 'tok-b', name: 'b.xlsx' };

// The indicator the bridge reads to know an open is in progress.
function fakeOpeningOverlay() {
  const classes = new Set();
  return {
    classList: {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
    },
  };
}

// `waiting`: what the queue holds when the page loads.
async function setup({ waiting = [], docType = null } = {}) {
  const queue = [];
  const take = pendingQueue(...waiting);
  const env = { newWindow: [], sameWindow: [], queue };
  const bridge = await loadBridge({
    invoke: {
      // What waited at load first, then whatever the Finder hands over.
      take_pending_open_file: () => take() || queue.shift() || null,
      open_in_new_window: (args) => { env.newWindow.push(args.token); return 'opened'; },
    },
  });
  env.bridge = bridge;
  env.overlay = fakeOpeningOverlay();
  bridge.sandbox.document.getElementById = (id) => (id === 'opening' ? env.overlay : null);
  // The start screen's open, as editor-patches.js does it: the indicator stays
  // up until the editor is mounted.
  bridge.sandbox._eoOpenPath = (file) => {
    env.sameWindow.push(file.token);
    env.overlay.classList.add('active');
    return Promise.resolve(true);
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  return env;
}

// What the Finder does after launch: queue the files, then send the event.
function handOver(env, ...files) {
  env.queue.push(...files);
  env.bridge.emit('pending-open-file');
}

// The open in progress ends with the editor mounted.
function editorMounted(env) {
  env.bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
  env.bridge.run('_eoHideOpening()');
}

const settle = (env) => new Promise((resolve) => setTimeout(resolve, 20))
  .then(() => env.bridge.stopTimers());

{
  const env = await setup({ docType: 'word' });
  handOver(env, A);
  await settle(env);
  check(results, 'With a document: the file goes to open_in_new_window',
    env.newWindow.join() === 'tok-a' && env.sameWindow.length === 0,
    `newWindow=${env.newWindow} sameWindow=${env.sameWindow}`);
}

{
  const env = await setup();
  handOver(env, A);
  await settle(env);
  check(results, 'On the start screen: the file opens in this window',
    env.sameWindow.join() === 'tok-a' && env.newWindow.length === 0,
    `newWindow=${env.newWindow} sameWindow=${env.sameWindow}`);
}

{
  const env = await setup();
  env.overlay.classList.add('active');
  handOver(env, A);
  await settle(env);
  const waited = env.sameWindow.length === 0 && env.newWindow.length === 0;
  editorMounted(env);
  await settle(env);
  check(results, 'With an open in progress: the file waits and opens when it ends',
    waited && env.newWindow.join() === 'tok-a' && env.sameWindow.length === 0,
    `waited=${waited} newWindow=${env.newWindow} sameWindow=${env.sameWindow}`);
}

{
  const env = await setup();
  env.overlay.classList.add('active');
  handOver(env, A);
  await settle(env);
  // The open in progress failed: the start screen is back.
  env.bridge.run('_eoHideOpening()');
  await settle(env);
  check(results, 'With an open in progress that fails: the file opens in this window',
    env.sameWindow.join() === 'tok-a' && env.newWindow.length === 0,
    `newWindow=${env.newWindow} sameWindow=${env.sameWindow}`);
}

{
  const env = await setup({ docType: 'word' });
  handOver(env, A);
  handOver(env, B);
  await settle(env);
  check(results, 'Two files in a row with a document: both open in new windows',
    env.newWindow.join() === 'tok-a,tok-b',
    `newWindow=${env.newWindow}`);
}

{
  const env = await setup();
  handOver(env, A, B);
  await settle(env);
  const firstHere = env.sameWindow.join() === 'tok-a' && env.newWindow.length === 0;
  editorMounted(env);
  await settle(env);
  check(results, 'Two files at once on the start screen: the first here, the second in a new window',
    firstHere && env.sameWindow.join() === 'tok-a' && env.newWindow.join() === 'tok-b',
    `firstHere=${firstHere} newWindow=${env.newWindow} sameWindow=${env.sameWindow}`);
}

{
  // The page takes the launch argument when it loads; an event for it may
  // arrive at the same time.
  const env = await setup({ waiting: [LAUNCH] });
  env.bridge.emit('pending-open-file');
  await settle(env);
  check(results, 'Cold start: the launch file opens once',
    env.sameWindow.join() === 'tok-launch' && env.newWindow.length === 0,
    `newWindow=${env.newWindow} sameWindow=${env.sameWindow}`);
}

{
  // The Finder's files arrive before the page asked: they wait in the queue
  // and the request on load collects them.
  const env = await setup({ waiting: [A, B] });
  await settle(env);
  const firstHere = env.sameWindow.join() === 'tok-a';
  editorMounted(env);
  await settle(env);
  check(results, 'Cold start with two files: each opens once, the second in a new window',
    firstHere && env.sameWindow.join() === 'tok-a' && env.newWindow.join() === 'tok-b',
    `firstHere=${firstHere} newWindow=${env.newWindow} sameWindow=${env.sameWindow}`);
}

finish(results);
