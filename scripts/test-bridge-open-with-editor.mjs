// Checks an open whose editor is created while the backend converts the
// document (bridge.js, _eoOpenWithEditor): the editor is asked for at once,
// sdkjs's request for the document (LocalStartOpen) waits for the answer and
// never gets a blank document in its place, the editor stays behind the page
// until the document arrives, and a failed open leaves the start screen as
// before, loading the page again when an editor had been created. Also the real openEditor of editor-patches.js, with the
// docs API replaced.
//
// Run: node scripts/test-bridge-open-with-editor.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const REPORT = { token: 'tok-report', name: 'report.docx' };
const BYTES = Buffer.from('DOCY;v5;4;AAAA').toString('base64');

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// Longer than the 100 ms LocalStartOpen waits before it opens the document.
const AFTER_START_OPEN = 160;

function element(...classes) {
  const names = new Set(classes);
  return {
    classList: {
      add: (name) => names.add(name),
      remove: (name) => names.delete(name),
      contains: (name) => names.has(name),
      toggle: (name, on) => (on ? names.add(name) : names.delete(name)),
    },
    inert: false,
    textContent: '',
  };
}

// A page with the start screen, the opening indicator and an editor frame to
// remove, and a backend whose open_file answers when the check says so.
async function loadPage({ fonts = 'ready' } = {}) {
  const page = { opens: [], editors: [], loaded: [], blank: 0, messages: [], reloads: 0, removedFrames: 0, commands: [] };
  const bridge = await loadBridge({
    invoke: {
      fonts_status: () => fonts,
      open_file: (args) => new Promise((resolve, reject) => { page.opens.push({ args, resolve, reject }); }),
      create_new: () => { page.commands.push('create_new'); return BYTES; },
      recovery_end: () => { page.commands.push('recovery_end'); return null; },
      close_document: () => { page.commands.push('close_document'); return null; },
    },
  });
  const elements = {
    'start-screen': element(),
    opening: element(),
    'opening-text': element(),
    placeholder: element(),
  };
  const frame = { remove: () => { page.removedFrames += 1; page.frames = []; } };
  page.frames = [];
  bridge.sandbox.document.body = element();
  bridge.sandbox.document.getElementById = (id) => elements[id] || null;
  bridge.sandbox.document.querySelectorAll = (selector) => (selector === 'iframe' ? page.frames : []);
  bridge.sandbox.location = { reload: () => { page.reloads += 1; } };
  bridge.sandbox.__TAURI__.dialog = {
    message: (text) => new Promise((resolve) => { page.messages.push({ text, close: resolve }); }),
  };
  // What openEditor does once it creates the editor of a document not here yet.
  bridge.sandbox._openEditor = (docType, awaited) => {
    page.editors.push({ docType, awaited, state: awaited && awaited.state });
    if (!awaited) return;
    awaited.editorCreated = true;
    if (awaited.state === 'converting') bridge.sandbox._eoSetEditorBehind(true);
    page.frames = [frame];
  };
  // The editor frame's API, as far as LocalStartOpen reaches it.
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { openDocument: () => { page.blank += 1; } } },
    AscCommon: { getEmpty: () => new Uint8Array(4), OpenFileResult: function() {}, History: {} },
  };
  bridge.sandbox._loadEditorBin = (data, fileName, options) => { page.loaded.push({ data, fileName, options }); };
  bridge.sandbox._eoSetDocumentUrls = () => {};
  bridge.sandbox._ensureCoreProps = () => {};
  bridge.sandbox._recoveryBegin = () => {};
  page.bridge = bridge;
  page.elements = elements;
  page.opening = () => elements.opening.classList.contains('active');
  page.startHidden = () => elements['start-screen'].classList.contains('hidden');
  page.startInert = () => elements['start-screen'].inert;
  page.editorBehind = () => bridge.sandbox.document.body.classList.contains('editor-behind');
  page.t = (key) => bridge.run(`_t(${JSON.stringify(key)})`);
  page.phases = () => bridge.logs
    .filter((line) => line.startsWith('[OPEN] phase='))
    .map((line) => /phase=(\w+)/.exec(line)[1])
    .join();
  return page;
}

async function startOpen(options) {
  const page = await loadPage(options);
  page.opened = page.bridge.run(`_eoOpenGranted(${JSON.stringify(REPORT)})`);
  await sleep(20);
  return page;
}

const startOpenRequest = (page) => page.bridge.run('window.AscDesktopEditor.LocalStartOpen();');

