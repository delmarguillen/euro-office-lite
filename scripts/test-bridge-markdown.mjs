// Checks that .md and .markdown go to the Markdown page and nothing else does,
// that the three lists of Markdown extensions agree, and that the page's text
// helpers (src/markdown/markdown-text.js) give back the file's own bytes and
// refuse image paths outside the document's folder.
//
// Run: node scripts/test-bridge-markdown.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { check, finish, loadBridge, pendingQueue, projectRoot } from './bridge-test-harness.mjs';

const results = [];

async function loadMarkdownText() {
  const sandbox = { TextDecoder, TextEncoder, decodeURIComponent };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  const file = path.join(projectRoot, 'src', 'markdown', 'markdown-text.js');
  vm.runInContext(await readFile(file, 'utf8'), sandbox, { filename: file });
  return sandbox.EoMarkdownText;
}

// A bridge whose invoke records the commands, with a location to navigate.
async function routingBridge() {
  const commands = [];
  const record = (name, answer) => (args) => { commands.push({ name, args }); return answer; };
  const bridge = await loadBridge({
    invoke: {
      open_file: record('open_file', 'BASE64'),
      open_pdf: record('open_pdf', 'BASE64'),
      recovery_end: record('recovery_end', null),
      set_document_modified: record('set_document_modified', null),
      open_in_new_window: record('open_in_new_window', 'opened'),
      fonts_status: () => 'ready',
    },
  });
  bridge.sandbox.location = { href: 'index.html', reload() { this.reloaded = true; } };
  bridge.sandbox._openEditor = (docType) => commands.push({ name: 'openEditor', args: docType });
  return { bridge, commands };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));

// Where the bridge sent the page: the page itself and what its address hands
// over (bridge.js, _eoGoToMarkdownPage).
function markdownTarget(bridge) {
  const url = new URL(String(bridge.sandbox.location.href), 'http://app/');
  return { page: url.pathname.slice(1), params: Object.fromEntries(url.searchParams) };
}
const LEGACY_KEY = 'eo-markdown-open';

// A bridge without the function under test (the base, for instance) fails
// the check instead of stopping the script.
function tryRun(bridge, code) {
  try {
    return bridge.run(code);
  } catch (error) {
    return 'error: ' + error.message;
  }
}

// ── New Markdown ──
{
  const { bridge } = await routingBridge();
  tryRun(bridge, '_eoNewMarkdown()');
  const target = markdownTarget(bridge);
  check(results, 'New Markdown goes to the Markdown page', target.page === 'markdown/markdown.html');
  check(results, 'New Markdown asks for a new document in the address', JSON.stringify(target.params) === JSON.stringify({ new: '1' }),
    String(bridge.sandbox.location.href));
  check(results, 'New Markdown writes nothing to localStorage', bridge.sandbox.localStorage.getItem(LEGACY_KEY) === null);
  const html = await readFile(path.join(projectRoot, 'src', 'index.html'), 'utf8');
  const patches = await readFile(path.join(projectRoot, 'src', 'editor-patches.js'), 'utf8');
  check(results, 'the start screen has a New Markdown button next to the others', /data-type="slide"[\s\S]*?data-new-markdown/.test(html));
  check(results, 'the button calls _eoNewMarkdown', /data-new-markdown[\s\S]{0,120}_eoNewMarkdown\(\)/.test(patches));
}

// Nothing reads or writes the old localStorage hand-over any more.
{
  const sources = await Promise.all(['bridge.js', 'editor-patches.js', 'markdown/markdown.js']
    .map((name) => readFile(path.join(projectRoot, 'src', ...name.split('/')), 'utf8')));
  check(results, 'no page uses the eo-markdown-open entry', sources.every((source) => !source.includes(LEGACY_KEY)));
}

