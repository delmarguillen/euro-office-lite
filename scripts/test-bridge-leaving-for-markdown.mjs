// Checks the start screen once it is going to the Markdown page (open a .md,
// New Markdown, Recover a Markdown document): that page shows no opening
// indicator, so nothing else must start opening here meanwhile, or the
// backend's open document would end up being another file than the one the
// Markdown page shows. Files still in the backend's queue stay there for the
// Markdown page, which takes them when it loads; a file already in hand (a
// dialog's, or one taken from the queue) goes to a new window.
//
// Run: node scripts/test-bridge-leaving-for-markdown.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const NOTES = { token: 'tok-notes', name: 'a.md' };
const REPORT = { token: 'tok-report', name: 'b.docx' };

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

async function setup() {
  const env = { queue: [], sameWindow: [], newWindow: [], dialogs: [] };
  const bridge = await loadBridge({
    invoke: {
      take_pending_open_file: () => env.queue.shift() || null,
      pick_document: () => new Promise((resolve) => { env.dialogs.push(resolve); }),
      open_in_new_window: (args) => { env.newWindow.push(args.token); return 'opened'; },
    },
  });
  env.bridge = bridge;
  env.overlay = fakeOpeningOverlay();
  bridge.sandbox.document.getElementById = (id) => (id === 'opening' ? env.overlay : null);
  bridge.sandbox.location = { href: 'index.html' };
  // editor-patches.js, _openPath: Markdown goes to its page, anything else
  // opens here behind the indicator.
  bridge.sandbox._eoOpenPath = (file) => {
    if (bridge.sandbox._eoIsMarkdownPath(file.name)) {
      bridge.sandbox._eoOpenMarkdown(file);
      return Promise.resolve(true);
    }
    env.sameWindow.push(file.token);
    env.overlay.classList.add('active');
    return Promise.resolve(true);
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = null;
  env.markdownPage = () => String(bridge.sandbox.location.href).startsWith('markdown/markdown.html');
  return env;
}

function handOver(env, ...files) {
  env.queue.push(...files);
  env.bridge.emit('pending-open-file');
}

const settle = (env) => new Promise((resolve) => setTimeout(resolve, 20))
  .then(() => env.bridge.stopTimers());

const state = (env) =>
  `markdownPage=${env.markdownPage()} sameWindow=${env.sameWindow} newWindow=${env.newWindow} queue=${env.queue.map((file) => file.token)}`;

{
  const env = await setup();
  handOver(env, NOTES, REPORT);
  await settle(env);
  check(results, 'a.md and b.docx from the Finder: b.docx does not start opening here',
    env.markdownPage() && env.sameWindow.length === 0, state(env));
  check(results, '... and stays in the queue for the Markdown page',
    env.queue.length === 1 && env.queue[0].token === REPORT.token && env.newWindow.length === 0, state(env));
}

{
  const env = await setup();
  env.bridge.run('AscDesktopEditor.LocalFileOpen()');
  await settle(env);
  handOver(env, NOTES);
  await settle(env);
  env.dialogs.shift()(REPORT);
  await settle(env);
  check(results, 'a file chosen in Open while the page goes to a.md opens in a new window',
    env.markdownPage() && env.sameWindow.length === 0 && env.newWindow.join() === REPORT.token, state(env));
}

{
  const env = await setup();
  env.bridge.run('_eoNewMarkdown()');
  handOver(env, REPORT);
  await settle(env);
  check(results, 'after New Markdown, a file from the Finder stays in the queue',
    env.sameWindow.length === 0 && env.queue.length === 1 && env.newWindow.length === 0, state(env));
}

{
  const env = await setup();
  env.bridge.run("_eoRecoverMarkdown('rec-1')");
  await env.bridge.run(`AscDesktopEditor.LocalFileOpen(${JSON.stringify(REPORT)})`);
  await settle(env);
  check(results, 'after Recover of a Markdown document, a file in hand opens in a new window',
    env.sameWindow.length === 0 && env.newWindow.join() === REPORT.token, state(env));
}

{
  const env = await setup();
  handOver(env, REPORT);
  await settle(env);
  check(results, 'without Markdown, a file from the Finder still opens here',
    env.sameWindow.join() === REPORT.token && env.newWindow.length === 0, state(env));
}

finish(results);
