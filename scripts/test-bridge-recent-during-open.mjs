// Checks a recent file clicked on the start screen (editor-patches.js,
// _openRecentFile) when another open starts in this window while the click
// asks the backend for a fresh token (recent_files_state; on macOS, a file the
// Finder hands over). The recent file then goes through LocalFileOpen, which
// waits for that open and sends it to a new window, instead of starting a
// second open here: the editor and the backend could end up with different
// files. Without another open, and for a file no longer listed, nothing
// changes. The start screen code is the real one, cut from editor-patches.js.
//
// Run: node scripts/test-bridge-recent-during-open.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const FINDER = { token: 'tok-finder', name: 'finder.docx' };
// As the start screen listed them, with tokens that may have expired since.
const REPORT = { path: '/docs/report.docx', token: 'tok-stale-report', name: 'report.docx' };
const NOTES = { path: '/docs/notes.md', token: 'tok-stale-notes', name: 'notes.md' };
const fresh = (entry) => ({ ...entry, token: entry.token.replace('stale', 'fresh') });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// Longer than the 100 ms LocalStartOpen waits before it opens the document.
const AFTER_START_OPEN = 160;

// Cut from editor-patches.js, from the first marker up to the second one, or
// up to the end of the function that starts at the first.
async function cutSource(startMarker, endMarker) {
  const raw = await readFile(path.join(projectRoot, 'src', 'editor-patches.js'), 'utf8');
  const source = raw.replace(/\r\n/g, '\n');
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start);
  if (start < 0 || end < 0) throw new Error(`editor-patches.js: cannot find ${startMarker}`);
  return source.slice(start, end);
}

const openPathSource = () => cutSource('    async function _openPath(file, silent) {', '\n    }\n')
  .then((source) => source + '\n    }\n');
const recentSource = () => cutSource('    // ── Recent files (Issue #13) ──', '    // Picks the start screen icon');

function element() {
  const names = new Set();
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

// `listed`: the entries recent_files_state answers with. Its first answer
// waits for answerRecent(), the later ones do not.
async function loadPage({ listed }) {
  const page = { opens: [], newWindow: [], editors: [], loaded: [], messages: [], saveTarget: null };
  let answerRecent;
  const firstAnswer = new Promise((resolve) => { answerRecent = resolve; });
  let recentCalls = 0;
  const bridge = await loadBridge({
    invoke: {
      fonts_status: () => 'ready',
      recent_files_state: () => {
        recentCalls += 1;
        const state = { enabled: true, files: listed };
        return recentCalls === 1 ? firstAnswer.then(() => state) : state;
      },
      open_file: (args) => {
        if (args.token.includes('stale')) return Promise.reject('Unknown or expired file token');
        return new Promise((resolve, reject) => { page.opens.push({ token: args.token, resolve, reject }); });
      },
      open_in_new_window: (args) => { page.newWindow.push(args.token); return 'opened'; },
    },
  });
  const elements = {
    'start-screen': element(),
    opening: element(),
    'opening-text': element(),
    placeholder: element(),
  };
  bridge.sandbox.document.body = element();
  bridge.sandbox.document.getElementById = (id) => elements[id] || null;
  bridge.sandbox.location = { href: '' };
  bridge.sandbox.__TAURI__.dialog = {
    message: (text) => { page.messages.push(text); return Promise.resolve(); },
  };
  // What openEditor does with the document it is created for.
  bridge.sandbox._openEditor = (docType, awaited) => {
    if (awaited && awaited.state === 'failed') return;
    bridge.sandbox.AscDesktopEditor._currentDocType = docType;
    if (awaited && awaited.state === 'converting') {
      bridge.sandbox._eoSetEditorBehind(true);
    } else {
      elements['start-screen'].classList.add('hidden');
      bridge.sandbox._eoHideOpening();
    }
    if (awaited) awaited.editorCreated = true;
    page.editors.push(awaited && awaited.name);
    bridge.sandbox.AscDesktopEditor.LocalStartOpen();
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { openDocument: () => {} } },
    AscCommon: { getEmpty: () => new Uint8Array(4), OpenFileResult: function() {}, History: {} },
  };
  bridge.sandbox._loadEditorBin = (data, fileName) => { page.loaded.push(fileName); };
  bridge.sandbox._log = () => {};
  bridge.sandbox._renderRecentFiles = () => {};
  bridge.run(`${await openPathSource()}\nwindow._eoOpenPath = _openPath;\n${await recentSource()}\n`
    + 'window.__openRecent = _openRecentFile;');
  page.bridge = bridge;
  page.answerRecent = answerRecent;
  page.click = (entry) => bridge.run(`__openRecent(${JSON.stringify(entry)})`);
  page.finderHandsOver = (file) => bridge.run(`AscDesktopEditor.LocalFileOpen(${JSON.stringify(file)})`);
  page.requested = () => page.opens.map((open) => open.token).join();
  // The backend makes a converted file the one it saves to (open_file).
  page.answer = (file) => {
    const open = page.opens.find((entry) => entry.token === file.token);
    if (!open) return;
    page.saveTarget = file.name;
    open.resolve('bytes-' + file.token);
  };
  page.opening = () => elements.opening.classList.contains('active');
  page.t = (key) => bridge.run(`_t(${JSON.stringify(key)})`);
  page.state = () => `converted here=${page.requested()} newWindow=${page.newWindow} editor shows=${page.loaded} `
    + `backend saves to=${page.saveTarget} location=${bridge.sandbox.location.href} messages=${page.messages.length}`;
  return page;
}