// ── Recovering a Markdown document ──
{
  const { bridge } = await routingBridge();
  tryRun(bridge, "_eoRecoverMarkdown('1700000000000-42')");
  const target = markdownTarget(bridge);
  check(results, 'Recover sends a Markdown document to its page', target.page === 'markdown/markdown.html');
  check(results, 'Recover puts the folder id in the address', JSON.stringify(target.params) === JSON.stringify({ recover: '1700000000000-42' }),
    String(bridge.sandbox.location.href));
  check(results, 'Recover writes nothing to localStorage', bridge.sandbox.localStorage.getItem(LEGACY_KEY) === null);
  const patches = await readFile(path.join(projectRoot, 'src', 'editor-patches.js'), 'utf8');
  check(results, 'the start screen routes a Markdown candidate to _eoRecoverMarkdown',
    /candidate\.docType === 'markdown'[\s\S]{0,80}_eoRecoverMarkdown\(candidate\.id\)/.test(patches));
}

// ── Routing ──
{
  const { bridge } = await routingBridge();
  for (const name of ['notes.md', 'README.MD', 'Notes.Markdown', '/home/u/a.b/c.md']) {
    check(results, `${name} is Markdown`, tryRun(bridge, `_eoIsMarkdownPath(${JSON.stringify(name)})`) === true);
  }
  for (const name of ['report.docx', 'md', 'notes.mdx', 'notes.md.docx', '/home/u/a.md/report.txt', '']) {
    check(results, `${name || '(empty)'} is not Markdown`, tryRun(bridge, `_eoIsMarkdownPath(${JSON.stringify(name)})`) === false);
  }
  bridge.stopTimers();
}

for (const name of ['notes.md', 'NOTES.MD', 'notes.markdown']) {
  const { bridge, commands } = await routingBridge();
  await bridge.run(`_eoOpenGranted({ token: 'tok+/=&x', name: ${JSON.stringify(name)} })`);
  const target = markdownTarget(bridge);
  check(results, `opening ${name} loads the Markdown page`, target.page === 'markdown/markdown.html');
  check(results, `opening ${name} hands over its token, and only it, in the address`,
    JSON.stringify(target.params) === JSON.stringify({ token: 'tok+/=&x' }), String(bridge.sandbox.location.href));
  check(results, `opening ${name} writes nothing to localStorage`, bridge.sandbox.localStorage.getItem(LEGACY_KEY) === null);
  check(results, `opening ${name} converts nothing`, !commands.some((command) => /^open_/.test(command.name)));
  bridge.stopTimers();
}

{
  const { bridge, commands } = await routingBridge();
  await bridge.run(`_eoOpenGranted({ token: 'tok', name: 'report.docx' })`);
  await settle();
  check(results, '.docx still opens in the editor through open_file',
    commands.some((command) => command.name === 'open_file') &&
    commands.some((command) => command.name === 'openEditor' && command.args === 'word'));
  check(results, '.docx does not load the Markdown page',
    bridge.sandbox.location.href === 'index.html' && bridge.sandbox.localStorage.getItem(LEGACY_KEY) === null);
  bridge.stopTimers();
}

// A .md given at launch (a new window, a double click on the file) opens on
// the Markdown page.
{
  const launch = { token: 'tok-launch', name: 'notes.md' };
  const bridge = await loadBridge({ invoke: { take_pending_open_file: pendingQueue(launch), fonts_status: () => 'ready' } });
  bridge.sandbox.location = { href: 'index.html', reload() { this.reloaded = true; } };
  // With no editor open, LocalFileOpen hands the file to _openPath in
  // editor-patches.js, which this harness does not load. Its Markdown routing
  // is checked below ('the start screen routes Markdown before converting').
  bridge.run(`window._eoOpenPath = async function(file) {
    if (window._eoIsMarkdownPath(file.name)) window._eoOpenMarkdown(file);
  }`);
  bridge.run('_eoTakePendingOpenFile()');
  await settle();
  const target = markdownTarget(bridge);
  check(results, 'a .md given at launch loads the Markdown page',
    target.page === 'markdown/markdown.html' && target.params.token === 'tok-launch');
  bridge.stopTimers();
}