// The editor is created while the backend converts.
{
  const page = await startOpen();
  check(results, 'the backend is asked for the document once', page.opens.length === 1
    && page.opens[0].args.token === REPORT.token, JSON.stringify(page.opens.map((open) => open.args)));
  check(results, 'the editor is created before the backend answers',
    page.editors.length === 1 && page.editors[0].docType === 'word' && page.editors[0].state === 'converting',
    JSON.stringify(page.editors.map((editor) => [editor.docType, editor.state])));
  check(results, 'it is told the name of the document it waits for',
    page.editors.length === 1 && page.editors[0].awaited && page.editors[0].awaited.name === REPORT.name);
  check(results, 'it loads behind the start screen and the opening indicator, as they were',
    page.opening() && page.editorBehind() && !page.startHidden(),
    `opening=${page.opening()} behind=${page.editorBehind()} hidden=${page.startHidden()}`);
  check(results, 'no document is pending yet', page.bridge.sandbox._pendingFileData === undefined);

  // sdkjs asks for the document before the backend has answered.
  startOpenRequest(page);
  await sleep(AFTER_START_OPEN);
  check(results, 'an editor that is ready first gets no blank document', page.blank === 0, String(page.blank));
  check(results, '... and no document at all until the answer', page.loaded.length === 0, String(page.loaded.length));

  page.opens[0].resolve(BYTES);
  await sleep(20);
  check(results, 'the answer gives the window to the editor: no indicator, no start screen',
    !page.opening() && !page.editorBehind() && page.startHidden(),
    `opening=${page.opening()} behind=${page.editorBehind()} hidden=${page.startHidden()}`);
  check(results, '... and is not opened before the wait LocalStartOpen always makes',
    page.loaded.length === 0, String(page.loaded.length));
  await sleep(AFTER_START_OPEN);
  check(results, 'the waiting editor then opens the document of the answer, once',
    page.loaded.length === 1 && page.loaded[0].data === BYTES && page.loaded[0].fileName === REPORT.name
      && page.loaded[0].options.path === REPORT.name && page.blank === 0,
    JSON.stringify(page.loaded.map((loaded) => loaded.fileName)));
  check(results, 'the marks are request, doc_request and then result',
    page.phases() === 'request,doc_request,result', page.phases());
  check(results, 'the open resolves', await Promise.race([page.opened.then(() => true), sleep(20).then(() => false)]));

  // sdkjs never asks twice, but a second request must not blank the document.
  startOpenRequest(page);
  await sleep(AFTER_START_OPEN);
  check(results, 'a second request leaves the open document as it is',
    page.loaded.length === 1 && page.blank === 0, `${page.loaded.length} ${page.blank}`);
  page.bridge.stopTimers();
}

// The backend answers first, as on a machine whose editor loads slowly.
{
  const page = await startOpen();
  page.opens[0].resolve(BYTES);
  await sleep(20);
  check(results, 'answer first: the document waits for the editor',
    page.bridge.sandbox._pendingFileData && page.bridge.sandbox._pendingFileData.data === BYTES
      && page.loaded.length === 0);
  startOpenRequest(page);
  await sleep(AFTER_START_OPEN);
  check(results, 'answer first: the editor opens it when it asks',
    page.loaded.length === 1 && page.loaded[0].data === BYTES && page.blank === 0);
  check(results, 'answer first: the marks are request, result and then doc_request',
    page.phases() === 'request,result,doc_request', page.phases());
  page.bridge.stopTimers();
}

// The font cache is still being built: the conversion does not wait for it,
// the editor does.
{
  const page = await startOpen({ fonts: 'pending' });
  check(results, 'fonts pending: the backend is asked at once', page.opens.length === 1);
  check(results, 'fonts pending: no editor yet', page.editors.length === 0, String(page.editors.length));
  page.opens[0].resolve(BYTES);
  await sleep(20);
  check(results, 'fonts pending: the answer is kept and the indicator stays for the editor to take down',
    page.bridge.sandbox._pendingFileData.data === BYTES && page.opening());
  page.bridge.emit('fonts-ready', 'ready');
  await sleep(20);
  check(results, 'fonts pending: the editor is created once they are ready, its document already here',
    page.editors.length === 1 && page.editors[0].state === 'arrived',
    JSON.stringify(page.editors.map((editor) => editor.state)));
  page.bridge.stopTimers();
}