// The Finder's file starts opening while the click waits for its token.
async function clickDuringFinderOpen(entry, listed) {
  const page = await loadPage({ listed });
  const clicked = page.click(entry);
  await sleep(20);
  page.finderHandsOver(FINDER);
  await sleep(20);
  page.answerRecent();
  await sleep(20);
  // The Finder's open keeps its indicator: nothing else ended it.
  page.waited = page.requested() === FINDER.token && page.newWindow.length === 0
    && page.bridge.sandbox.location.href === '' && page.opening();
  page.answer(FINDER);
  await sleep(AFTER_START_OPEN);
  // A second conversion here, if there was one, ends last.
  page.answer(fresh(entry));
  await sleep(AFTER_START_OPEN);
  page.clicked = clicked;
  return page;
}

{
  const page = await clickDuringFinderOpen(REPORT, [fresh(REPORT)]);
  check(results, 'recent during an open: not converted here, it waits', page.waited, page.state());
  check(results, '... then opens in a new window with its fresh token',
    page.requested() === FINDER.token && page.newWindow.join() === fresh(REPORT).token, page.state());
  check(results, '... the editor shows the Finder\'s file, which the backend saves to',
    page.loaded.join() === FINDER.name && page.saveTarget === FINDER.name, page.state());
  check(results, '... and no message', page.messages.length === 0, page.state());
  page.bridge.stopTimers();
}

{
  const page = await clickDuringFinderOpen(NOTES, [fresh(NOTES)]);
  check(results, 'Markdown recent during an open: the page does not leave for the Markdown page',
    page.waited && page.bridge.sandbox.location.href === '', page.state());
  check(results, '... it opens in a new window once the Finder\'s file is open',
    page.newWindow.join() === fresh(NOTES).token && page.loaded.join() === FINDER.name, page.state());
  page.bridge.stopTimers();
}

{
  const page = await clickDuringFinderOpen(REPORT, []);
  check(results, 'recent no longer listed, during an open: nothing opens for it',
    page.waited && page.requested() === FINDER.token && page.newWindow.length === 0, page.state());
  check(results, '... and it says the file may have been moved or deleted',
    page.messages.join() === page.t('openFailedMsg'), page.state());
  page.bridge.stopTimers();
}

{
  const page = await loadPage({ listed: [fresh(REPORT)] });
  page.click(REPORT);
  page.answerRecent();
  await sleep(20);
  page.answer(fresh(REPORT));
  await sleep(AFTER_START_OPEN);
  check(results, 'recent with nothing else opening: opens here with its fresh token',
    page.requested() === fresh(REPORT).token && page.newWindow.length === 0
      && page.loaded.join() === REPORT.name && page.messages.length === 0, page.state());
  page.bridge.stopTimers();
}

{
  const page = await loadPage({ listed: [] });
  page.click(REPORT);
  page.answerRecent();
  await sleep(20);
  check(results, 'recent no longer listed, nothing else opening: the moved or deleted message',
    page.requested() === '' && page.newWindow.length === 0 && page.messages.join() === page.t('openFailedMsg'),
    page.state());
  page.bridge.stopTimers();
}

{
  const page = await loadPage({ listed: [fresh(NOTES)] });
  page.click(NOTES);
  page.answerRecent();
  await sleep(20);
  check(results, 'Markdown recent with nothing else opening: goes to the Markdown page',
    page.bridge.sandbox.location.href === 'markdown/markdown.html?token=' + fresh(NOTES).token, page.state());
  page.bridge.stopTimers();
}

finish(results);