// File > Open from an editor: every file, a .md too, goes to a new window
// and the open document stays as it is.
for (const name of ['notes.md', 'report.docx']) {
  const { bridge, commands } = await routingBridge();
  bridge.run(`window.AscDesktopEditor._currentDocType = 'word'`);
  await bridge.run(`window.AscDesktopEditor.LocalFileOpen({ token: 'tok', name: '${name}' })`);
  await settle();
  const opened = commands.filter((command) => command.name === 'open_in_new_window');
  check(results, `from an open editor, ${name} goes to open_in_new_window`,
    opened.length === 1 && opened[0].args.token === 'tok');
  check(results, `from an open editor, ${name} leaves this page as it is`,
    bridge.sandbox.location.href === 'index.html' && !bridge.sandbox.location.reloaded);
  check(results, `from an open editor, ${name} does not end the session`,
    !commands.some((command) => command.name === 'recovery_end'));
  check(results, `from an open editor, ${name} is queued for nothing`,
    bridge.sandbox.localStorage.getItem('eo-pending-open') === null &&
    bridge.sandbox.localStorage.getItem(LEGACY_KEY) === null);
  bridge.stopTimers();
}

// The start screen's Open and Recents go through _openPath in editor-patches.js.
{
  const source = await readFile(path.join(projectRoot, 'src', 'editor-patches.js'), 'utf8');
  const start = source.indexOf('async function _openPath(file, silent) {');
  const body = source.slice(start, source.indexOf('\n    }\n', start));
  check(results, 'the start screen routes Markdown before converting',
    start >= 0 && body.indexOf('_eoOpenMarkdown(file)') !== -1 &&
    body.indexOf('_eoOpenMarkdown(file)') < body.indexOf('_eoOpenCommandForPath'));
}

// ── The extension lists agree ──
const Text = await loadMarkdownText();
{
  const rust = await readFile(path.join(projectRoot, 'src-tauri', 'src', 'markdown.rs'), 'utf8');
  const match = /MARKDOWN_EXTENSIONS: \[&str; \d+\] = \[([^\]]*)\]/.exec(rust);
  const rustList = match ? match[1].split(',').map((ext) => ext.trim().replace(/"/g, '')).filter(Boolean) : [];
  const { bridge } = await routingBridge();
  const bridgeList = tryRun(bridge, 'typeof _EO_MARKDOWN_EXTENSIONS === "undefined" ? [] : _EO_MARKDOWN_EXTENSIONS');
  check(results, 'bridge.js and markdown.rs list the same extensions',
    JSON.stringify([...bridgeList]) === JSON.stringify(rustList), `${bridgeList} / ${rustList}`);
  check(results, 'markdown-text.js lists the same extensions',
    JSON.stringify([...Text.MARKDOWN_EXTENSIONS]) === JSON.stringify(rustList));
  bridge.stopTimers();
}

// ── The page explains the backend's refusal of a file that is not UTF-8 ──
{
  const rust = await readFile(path.join(projectRoot, 'src-tauri', 'src', 'markdown.rs'), 'utf8');
  const page = await readFile(path.join(projectRoot, 'src', 'markdown', 'markdown.js'), 'utf8');
  const rustError = (/NOT_UTF8_ERROR: &str = "([^"]*)"/.exec(rust) || [])[1];
  const pageError = (/NOT_UTF8_ERROR = '([^']*)'/.exec(page) || [])[1];
  check(results, 'markdown.rs and markdown.js name the not-UTF-8 error alike',
    rustError !== undefined && rustError === pageError, `${rustError} / ${pageError}`);
}

