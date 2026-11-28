// Checks a file chosen in an open dialog while another open has started in
// this window during the dialog (on macOS, a file the Finder hands over:
// pending_open.rs). The start screen's Open file button (its real handler in
// editor-patches.js), Ctrl+O, Edit PDF and New from template all wait for the
// open in progress to end, then open the chosen file in a new window or, if the
// other open failed, here. Never two opens at once in one window: the backend
// and the editor could end up with different files, and a save would write one
// document to the other's path. Without another open, nothing changes.
//
// Run: node scripts/test-bridge-open-during-dialog.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const FINDER = { token: 'tok-finder', name: 'finder.docx' };
const CHOSEN = { token: 'tok-chosen', name: 'chosen.docx' };
const CHOSEN_PDF = { token: 'tok-chosen-pdf', name: 'chosen.pdf' };
const TEMPLATE = { token: 'tok-template', name: 'Letter.dotx' };

const START = "document.querySelector('.btn[data-open]').addEventListener('click', ";
const END = '\n    });';

// The Open file button's click handler, cut from editor-patches.js.
async function openButtonHandlerSource() {
  const raw = await readFile(path.join(projectRoot, 'src', 'editor-patches.js'), 'utf8');
  const source = raw.replace(/\r\n/g, '\n');
  const start = source.indexOf(START);
  const end = source.indexOf(END, start);
  if (start < 0 || end < 0) throw new Error('editor-patches.js: cannot find the Open file button handler');
  return source.slice(start + START.length, end + END.length - ');'.length);
}

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