// The backend refuses before there is an editor: the page stays as it was.
{
  const page = await startOpen({ fonts: 'pending' });
  page.opens[0].reject('x2t exit code 80');
  await sleep(20);
  check(results, 'refused, no editor: the usual message over the start screen',
    page.messages.length === 1 && page.messages[0].text === page.t('openErrorMsg')
      && !page.opening() && !page.startHidden() && !page.startInert(),
    JSON.stringify(page.messages.map((message) => message.text)));
  page.bridge.emit('fonts-ready', 'ready');
  await sleep(20);
  check(results, 'refused, no editor: none is created when the fonts are ready',
    page.editors.length === 0, String(page.editors.length));
  page.messages[0].close();
  await sleep(20);
  check(results, 'refused, no editor: the page is not loaded again', page.reloads === 0 && page.removedFrames === 0,
    `${page.reloads} ${page.removedFrames}`);

  // The page goes on: a new document in it opens as before.
  page.bridge.sandbox._pendingFileData = { data: BYTES, path: null, name: 'Document' };
  startOpenRequest(page);
  await sleep(AFTER_START_OPEN);
  check(results, 'refused, no editor: a new document in the same page still opens',
    page.loaded.length === 1 && page.loaded[0].options.name === 'Document', String(page.loaded.length));
  page.bridge.stopTimers();
}

// The backend refuses with the editor already created.
for (const [label, rejection, messageKey] of [
  ['damaged', 'x2t exit code 80', 'openErrorMsg'],
  ['open elsewhere', 'open-elsewhere', 'openElsewhereMsg'],
]) {
  const page = await startOpen();
  startOpenRequest(page);
  page.opens[0].reject(rejection);
  await sleep(20);
  check(results, `refused with an editor (${label}): the editor frame is removed`,
    page.removedFrames === 1, String(page.removedFrames));
  check(results, `refused with an editor (${label}): the start screen alone is left, taking no clicks`,
    !page.startHidden() && page.startInert() && !page.opening() && !page.editorBehind(),
    `hidden=${page.startHidden()} inert=${page.startInert()} opening=${page.opening()} behind=${page.editorBehind()}`);
  check(results, `refused with an editor (${label}): the same message as without one`,
    page.messages.length === 1 && page.messages[0].text === page.t(messageKey),
    JSON.stringify(page.messages.map((message) => message.text)));
  await sleep(AFTER_START_OPEN);
  check(results, `refused with an editor (${label}): the waiting editor opens nothing, blank or not`,
    page.loaded.length === 0 && page.blank === 0, `${page.loaded.length} ${page.blank}`);
  check(results, `refused with an editor (${label}): the page stays while the message is up`, page.reloads === 0);
  page.messages[0].close();
  await sleep(20);
  check(results, `refused with an editor (${label}): the page is loaded again once it is closed`,
    page.reloads === 1 && page.commands.includes('close_document'), `${page.reloads} ${page.commands.join()}`);
  check(results, `refused with an editor (${label}): nothing runs after the message in the old page`,
    await Promise.race([page.opened.then(() => false), sleep(20).then(() => true)]));
  page.bridge.stopTimers();
}

// The dialog of a caller that words its own failure.
{
  const page = await loadPage();
  const told = Promise.resolve('closed');
  const same = page.bridge.sandbox._eoAfterOpenFailure && page.bridge.sandbox._eoAfterOpenFailure(told) === told;
  check(results, 'without an editor to leave, the dialog of a failed open is returned as it is', same === true);
  page.bridge.stopTimers();
}

// A new document, which no backend answer is awaited for, opens as before.
{
  const page = await loadPage();
  startOpenRequest(page);
  await sleep(AFTER_START_OPEN);
  check(results, 'no open, no pending document: the blank document, as before', page.blank === 1 && page.loaded.length === 0,
    `${page.blank} ${page.loaded.length}`);
  page.bridge.stopTimers();
}

// The real openEditor of editor-patches.js, from its comment to its end, with
// the docs API, the system fonts and the start screen replaced.
const patches = (await readFile(path.join(projectRoot, 'src/editor-patches.js'), 'utf8')).replace(/\r\n/g, '\n');

function loadOpenEditor(page) {
  const start = patches.indexOf('    async function openEditor(');
  const end = patches.indexOf('\n    }\n', start);
  if (start < 0 || end < 0) throw new Error('editor-patches.js: cannot find openEditor');
  page.created = [];
  page.bridge.sandbox.DocsAPI = { DocEditor: function(id, config) { page.created.push(config); } };
  page.bridge.sandbox.startScreen = page.elements['start-screen'];
  page.bridge.run(`
    function _prepareSystemFonts() { return Promise.resolve(''); }
    function _fileTypeFor() { return 'docx'; }
    function _log() {}
  ` + patches.slice(start, end + 6) + '\nwindow.__openEditor = openEditor;');
  return (docType, awaited) => page.bridge.sandbox.__openEditor(docType, awaited);
}