// ── Load and save without changes: same bytes ──
// What a <textarea> does to its value: CRLF and lone CR become LF.
const throughTextarea = (text) => text.replace(/\r\n?/g, '\n');
const bytesOf = (...parts) => Buffer.concat(parts.map((part) => (typeof part === 'string' ? Buffer.from(part, 'utf8') : Buffer.from(part))));
const BOM = [0xef, 0xbb, 0xbf];
const files = {
  LF: bytesOf('# Title\n\nSome *text*, ñ.\n'),
  CRLF: bytesOf('# Title\r\n\r\nSome *text*, ñ.\r\n'),
  'BOM and LF': bytesOf(BOM, '# Title\n\ntext'),
  'BOM and CRLF': bytesOf(BOM, '# Title\r\n\r\ntext\r\n'),
  'empty file': bytesOf(''),
  'BOM only': bytesOf(BOM),
};
for (const [label, bytes] of Object.entries(files)) {
  const decoded = Text.decode(new Uint8Array(bytes));
  const saved = Buffer.from(Text.encode(throughTextarea(decoded.text), decoded));
  check(results, `${label}: loaded and saved untouched, the bytes are the same`, saved.equals(bytes));
}
{
  const decoded = Text.decode(new Uint8Array(files['BOM and CRLF']));
  const edited = throughTextarea(decoded.text) + 'new line\n';
  const saved = Buffer.from(Text.encode(edited, decoded)).toString('utf8');
  check(results, 'an edit keeps the BOM and writes CRLF', saved === '\uFEFF# Title\r\n\r\ntext\r\nnew line\r\n');
  check(results, 'the textarea never sees a CR', !decoded.text.includes('\r'));
}
{
  let code = null;
  try { Text.decode(new Uint8Array([0x63, 0x61, 0x66, 0xe9, 0x0a])); } catch (error) { code = error.code; }
  check(results, 'a file that is not UTF-8 is refused, not changed', code === 'not-utf8');
}
{
  // UTF-16 LE without a BOM, "A\r\n": every byte is valid UTF-8, NULs included.
  let code = null;
  try { Text.decodeFile(new Uint8Array([0x41, 0x00, 0x0d, 0x00, 0x0a, 0x00])); } catch (error) { code = error.code; }
  check(results, 'a file with a NUL (UTF-16 without a BOM) is refused, not changed', code === 'not-utf8');
}
{
  // A recovery copy is what encode wrote: a NUL pasted into the text comes back.
  const draft = '# Draft\n\0pasted\n';
  const bytes = Text.encode(draft, { bom: true, eol: '\r\n' });
  let decoded = null;
  try { decoded = Text.decode(bytes); } catch (error) {}
  check(results, 'a recovery copy with a NUL decodes to its text', decoded !== null && decoded.text === draft);
  const utf8 = Text.decodeFile(new Uint8Array([0x63, 0x61, 0x66, 0xc3, 0xa9, 0x0a]));
  check(results, 'a UTF-8 file without a NUL still opens', utf8.text === 'café\n');
}

// ── Mixed and CR-only line ends: opened and saved untouched, the same bytes ──
// The textarea and CodeMirror only hold LF, so no single line end gives these
// files back.
{
  const unusual = {
    'CRLF and LF mixed': bytesOf('a\r\nb\nc\r\n'),
    'CR only': bytesOf('a\rb\rc\r'),
    'CRLF, CR and LF mixed': bytesOf('# T\r\none\rtwo\nthree\r\n'),
    'BOM, CRLF and LF mixed': bytesOf(BOM, 'a\r\nb\nc\r\n'),
  };
  for (const [label, bytes] of Object.entries(unusual)) {
    const decoded = Text.decodeFile(new Uint8Array(bytes));
    const saved = Buffer.from(Text.encode(throughTextarea(decoded.text), decoded));
    check(results, `${label}: opened and saved untouched, the bytes are the same`, saved.equals(bytes),
      'saved ' + JSON.stringify(saved.toString('utf8')));
  }
}
{
  // Edited, a file is saved with the line end most of its lines use.
  const edit = (bytes) => {
    const decoded = Text.decodeFile(new Uint8Array(bytes));
    return Buffer.from(Text.encode(decoded.text + 'Z\n', decoded)).toString('utf8');
  };
  const crOnly = edit(bytesOf('a\rb\r'));
  check(results, 'an edited CR-only file keeps CR', crOnly === 'a\rb\rZ\r', JSON.stringify(crOnly));
  const mostlyCrlf = edit(bytesOf('a\r\nb\r\nc\n'));
  check(results, 'an edited mixed file takes the line end most lines use', mostlyCrlf === 'a\r\nb\r\nc\r\nZ\r\n', JSON.stringify(mostlyCrlf));
  const tie = edit(bytesOf('a\rb\n'));
  check(results, 'an edited file with a tie gets LF', tie === 'a\nb\nZ\n', JSON.stringify(tie));
}