// Every dialog stays up until the check closes it with `choose`.
async function setup({ docType = null } = {}) {
  const env = { queue: [], sameWindow: [], newWindow: [], templates: [], newTemplateWindow: [], dialogs: [] };
  const dialog = (command) => () => new Promise((resolve) => { env.dialogs.push({ command, resolve }); });
  const bridge = await loadBridge({
    invoke: {
      take_pending_open_file: () => env.queue.shift() || null,
      pick_document: dialog('pick_document'),
      pick_pdf: dialog('pick_pdf'),
      pick_template: dialog('pick_template'),
      open_in_new_window: (args) => { env.newWindow.push(args.token); return 'opened'; },
      new_document_in_new_window: (args) => { env.newTemplateWindow.push(args.token); return 'opened'; },
      create_from_template: (args) => {
        env.templates.push(args.token);
        env.overlay.classList.add('active');
        return new Promise(() => {});
      },
    },
  });
  env.bridge = bridge;
  env.overlay = fakeOpeningOverlay();
  const elements = { opening: env.overlay, 'opening-text': { textContent: '' } };
  bridge.sandbox.document.getElementById = (id) => elements[id] || null;
  // The start screen's open in this window (editor-patches.js, _openPath):
  // the indicator stays up until the editor is mounted.
  bridge.sandbox._eoOpenPath = (file) => {
    env.sameWindow.push(file.token);
    env.overlay.classList.add('active');
    return Promise.resolve(true);
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  env.choose = (file) => env.dialogs.shift().resolve(file);
  return env;
}

// Installs the real button handler against this bridge and clicks it.
async function clickOpenButton(env) {
  const handlerSource = await openButtonHandlerSource();
  env.bridge.sandbox._openPath = env.bridge.sandbox._eoOpenPath;
  env.bridge.run(`var __openButtonClick = ${handlerSource};`);
  env.bridge.run('__openButtonClick()');
}

const PRESS = {
  'Open file button': clickOpenButton,
  'Ctrl+O': (env) => { env.bridge.run('AscDesktopEditor.LocalFileOpen()'); },
};

// The Finder hands a file over while the dialog is up: it starts opening here.
function finderHandsOver(env, file) {
  env.queue.push(file);
  env.bridge.emit('pending-open-file');
}

function editorMounted(env) {
  env.bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
  env.bridge.run('_eoHideOpening()');
}

function openFailed(env) {
  env.bridge.run('_eoHideOpening()');
}

const settle = (env) => new Promise((resolve) => setTimeout(resolve, 20))
  .then(() => env.bridge.stopTimers());

const state = (env) => `sameWindow=${env.sameWindow} newWindow=${env.newWindow}`;

for (const [label, press] of Object.entries(PRESS)) {
  {
    const env = await setup();
    await press(env);
    await settle(env);
    env.choose(CHOSEN);
    await settle(env);
    check(results, `${label}, nothing else opening: the file opens here`,
      env.sameWindow.join() === CHOSEN.token && env.newWindow.length === 0, state(env));
  }

  {
    const env = await setup();
    await press(env);
    await settle(env);
    finderHandsOver(env, FINDER);
    await settle(env);
    env.choose(CHOSEN);
    await settle(env);
    const waited = env.sameWindow.join() === FINDER.token && env.newWindow.length === 0;
    editorMounted(env);
    await settle(env);
    check(results, `${label}, the Finder's file opening: the chosen file waits, no second open here`,
      waited, state(env));
    check(results, `... and opens in a new window once the Finder's file is open`,
      env.sameWindow.join() === FINDER.token && env.newWindow.join() === CHOSEN.token, state(env));
  }

  {
    // With the editor created during the conversion, the document type is
    // set while the indicator is still up.
    const env = await setup();
    await press(env);
    await settle(env);
    finderHandsOver(env, FINDER);
    await settle(env);
    env.bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
    env.choose(CHOSEN);
    await settle(env);
    const waited = env.sameWindow.join() === FINDER.token && env.newWindow.length === 0;
    env.bridge.run('_eoHideOpening()');
    await settle(env);
    check(results, `${label}, the Finder's file converting with its editor created: the chosen file waits`,
      waited, state(env));
    check(results, '... then opens in a new window',
      env.sameWindow.join() === FINDER.token && env.newWindow.join() === CHOSEN.token, state(env));
  }

  {
    const env = await setup();
    await press(env);
    await settle(env);
    finderHandsOver(env, FINDER);
    await settle(env);
    env.choose(CHOSEN);
    await settle(env);
    const waited = env.sameWindow.join() === FINDER.token;
    openFailed(env);
    await settle(env);
    check(results, `${label}, the Finder's file failing: the chosen file opens here after it, not during`,
      waited && env.sameWindow.join() === `${FINDER.token},${CHOSEN.token}` && env.newWindow.length === 0,
      `waited=${waited} ${state(env)}`);
  }

  {
    const env = await setup();
    await press(env);
    await settle(env);
    finderHandsOver(env, FINDER);
    await settle(env);
    editorMounted(env);
    await settle(env);
    env.choose(CHOSEN);
    await settle(env);
    check(results, `${label}, the Finder's file already open: the chosen file goes to a new window`,
      env.sameWindow.join() === FINDER.token && env.newWindow.join() === CHOSEN.token, state(env));
  }
}

// Edit PDF opens what it picks as Open would.
{
  const env = await setup();
  const done = env.bridge.run('_eoEditPdf()');
  await settle(env);
  finderHandsOver(env, FINDER);
  await settle(env);
  env.choose(CHOSEN_PDF);
  await settle(env);
  const waited = env.sameWindow.join() === FINDER.token && env.newWindow.length === 0;
  editorMounted(env);
  await done;
  await settle(env);
  check(results, "Edit PDF, the Finder's file opening: the PDF waits, then opens in a new window",
    waited && env.sameWindow.join() === FINDER.token && env.newWindow.join() === CHOSEN_PDF.token,
    `waited=${waited} ${state(env)}`);
}

{
  const env = await setup();
  const done = env.bridge.run('_eoEditPdf()');
  await settle(env);
  env.choose(CHOSEN_PDF);
  await done;
  await settle(env);
  check(results, 'Edit PDF, nothing else opening: the PDF opens here',
    env.sameWindow.join() === CHOSEN_PDF.token && env.newWindow.length === 0, state(env));
}

// New from template starts a new document, here or in a new window.
{
  const env = await setup();
  env.bridge.run('_eoNewFromTemplate()');
  await settle(env);
  finderHandsOver(env, FINDER);
  await settle(env);
  env.choose(TEMPLATE);
  await settle(env);
  const waited = env.templates.length === 0 && env.newTemplateWindow.length === 0;
  editorMounted(env);
  await settle(env);
  check(results, "New from template, the Finder's file opening: the template waits, then starts a new window",
    waited && env.templates.length === 0 && env.newTemplateWindow.join() === TEMPLATE.token,
    `waited=${waited} templates=${env.templates} newTemplateWindow=${env.newTemplateWindow}`);
}

{
  const env = await setup();
  env.bridge.run('_eoNewFromTemplate()');
  await settle(env);
  env.choose(TEMPLATE);
  await settle(env);
  check(results, 'New from template, nothing else opening: the new document starts here',
    env.templates.join() === TEMPLATE.token && env.newTemplateWindow.length === 0,
    `templates=${env.templates} newTemplateWindow=${env.newTemplateWindow}`);
}

// Ctrl+O while the indicator is up still shows no dialog.
{
  const env = await setup();
  env.overlay.classList.add('active');
  env.bridge.run('AscDesktopEditor.LocalFileOpen()');
  await settle(env);
  check(results, 'Ctrl+O during an open shows no dialog', env.dialogs.length === 0, String(env.dialogs.length));
}

finish(results);