async function awaitedDocument(state) {
  const page = await loadPage();
  const openEditor = loadOpenEditor(page);
  page.bridge.run("_eoOpenPhases.begin(); _eoShowOpening('Opening report.docx');");
  const awaited = { name: REPORT.name, state, editorCreated: false };
  await openEditor('word', awaited);
  return { page, awaited };
}

{
  const { page, awaited } = await awaitedDocument('converting');
  check(results, 'openEditor: an awaited document gets its editor before it arrives',
    page.created.length === 1 && page.created[0].document.title === REPORT.name && awaited.editorCreated === true,
    `${page.created.length} ${awaited.editorCreated}`);
  check(results, 'openEditor: behind the start screen and the opening indicator, as they were',
    page.opening() && page.editorBehind() && !page.startHidden(),
    `opening=${page.opening()} behind=${page.editorBehind()} hidden=${page.startHidden()}`);
  check(results, 'openEditor: it asks for no blank document meanwhile',
    !page.commands.includes('create_new') && page.bridge.sandbox._pendingFileData === undefined, page.commands.join());
  page.bridge.stopTimers();
}
{
  const { page, awaited } = await awaitedDocument('failed');
  check(results, 'openEditor: a document that failed meanwhile gets no editor',
    page.created.length === 0 && awaited.editorCreated === false && !page.startHidden(),
    `${page.created.length} ${awaited.editorCreated} ${page.startHidden()}`);
  check(results, 'openEditor: ... and no fonts mark', !page.phases().includes('fonts'), page.phases());
  page.bridge.stopTimers();
}
{
  const { page } = await awaitedDocument('arrived');
  check(results, 'openEditor: a document already here gives it the window at once',
    page.created.length === 1 && !page.opening() && page.startHidden() && !page.editorBehind(),
    `${page.created.length} ${page.opening()} ${page.startHidden()} ${page.editorBehind()}`);
  page.bridge.stopTimers();
}
{
  const page = await loadPage();
  const openEditor = loadOpenEditor(page);
  page.bridge.run("_eoShowOpening('Opening');");
  page.bridge.sandbox._pendingFileData = { data: BYTES, path: 'a.docx', name: 'a.docx' };
  await openEditor('word');
  check(results, 'openEditor: with the document pending and none awaited, as before',
    page.created.length === 1 && page.created[0].document.title === 'a.docx' && !page.opening()
      && page.startHidden() && !page.editorBehind() && !page.commands.includes('create_new'),
    `${page.created.length} ${page.opening()} ${page.startHidden()}`);
  page.bridge.stopTimers();
}
{
  const page = await loadPage();
  const openEditor = loadOpenEditor(page);
  await openEditor('word');
  check(results, 'openEditor: with nothing pending and none awaited, the blank document is asked for, as before',
    page.commands.includes('create_new') && page.created.length === 1
      && page.bridge.sandbox._pendingFileData.data === BYTES, page.commands.join());
  page.bridge.stopTimers();
}

// What keeps the editor out of sight while it is behind the page.
{
  const html = await readFile(path.join(projectRoot, 'src/index.html'), 'utf8');
  check(results, 'index.html covers an editor that is behind with the page background and the start screen',
    /body\.editor-behind::before \{[^}]*position: fixed;[^}]*background: inherit;/.test(html)
      && /body\.editor-behind \.start \{ position: relative; z-index: 2; \}/.test(html));
  check(results, 'index.html keeps that editor in the window, not below the start screen',
    /body\.editor-behind iframe\[name="frameEditor"\] \{ position: absolute; left: 0; top: 0; \}/.test(html));
}

// The start screen's opens in editor-patches.js, which this harness does not run.
{
  const body = (marker) => {
    const start = patches.indexOf(marker);
    return start < 0 ? '' : patches.slice(start, patches.indexOf('\n    }', start));
  };
  check(results, 'the start screen\'s Open creates the editor while the backend converts',
    body('async function _openPath(file, silent) {').includes('await window._eoOpenWithEditor(file, openId)'));
  check(results, 'the file reopened after a reload does too',
    body('(function checkPendingOpen() {').includes('window._eoOpenWithEditor(pending, openId)'));
  check(results, 'a recent file that fails to open leaves its editor through _eoAfterOpenFailure',
    body('async function _openRecentFile(file) {').includes('await window._eoAfterOpenFailure(window.__TAURI__.dialog.message('));
}

finish(results);