// ── Saving while the text changes ──
// A save that takes a while, and an undo back to the saved text before it
// ends: the file is still being written, so the document counts as unsaved
// until then, and Close and Quit wait for the write.
{
  const api = ['savedState', 'afterSaveEvent', 'unsavedChanges', 'closeAction'];
  const missing = api.filter((name) => typeof Text[name] !== 'function');
  check(results, 'markdown-text.js has the save state helpers', missing.length === 0,
    missing.length ? 'missing: ' + missing.join(', ') : '');
  if (missing.length === 0) {
    const flags = (state, text) => JSON.stringify(Text.unsavedChanges(state, text));
    const unsaved = (modified, savePending) => JSON.stringify({ modified, savePending });
    const writeStart = (state, text) => Text.afterSaveEvent(state, { type: 'write-start', text });
    const writeEnd = (state, ok) => Text.afterSaveEvent(state, { type: 'write-end', ok });

    // Opened A, edited to B, Save writes B; undo back to A while it writes.
    let state = Text.savedState('A');
    check(results, 'an edit is a change', flags(state, 'B') === unsaved(true, false), flags(state, 'B'));
    state = writeStart(state, 'B');
    check(results, 'undoing to the saved text during a save leaves the save pending',
      flags(state, 'A') === unsaved(false, true), flags(state, 'A'));
    check(results, 'Close and Quit wait for the save that is writing',
      Text.closeAction(state, 'A') === 'wait', Text.closeAction(state, 'A'));
    state = writeEnd(state, true);
    check(results, 'once B is written, the undone text A is a change',
      flags(state, 'A') === unsaved(true, false), flags(state, 'A'));
    check(results, 'and Close asks before discarding it', Text.closeAction(state, 'A') === 'ask', Text.closeAction(state, 'A'));

    // The same, but the write fails: the file still holds A.
    state = writeEnd(writeStart(Text.savedState('A'), 'B'), false);
    check(results, 'after a failed write the saved text is still the file\'s',
      flags(state, 'A') === unsaved(false, false), flags(state, 'A'));
    check(results, 'and Close closes without asking', Text.closeAction(state, 'A') === 'close', Text.closeAction(state, 'A'));

    // Typing on while the write runs.
    state = writeStart(Text.savedState('A'), 'B');
    check(results, 'typing during a save is a change, and the save is pending',
      flags(state, 'BC') === unsaved(true, true), flags(state, 'BC'));
    state = writeEnd(state, true);
    check(results, 'what was typed during the save is still a change',
      flags(state, 'BC') === unsaved(true, false), flags(state, 'BC'));

    // A save of the text on screen, and recovered text that was never saved.
    state = writeEnd(writeStart(Text.savedState('A'), 'B'), true);
    check(results, 'a finished save of the text on screen leaves nothing unsaved',
      flags(state, 'B') === unsaved(false, false), flags(state, 'B'));
    const recovered = Text.savedState(null);
    check(results, 'recovered text is a change until it is saved',
      flags(recovered, 'A') === unsaved(true, false), flags(recovered, 'A'));
  }
}

// ── Image references ──
{
  const local = (src) => {
    const found = Text.classifyImageSource(src);
    return found.kind === 'local' ? found.path : found.kind;
  };
  const cases = [
    ['img/a.png', 'img/a.png'],
    ['./img/a.png', 'img/a.png'],
    ['img/a%20b.png', 'img/a b.png'],
    ['img/../a.png?x=1#y', 'a.png'],
    ['img\\a.png', 'img/a.png'],
    ['https://example.com/a.png', 'remote'],
    ['HTTP://example.com/a.png', 'remote'],
    ['//example.com/a.png', 'remote'],
    ['../secret.png', 'refused'],
    ['img/../../secret.png', 'refused'],
    ['%2e%2e/secret.png', 'refused'],
    ['..\\secret.png', 'refused'],
    ['/etc/passwd', 'refused'],
    ['\\\\server\\share\\a.png', 'refused'],
    ['C:/Windows/a.png', 'refused'],
    ['file:///etc/passwd', 'refused'],
    ['javascript:alert(1)', 'refused'],
    ['data:image/png;base64,AAAA', 'refused'],
    ['', 'refused'],
    ['%zz.png', 'refused'],
  ];
  for (const [src, expected] of cases) {
    const got = local(src);
    check(results, `image ${src || '(empty)'} -> ${expected}`, got === expected, got);
  }
}

finish(results);
