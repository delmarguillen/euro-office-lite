// Checks that .md and .markdown go to the Markdown page and nothing else does,
// that the three lists of Markdown extensions agree, and that the page's text
// helpers (src/markdown/markdown-text.js) give back the file's own bytes and
// refuse image paths outside the document's folder.
//
// Run: node scripts/test-bridge-markdown.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

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
      fonts_status: () => 'ready',
    },
  });
  bridge.sandbox.location = { href: 'index.html', reload() { this.reloaded = true; } };
  bridge.sandbox._openEditor = (docType) => commands.push({ name: 'openEditor', args: docType });
  return { bridge, commands };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 20));

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
  const pending = bridge.sandbox.localStorage.getItem('eo-markdown-open');
  check(results, 'New Markdown goes to the Markdown page', bridge.sandbox.location.href === 'markdown/markdown.html');
  check(results, 'New Markdown leaves a new document for the page', pending === JSON.stringify({ new: true }), String(pending));
  const html = await readFile(path.join(projectRoot, 'src', 'index.html'), 'utf8');
  const patches = await readFile(path.join(projectRoot, 'src', 'editor-patches.js'), 'utf8');
  check(results, 'the start screen has a New Markdown button next to the others', /data-type="slide"[\s\S]*?data-new-markdown/.test(html));
  check(results, 'the button calls _eoNewMarkdown', /data-new-markdown[\s\S]{0,120}_eoNewMarkdown\(\)/.test(patches));
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
  await bridge.run(`_eoOpenGranted({ token: 'tok', name: ${JSON.stringify(name)} })`);
  const pending = JSON.parse(bridge.sandbox.localStorage.getItem('eo-markdown-open') || 'null');
  check(results, `opening ${name} loads the Markdown page`,
    bridge.sandbox.location.href === 'markdown/markdown.html');
  check(results, `opening ${name} hands over its token`, pending && pending.token === 'tok' && pending.name === name);
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
    bridge.sandbox.location.href === 'index.html' && bridge.sandbox.localStorage.getItem('eo-markdown-open') === null);
  bridge.stopTimers();
}

// File > Open from an editor: the editor is dropped the usual way first.
{
  const { bridge, commands } = await routingBridge();
  bridge.run(`window.AscDesktopEditor._currentDocType = 'word'`);
  await bridge.run(`window.AscDesktopEditor.LocalFileOpen({ token: 'tok', name: 'notes.md' })`);
  await settle();
  check(results, 'from an open editor, a .md loads the Markdown page',
    bridge.sandbox.location.href === 'markdown/markdown.html' && !bridge.sandbox.location.reloaded);
  check(results, 'from an open editor, the session ends as on any switch',
    commands.some((command) => command.name === 'recovery_end') &&
    commands.some((command) => command.name === 'set_document_modified'));
  check(results, 'from an open editor, the .md is not queued for an editor',
    bridge.sandbox.localStorage.getItem('eo-pending-open') === null);
  bridge.stopTimers();
}

{
  const { bridge } = await routingBridge();
  bridge.run(`window.AscDesktopEditor._currentDocType = 'word'`);
  await bridge.run(`window.AscDesktopEditor.LocalFileOpen({ token: 'tok', name: 'report.docx' })`);
  await settle();
  check(results, 'from an open editor, a .docx reloads for the editor as before',
    bridge.sandbox.location.reloaded === true && bridge.sandbox.location.href === 'index.html');
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
