// Opens src/markdown/markdown.html in headless Chromium with a fake Tauri
// bridge and checks what a webview would do with it: the preview is
// sanitized, remote images and paths outside the folder are not loaded, links
// do not navigate, and saving untouched text gives back the file's bytes.
//
// Needs Playwright and its Chromium (not installed by this repository), so
// it is not one of the scripts/test-bridge-*.mjs checks CI runs.
//
// Run: node scripts/test-markdown-page.mjs
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { check, finish, projectRoot } from './bridge-test-harness.mjs';

// Playwright is found through NODE_PATH, which only require() follows.
const { chromium } = createRequire(import.meta.url)('playwright');

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' };

function serveSource() {
  const root = path.join(projectRoot, 'src');
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://localhost');
    if (url.pathname === '/index.html') {
      response.writeHead(200, { 'content-type': 'text/html' });
      response.end('<!DOCTYPE html><title>start</title>');
      return;
    }
    const file = path.join(root, path.normalize(decodeURIComponent(url.pathname)));
    try {
      const body = await readFile(file);
      response.writeHead(200, { 'content-type': TYPES[path.extname(file)] || 'application/octet-stream' });
      response.end(body);
    } catch {
      response.writeHead(404);
      response.end();
    }
  });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server)));
}

// Replaces window.__TAURI__ before the page's scripts run. `fake` says what
// the backend answers; every call is recorded in window.__calls.
function installFakeTauri(fake) {
  // Kept in sessionStorage too: an error sends the page back to index.html.
  window.__calls = JSON.parse(sessionStorage.getItem('calls') || '[]');
  const record = (entry) => {
    window.__calls.push(entry);
    sessionStorage.setItem('calls', JSON.stringify(window.__calls));
  };
  window.__listeners = {};
  localStorage.setItem('eo-ui-lang', fake.lang || 'en');
  if (!sessionStorage.getItem('opened')) {
    sessionStorage.setItem('opened', '1');
    let pending = { token: 'tok-open', name: fake.name };
    if (fake.isNew) pending = { new: true };
    if (fake.recoverId) pending = { recover: fake.recoverId };
    localStorage.setItem('eo-markdown-open', JSON.stringify(pending));
  }
  // The file on disk and the recovery copy outlive a reload, as in the backend.
  const stored = (key) => sessionStorage.getItem(key);
  const handlers = {
    read_markdown: (args) => {
      if (args.token !== 'tok-open') throw new Error('Unknown or expired file token');
      if (fake.readError) throw fake.readError;
      // Opening a file ends the recovery session (start_markdown_document).
      sessionStorage.removeItem('copy');
      return { data: stored('disk') || fake.data, name: fake.name };
    },
    save_markdown: (args) => {
      if (fake.saveError) throw fake.saveError;
      sessionStorage.setItem('disk', args.data);
      sessionStorage.removeItem('copy');
      return args.token ? 'copy.md' : fake.name;
    },
    markdown_recovery_write: (args) => { sessionStorage.setItem('copy', args.data); },
    markdown_recovery_reload: () => (stored('copy')
      ? { data: stored('copy'), name: fake.name, hasFile: !fake.isNew && !fake.recoverId }
      : null),
    recovery_end: () => { sessionStorage.removeItem('copy'); },
    pick_save_target: (args) => (args.requestedExt === 'html'
      ? { token: 'tok-html', name: 'notes.html' }
      : { token: 'tok-save', name: 'copy.md' }),
    export_markdown_html: () => 'notes.html',
    markdown_recovery_load: (args) => {
      if (fake.recoverError || args.id !== fake.recoverId) throw new Error(fake.recoverError || 'No Markdown recovery session');
      return { data: fake.data, name: fake.name };
    },
    markdown_image: (args) => {
      const image = (fake.images || {})[args.src];
      if (!image) throw new Error('the image is outside the document\'s folder');
      return image;
    },
    current_document: () => (fake.isNew || fake.recoverId ? null : { token: 'tok-open', name: fake.name }),
    pick_document: () => window.__pickDocument || null,
  };
  window.__TAURI__ = {
    core: {
      invoke: (command, args) => {
        record({ command, args: args || {} });
        try {
          return Promise.resolve(handlers[command] ? handlers[command](args || {}) : null);
        } catch (error) {
          return Promise.reject(error.message || error);
        }
      },
    },
    event: {
      listen: (name, handler) => {
        (window.__listeners[name] = window.__listeners[name] || []).push(handler);
        return Promise.resolve(() => {});
      },
    },
    dialog: {
      message: (text, options) => {
        record({ command: 'dialog.message', args: { text, options } });
        return Promise.resolve();
      },
      confirm: (text) => {
        record({ command: 'dialog.confirm', args: { text } });
        return Promise.resolve(!!window.__confirmAnswer);
      },
    },
  };
}

const results = [];
const server = await serveSource();
const base = `http://127.0.0.1:${server.address().port}/markdown/markdown.html`;
const browser = await chromium.launch();

// A fresh page with `fake` as the backend. Requests leaving 127.0.0.1, every
// script requested and CSP reports are collected. `options.blockCodeMirror`
// answers 404 for the editor's bundle; `options.noAdoptedStyles` plays an
// older webview without adoptedStyleSheets.
async function openPage(fake, options = {}) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const outside = [];
  const csp = [];
  const scripts = [];
  if (options.blockCodeMirror) {
    await page.route('**/codemirror.min.js', (route) => route.fulfill({ status: 404, body: '' }));
  }
  if (options.noAdoptedStyles) {
    await page.addInitScript(() => { delete ShadowRoot.prototype.adoptedStyleSheets; });
  }
  page.on('request', (request) => {
    if (request.resourceType() === 'script') scripts.push(request.url());
    if (!request.url().startsWith('http://127.0.0.1') && !request.url().startsWith('data:')) {
      outside.push(request.url());
    }
  });
  page.on('console', (message) => {
    if (/Content Security Policy/i.test(message.text())) csp.push(message.text());
  });
  await page.addInitScript(installFakeTauri, fake);
  await page.goto(base);
  let first = 'read_markdown';
  if (fake.isNew) first = 'new_markdown';
  if (fake.recoverId) first = 'markdown_recovery_load';
  await page.waitForFunction((command) => window.__calls.some((call) => call.command === command), first);
  await page.waitForTimeout(400);
  const calls = () => page.evaluate(() => window.__calls);
  const preview = () => page.evaluate(() => document.getElementById('md-preview').contentDocument.body.innerHTML);
  return { context, page, outside, csp, scripts, calls, preview };
}

const b64 = (text) => Buffer.from(text, 'binary').toString('base64');
const utf8 = (text) => Buffer.from(text, 'utf8').toString('base64');

// ── Sanitizing and links ──
{
  const malicious = [
    '# Title',
    '<script>window.parent.__pwned = "script"</script>',
    '<img src="x.png" onerror="window.parent.__pwned = \'onerror\'">',
    '[click](javascript:window.parent.__pwned=1)',
    '<iframe src="https://example.com/frame"></iframe>',
    '<svg onload="window.parent.__pwned = \'svg\'"><a xlink:href="https://example.com/svg"><text>s</text></a></svg>',
    '<a href="https://example.com/page" target="_top">out</a> and a note[^1]',
    '<form action="https://example.com/f"><button>go</button></form>',
    '<p style="background:url(https://example.com/bg.png)">styled</p>',
    '[^1]: The footnote.',
    '- [x] done\n- [ ] todo',
    '| a | b |\n|:-:|--:|\n| 1 | 2 |',
  ].join('\n\n');
  const { context, page, outside, csp, preview, calls } = await openPage({ name: 'evil.md', data: utf8(malicious) });
  const html = await preview();
  const pwned = await page.evaluate(() => window.__pwned);
  check(results, 'no script from the document runs', pwned === undefined, String(pwned));
  for (const banned of ['<script', 'onerror', 'onload', '<iframe', '<form', 'style=', 'xlink:href']) {
    check(results, `the preview has no ${banned}`, !html.includes(banned));
  }
  // markdown-it leaves a javascript: link as plain text; no attribute holds one.
  const scriptUrls = await page.evaluate(() => [...document.getElementById('md-preview').contentDocument.body.querySelectorAll('*')]
    .flatMap((node) => [...node.attributes].map((attr) => attr.value))
    .filter((value) => /^\s*javascript:/i.test(value)));
  check(results, 'no attribute holds a javascript: address', scriptUrls.length === 0, scriptUrls.join(' '));
  const hrefs = await page.evaluate(() => [...document.getElementById('md-preview').contentDocument.body.querySelectorAll('[href]')]
    .map((node) => node.getAttribute('href')));
  check(results, 'only links inside the document keep an address', hrefs.length > 0 && hrefs.every((href) => href.startsWith('#')), hrefs.join(' '));
  check(results, 'a link shows its address as a tooltip', html.includes('title="https://example.com/page"'));
  check(results, 'task lists render as checkboxes', html.includes('task-list-item-checkbox'));
  check(results, 'footnotes render', html.includes('footnote'));
  check(results, 'table alignment survives as a class', html.includes('eo-align-center') && html.includes('eo-align-right'));
  check(results, 'nothing is requested outside the app', outside.length === 0, outside.join(' '));
  // DOMPurify parses the document before it drops style attributes, and
  // Chromium reports each one it refuses; anything else would be a real miss.
  const otherCsp = csp.filter((text) => !/Applying inline style/.test(text));
  check(results, 'the page breaks no other CSP rule', otherCsp.length === 0, otherCsp.join(' | '));
  const imageCalls = (await calls()).filter((call) => call.command === 'markdown_image');
  check(results, 'a local image is asked for by its relative path', imageCalls.some((call) => call.args.src === 'x.png'));

  // Clicking a link leaves the window where it is.
  const frame = page.frames().find((candidate) => candidate !== page.mainFrame());
  await frame.click('text=out');
  await page.waitForTimeout(200);
  check(results, 'clicking a link navigates nothing', page.url() === base && frame.url() === 'about:srcdoc', frame.url());
  await context.close();
}

// ── Images ──
{
  const text = [
    '![remote](https://example.com/a.png)',
    '![protocol](//example.com/b.png)',
    '![local](img/a%20b.png)',
    '![up](../secret.png)',
    '![deep](img/../../secret.png)',
    '![absolute](/etc/secret.png)',
    '![drive](C:/secret.png)',
    '![file](file:///etc/secret.png)',
    '![missing](img/missing.png)',
  ].join('\n\n');
  const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=';
  const { context, page, outside, preview, calls } = await openPage({
    name: 'images.md',
    data: utf8(text),
    images: { 'img/a b.png': { mime: 'image/png', data: png } },
  });
  await page.waitForTimeout(500);
  const html = await preview();
  const asked = (await calls()).filter((call) => call.command === 'markdown_image').map((call) => call.args.src);
  check(results, 'remote images are not requested', outside.length === 0 && !html.includes('example.com'), outside.join(' '));
  check(results, 'remote images leave a placeholder', (html.match(/Remote image not loaded/g) || []).length === 2);
  check(results, 'a local image is shown as a data: URL', html.includes('src="data:image/png;base64,' + png));
  check(results, 'paths that leave the folder are never asked for', asked.every((src) => src === 'img/a b.png' || src === 'img/missing.png'), asked.join(' '));
  // markdown-it itself leaves file: as text, so five images reach the check.
  check(results, 'refused and missing images leave a placeholder', (html.match(/Image not shown/g) || []).length === 5);
  check(results, 'a file: image is not an image at all', !/<img[^>]*file:/.test(html));
  await context.close();
}

// ── Saving untouched text gives back the same bytes ──
for (const [label, raw] of [
  ['LF', '# Title\n\nSome *text*.\n\n- a\n- b\n'],
  ['CRLF', '# Title\r\n\r\nSome *text*.\r\n\r\n- a\r\n- b\r\n'],
  ['BOM and CRLF', '\uFEFF# Title\r\n\r\nná\r\n'],
  ['BOM and LF, no final line end', '\uFEFF# Title\n\nñ'],
]) {
  const original = utf8(raw);
  const { context, page, calls } = await openPage({ name: 'notes.md', data: original });
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  const saved = (await calls()).filter((call) => call.command === 'save_markdown');
  check(results, `${label}: Ctrl+S without changes writes the same bytes`,
    saved.length === 1 && saved[0].args.data === original && saved[0].args.token === null);
  await context.close();
}

// ── Editing ──
{
  const { context, page, calls, preview } = await openPage({ name: 'notes.md', data: b64('# One\r\n') });
  const hidden = () => page.evaluate(() => document.getElementById('md-editor').hidden);
  check(results, 'the page opens on the preview alone', await hidden());
  await page.click('#md-edit');
  await page.waitForSelector('#md-editor .cm-content:focus');
  check(results, 'Edit shows the editor', !(await hidden()));
  await page.keyboard.press('Control+End');
  await page.keyboard.type('\nTwo **bold**');
  await page.waitForTimeout(500);
  check(results, 'the preview follows the text', (await preview()).includes('<strong>bold</strong>'));
  const modified = (await calls()).filter((call) => call.command === 'set_document_modified');
  check(results, 'typing marks the document modified', modified.length === 1 && modified[0].args.modified === true);
  await page.keyboard.press('Control+Shift+S');
  await page.waitForTimeout(200);
  const saved = (await calls()).filter((call) => call.command === 'save_markdown').pop();
  check(results, 'Save As writes to the token the dialog gave', saved && saved.args.token === 'tok-save');
  check(results, 'new lines take the file\'s CRLF',
    saved && Buffer.from(saved.args.data, 'base64').toString('binary') === '# One\r\n\r\nTwo **bold**');
  const afterSave = (await calls()).filter((call) => call.command === 'set_document_modified').pop();
  check(results, 'saving clears the modified mark', afterSave.args.modified === false);
  await page.click('#md-edit');
  check(results, 'a second press goes back to the preview alone', await hidden());
  await context.close();
}

// ── CodeMirror ──
const loadsCodeMirror = (scripts) => scripts.some((url) => url.endsWith('/codemirror.min.js'));

async function startEditing(page) {
  await page.click('#md-edit');
  await page.waitForSelector('#md-editor .cm-content:focus');
}

const savedCalls = async (calls) => (await calls()).filter((call) => call.command === 'save_markdown');

{
  const { context, page, scripts } = await openPage({ name: 'notes.md', data: utf8('# Title\n') });
  const loaded = await page.evaluate(() => typeof window.EoCodeMirror);
  check(results, 'reading a file does not load CodeMirror', !loadsCodeMirror(scripts) && loaded === 'undefined');
  await startEditing(page);
  check(results, 'Edit loads CodeMirror', loadsCodeMirror(scripts));
  await context.close();
}

// Load, change a line, undo, save: the same bytes. Then change it for real:
// only that line differs, in the file's own line end.
for (const [label, raw, eol] of [
  ['LF', '# Title\n\n\tTabbed  \ntrailing   \n', '\n'],
  ['CRLF', '# Title\r\n\r\n\tTabbed  \r\ntrailing   \r\n', '\r\n'],
  ['BOM', '\uFEFF# Título\r\n\r\n\tñ \t\r\nend', '\r\n'],
]) {
  const original = utf8(raw);
  const { context, page, calls } = await openPage({ name: 'notes.md', data: original });
  await startEditing(page);
  await page.keyboard.press('Control+Home');
  await page.keyboard.press('End');
  await page.keyboard.type(' edited');
  await page.keyboard.press('Control+z');
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  let saved = await savedCalls(calls);
  check(results, `CodeMirror ${label}: edit, undo, save gives the same bytes`, saved.length === 1 && saved[0].args.data === original);
  await page.keyboard.type(' edited');
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  saved = await savedCalls(calls);
  const lines = raw.split(eol);
  lines[0] += ' edited';
  const expected = utf8(lines.join(eol));
  check(results, `CodeMirror ${label}: an edit changes only its line, tabs and trailing spaces kept`, saved.length === 2 && saved[1].args.data === expected,
    saved.length === 2 ? JSON.stringify(Buffer.from(saved[1].args.data, 'base64').toString('utf8')) : '');
  await context.close();
}

{
  const text = '# Title\n\n<img src="x.png" onerror="window.__pwned = 1">\n\n<script>window.__pwned = 2</script>\n\n<b>bold</b>\n';
  const { context, page, csp } = await openPage({ name: 'notes.md', data: utf8(text) });
  await page.evaluate(() => {
    window.__violations = [];
    document.addEventListener('securitypolicyviolation', (e) => window.__violations.push(e.violatedDirective + ' ' + e.sourceFile));
  });
  await startEditing(page);
  await page.keyboard.type('x');
  await page.waitForTimeout(400);
  // CodeMirror lives in #md-editor's shadow root (entry.mjs, shadowFor).
  const editorDom = await page.evaluate(() => {
    const shadow = document.getElementById('md-editor').shadowRoot;
    return {
      elements: shadow.querySelectorAll('.cm-content img, .cm-content script, .cm-content b').length,
      text: shadow.querySelector('.cm-content').textContent,
      pwned: window.__pwned,
      adopted: shadow.adoptedStyleSheets.length,
      styleElements: document.querySelectorAll('style').length + shadow.querySelectorAll('style').length,
      violations: window.__violations,
    };
  });
  check(results, 'CodeMirror shows HTML in the document as text', editorDom.elements === 0 && editorDom.text.includes('<script>') && editorDom.pwned === undefined);
  check(results, 'CodeMirror styles go through adoptedStyleSheets, no <style>', editorDom.adopted > 0 && editorDom.styleElements === 0);
  const styleCsp = csp.filter((line) => !/Applying inline style/.test(line));
  check(results, 'CodeMirror breaks no CSP rule', editorDom.violations.length === 0 && styleCsp.length === 0, editorDom.violations.concat(styleCsp).join(' | '));
  await page.keyboard.press('Control+f');
  await page.waitForSelector('.cm-search input[name=search]');
  check(results, 'Ctrl+F opens search and replace', await page.isVisible('.cm-search input[name=replace]'));
  await context.close();
}

// The app's shortcuts with the focus in CodeMirror.
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('text\n') });
  await startEditing(page);
  await page.keyboard.press('Control+s');
  await page.keyboard.press('Control+Shift+S');
  await page.waitForTimeout(200);
  const saved = await savedCalls(calls);
  check(results, 'Ctrl+S and Ctrl+Shift+S save from CodeMirror', saved.length === 2 && saved[0].args.token === null && saved[1].args.token === 'tok-save');
  await page.focus('#md-editor .cm-content');
  await page.keyboard.press('Control+o');
  await page.waitForTimeout(100);
  const names = (await calls()).map((call) => call.command);
  check(results, 'Ctrl+O opens the file dialog from CodeMirror', names.includes('pick_document'));
  await page.focus('#md-editor .cm-content');
  await page.keyboard.type('x');
  await page.keyboard.press('Control+w');
  await page.waitForTimeout(100);
  check(results, 'Ctrl+W asks before closing a modified document', (await calls()).some((call) => call.command === 'dialog.confirm') && !page.url().endsWith('/index.html'));
  await page.evaluate(() => { window.__confirmAnswer = true; });
  await page.focus('#md-editor .cm-content');
  await page.keyboard.press('Control+w');
  await page.waitForURL('**/index.html');
  check(results, 'Ctrl+W closes to the start screen', page.url().endsWith('/index.html'));
  await context.close();
}

{
  const { context, page } = await openPage({ name: 'notes.md', data: utf8('one\n') });
  await page.evaluate(() => { window.__pickDocument = { token: 'tok-other', name: 'report.docx' }; });
  await page.keyboard.press('Control+o');
  await page.waitForURL('**/index.html');
  const pending = await page.evaluate(() => localStorage.getItem('eo-pending-open'));
  check(results, 'Ctrl+O hands another format to the editors', pending === JSON.stringify({ token: 'tok-other', name: 'report.docx' }));
  await context.close();
}

{
  const { context, page } = await openPage({ name: 'notes.md', data: utf8('one\n') }, { noAdoptedStyles: true });
  await page.click('#md-edit');
  await page.waitForSelector('#md-source:focus');
  check(results, 'without adoptedStyleSheets the textarea is used', await page.evaluate(() => document.getElementById('md-editor').hidden));
  await context.close();
}

// Without CodeMirror, the textarea edits and saves as before.
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('one\r\n') }, { blockCodeMirror: true });
  await page.click('#md-edit');
  await page.waitForSelector('#md-source:focus');
  const editorHidden = await page.evaluate(() => document.getElementById('md-editor').hidden);
  check(results, 'a missing CodeMirror falls back to the textarea', editorHidden);
  await page.keyboard.press('Control+End');
  await page.keyboard.type('two');
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  const saved = await savedCalls(calls);
  check(results, 'the textarea fallback saves', saved.length === 1 && Buffer.from(saved[0].args.data, 'base64').toString() === 'one\r\ntwo');
  await context.close();
}

// ── New Markdown ──
{
  const { context, page, calls, scripts } = await openPage({ isNew: true });
  await page.waitForSelector('#md-editor .cm-content:focus');
  const name = await page.textContent('#md-name');
  check(results, 'New Markdown opens empty in Edit, in CodeMirror', name === 'Untitled.md' && loadsCodeMirror(scripts)
    && (await page.getAttribute('#md-edit', 'aria-pressed')) === 'true');
  check(results, 'New Markdown reads no file', !(await calls()).some((call) => call.command === 'read_markdown'));
  await page.keyboard.type('# New\n\nText');
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  const picked = (await calls()).filter((call) => call.command === 'pick_save_target');
  check(results, 'the first Ctrl+S asks where, offering Untitled.md', picked.length === 1 && picked[0].args.suggestedName === 'Untitled.md');
  let saved = await savedCalls(calls);
  check(results, 'the first save writes LF without BOM to the dialog\'s token', saved.length === 1 && saved[0].args.token === 'tok-save'
    && Buffer.from(saved[0].args.data, 'base64').toString('binary') === '# New\n\nText');
  check(results, 'after the first save the name is the saved file\'s', (await page.textContent('#md-name')) === 'copy.md');
  await page.keyboard.type('!');
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  saved = await savedCalls(calls);
  const pickedAgain = (await calls()).filter((call) => call.command === 'pick_save_target');
  check(results, 'later saves go over the file without a dialog', saved.length === 2 && saved[1].args.token === null && pickedAgain.length === 1);
  await context.close();
}
{
  const { context, page, calls } = await openPage({ isNew: true, lang: 'es' });
  await page.waitForSelector('#md-editor .cm-content:focus');
  await page.keyboard.type('x');
  await page.evaluate(() => window.__listeners['confirm-close'].forEach((handler) => handler({})));
  await page.waitForTimeout(100);
  const confirm = (await calls()).find((call) => call.command === 'dialog.confirm');
  check(results, 'closing a never saved document asks first (Spanish)', confirm && confirm.args.text.includes('cambios sin guardar')
    && !(await calls()).some((call) => call.command === 'force_close'));
  const labels = await page.evaluate(() => [...document.querySelectorAll('.toolbar button')].map((b) => b.textContent));
  check(results, 'New Markdown\'s toolbar is in Spanish', labels.join('|') === 'Cerrar|Vista previa|Guardar|Guardar como…|Exportar como HTML', labels.join('|'));
  await context.close();
}

// ── Export as HTML ──
{
  const text = [
    'Intro before any heading',
    '## The <em>first</em> heading',
    '<script>window.parent.__pwned = 1</script>',
    '<img src="x.png" onerror="window.parent.__pwned = 2">',
    '<a href="javascript:alert(1)">js</a> [md](javascript:alert(2))',
    '<svg onload="alert(3)"></svg>',
    '![local](img/a%20b.png) ![remote](https://example.com/r.png) ![up](../secret.png)',
    '| a |\n|:-:|\n| 1 |',
  ].join('\n\n');
  const { context, page, calls, outside } = await openPage({ name: 'evil.md', data: utf8(text) });
  await page.click('#md-export');
  await page.waitForTimeout(300);
  const picked = (await calls()).filter((call) => call.command === 'pick_save_target').pop();
  check(results, 'Export asks where, offering the document\'s name as .html', picked && picked.args.suggestedName === 'evil.html' && picked.args.requestedExt === 'html');
  const exported = (await calls()).filter((call) => call.command === 'export_markdown_html');
  check(results, 'Export writes to the dialog\'s token', exported.length === 1 && exported[0].args.token === 'tok-html');
  const html = exported.length ? Buffer.from(exported[0].args.data, 'base64').toString('utf8') : '';
  check(results, 'the exported HTML has no <script>', !/<script/i.test(html));
  check(results, 'the exported HTML has no on* attribute', !/\son[a-z]+\s*=/i.test(html), (html.match(/\son[a-z]+\s*=[^>]*/i) || [''])[0]);
  check(results, 'the exported HTML has no javascript: address', !/(href|src)\s*=\s*["']?\s*javascript:/i.test(html));
  check(results, 'the exported HTML starts with a doctype, utf-8 and a CSP for scripts', html.startsWith('<!DOCTYPE html>')
    && html.includes('<meta charset="utf-8">') && html.includes("script-src 'none'"));
  check(results, 'the title is the first heading', html.includes('<title>The first heading</title>'));
  check(results, 'the stylesheet is inside the file', /<style>[\s\S]*max-width: 860px[\s\S]*eo-align-center[\s\S]*<\/style>/.test(html) && !html.includes('<link'));
  check(results, 'a local image keeps its reference as written', html.includes('src="img/a%20b.png"'));
  check(results, 'remote and refused images become placeholders', html.includes('Remote image not loaded: remote') && html.includes('Image not shown: up')
    && !html.includes('example.com/r.png') && !html.includes('secret.png'));
  check(results, 'exporting leaves the document as it was', !(await calls()).some((call) => call.command === 'save_markdown' || call.command === 'set_document_modified'));
  check(results, 'exporting requests nothing outside the app', outside.length === 0, outside.join(' '));
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'plain.md', data: utf8('no heading here\n') });
  await page.click('#md-export');
  await page.waitForTimeout(300);
  const exported = (await calls()).filter((call) => call.command === 'export_markdown_html');
  const html = exported.length ? Buffer.from(exported[0].args.data, 'base64').toString('utf8') : '';
  check(results, 'without a heading the title is the file name', html.includes('<title>plain</title>'));
  await context.close();
}

// ── Crash recovery ──
const RECOVERY_WAIT_MS = 3600;
const commandsNamed = async (calls, name) => (await calls()).filter((call) => call.command === name);

{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('\uFEFF# One\r\n') });
  await page.waitForTimeout(RECOVERY_WAIT_MS);
  check(results, 'an unmodified document is not copied', (await commandsNamed(calls, 'markdown_recovery_write')).length === 0);
  await startEditing(page);
  await page.keyboard.press('Control+End');
  for (const key of 'two words') await page.keyboard.type(key);
  await page.waitForTimeout(RECOVERY_WAIT_MS);
  const writes = await commandsNamed(calls, 'markdown_recovery_write');
  check(results, 'a modified document is copied once after a pause, not on every key', writes.length === 1, String(writes.length));
  check(results, 'the copy is the bytes Save would write (BOM, CRLF)', writes.length === 1
    && Buffer.from(writes[0].args.data, 'base64').toString('binary') === Buffer.from('\uFEFF# One\r\ntwo words', 'utf8').toString('binary'));
  await page.keyboard.type('!');
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(RECOVERY_WAIT_MS);
  const names = (await calls()).map((call) => call.command);
  const lastSave = names.lastIndexOf('save_markdown');
  check(results, 'no copy is written after a save of the same text', lastSave > 0 && names.indexOf('markdown_recovery_write', lastSave) === -1);
  check(results, 'saving does not discard through the page (the backend drops the copy)', !names.includes('recovery_end'));
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('text\n') });
  await startEditing(page);
  await page.keyboard.type('x');
  await page.click('#md-close');
  await page.waitForTimeout(100);
  check(results, 'refusing to close keeps the copy', (await commandsNamed(calls, 'recovery_end')).length === 0);
  await page.evaluate(() => { window.__confirmAnswer = true; });
  await page.click('#md-close');
  await page.waitForURL('**/index.html');
  const ended = await commandsNamed(calls, 'recovery_end');
  check(results, 'closing and discarding drops the copy', ended.length === 1 && ended[0].args.discard === true);
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('\uFEFF# Draft\r\n\r\nlost text'), recoverId: '1700000000000-42' });
  await page.waitForSelector('#md-editor .cm-content:focus');
  const text = await page.evaluate(() => document.getElementById('md-editor').shadowRoot.querySelector('.cm-content').textContent);
  check(results, 'a recovered document reopens in the Markdown page with its text', text.includes('lost text') && (await page.textContent('#md-name')) === 'notes.md');
  const modified = await commandsNamed(calls, 'set_document_modified');
  check(results, 'a recovered document is marked modified', modified.length > 0 && modified[modified.length - 1].args.modified === true);
  check(results, 'recovering reads no file and writes none', !(await calls()).some((call) => ['read_markdown', 'save_markdown'].includes(call.command)));
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  const picked = await commandsNamed(calls, 'pick_save_target');
  const saved = await savedCalls(calls);
  check(results, 'the first save of a recovered document goes through Save As', picked.length === 1 && picked[0].args.suggestedName === 'notes.md'
    && saved.length === 1 && saved[0].args.token === 'tok-save');
  check(results, 'a recovered document is saved with its BOM and CRLF', saved.length === 1
    && saved[0].args.data === utf8('\uFEFF# Draft\r\n\r\nlost text'));
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: '', recoverId: '1-1', recoverError: 'locked' });
  await page.waitForURL('**/index.html');
  const names = (await calls()).map((call) => call.command);
  check(results, 'a recovery that cannot be loaded says so and discards nothing', names.includes('dialog.message')
    && !names.includes('recovery_end') && !names.includes('recovery_discard'));
  await context.close();
}

// ── Reloading the page ──
const editorText = (page) => page.evaluate(() => {
  const host = document.getElementById('md-editor');
  const content = host.shadowRoot && host.shadowRoot.querySelector('.cm-content');
  if (content && !host.hidden) return content.textContent;
  return document.getElementById('md-source').value;
});

async function reloadPage(page) {
  await page.reload();
  await page.waitForTimeout(1000);
}

{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('# One\n') });
  await startEditing(page);
  await page.keyboard.press('Control+End');
  await page.keyboard.type('two words');
  await page.waitForTimeout(RECOVERY_WAIT_MS);
  const before = (await calls()).length;
  await reloadPage(page);
  check(results, 'a reload with unsaved changes keeps the text', (await editorText(page)).includes('two words'), await editorText(page));
  const modified = (await calls()).slice(before).filter((call) => call.command === 'set_document_modified');
  check(results, 'a reloaded document is still marked modified', modified.length > 0 && modified[modified.length - 1].args.modified === true);
  const copy = await page.evaluate(() => sessionStorage.getItem('copy'));
  check(results, 'a reload keeps the recovery copy', copy !== null);
  await page.keyboard.press('Control+s');
  await page.waitForTimeout(200);
  const saved = await savedCalls(calls);
  check(results, 'after a reload, Save writes over the open file', saved.length === 1 && saved[0].args.token === null
    && Buffer.from(saved[0].args.data, 'base64').toString('utf8') === '# One\ntwo words');
  await reloadPage(page);
  const left = await page.evaluate(() => sessionStorage.getItem('copy'));
  check(results, 'a save then a reload leaves no recovery copy', left === null);
  check(results, 'a save then a reload shows the saved file, unmodified', (await page.textContent('#md-name')) === 'notes.md'
    && !(await page.evaluate(() => document.body.classList.contains('editing'))));
  const last = (await commandsNamed(calls, 'set_document_modified')).pop();
  check(results, 'the saved file reopens unmodified', last && last.args.modified === false);
  await context.close();
}

// ── Closing with changes ──
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: b64('text\n') });
  await page.click('#md-edit');
  await page.waitForSelector('#md-editor .cm-content:focus');
  await page.keyboard.type('x');
  await page.evaluate(() => window.__listeners['confirm-close'].forEach((handler) => handler({})));
  await page.waitForTimeout(100);
  let names = (await calls()).map((call) => call.command);
  check(results, 'closing the window asks first', names.includes('dialog.confirm') && !names.includes('force_close'));
  await page.evaluate(() => { window.__confirmAnswer = true; });
  await page.evaluate(() => window.__listeners['confirm-close'].forEach((handler) => handler({})));
  await page.waitForTimeout(100);
  names = (await calls()).map((call) => call.command);
  check(results, 'a confirmed close closes', names.includes('force_close'));
  await page.click('#md-close');
  await page.waitForURL('**/index.html');
  check(results, 'Close goes back to the start screen', page.url().endsWith('/index.html'));
  await context.close();
}

// Every way back to the start screen tells the backend the document is closed
// first, so another window may open the file.
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('one\n') });
  await page.click('#md-close');
  await page.waitForURL('**/index.html');
  const names = (await calls()).map((call) => call.command);
  check(results, 'Close closes the document before leaving', names.includes('close_document'), names.join(' '));
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: '', readError: 'open-elsewhere' });
  await page.waitForURL('**/index.html');
  const names = (await calls()).map((call) => call.command);
  check(results, 'a refused open leaves no document open', names.includes('close_document'), names.join(' '));
  await context.close();
}

// ── Errors and language ──
{
  const { context, page, calls } = await openPage({ name: 'big.md', data: '', readError: 'markdown-too-large: the file is larger than 10485760 bytes' });
  await page.waitForURL('**/index.html');
  const message = (await calls()).find((call) => call.command === 'dialog.message');
  check(results, 'a file over 10 MB gets its own message', message && message.args.text.includes('10 MB'));
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: '', readError: 'open-elsewhere', lang: 'es' });
  await page.waitForURL('**/index.html');
  const message = (await calls()).find((call) => call.command === 'dialog.message');
  check(results, 'a file open in another window gets the warning the editors show',
    message && message.args.text.includes('otra ventana') && message.args.options.kind === 'warning',
    message && message.args.text);
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: utf8('one\n'), saveError: 'open-elsewhere' });
  await page.click('#md-save-as');
  await page.waitForTimeout(200);
  const message = (await calls()).find((call) => call.command === 'dialog.message');
  check(results, 'a Save As over a file open in another window says so',
    message && message.args.text.includes('open in another window') && message.args.text.includes('Nothing has been saved'),
    message && message.args.text);
  await context.close();
}
{
  const { context, page, calls } = await openPage({ name: 'latin1.md', data: b64('caf\xE9\n'), lang: 'es' });
  await page.waitForURL('**/index.html');
  const message = (await calls()).find((call) => call.command === 'dialog.message');
  check(results, 'a file that is not UTF-8 is not opened (Spanish message)', message && message.args.text.includes('UTF-8') && message.args.text.includes('archivo'));
  await context.close();
}
{
  const { context, page } = await openPage({ name: 'notes.md', data: b64('x'), lang: 'es' });
  const labels = await page.evaluate(() => [...document.querySelectorAll('.toolbar button')].map((b) => b.textContent));
  check(results, 'the toolbar is in Spanish', labels.join('|') === 'Cerrar|Editar|Guardar|Guardar como…|Exportar como HTML', labels.join('|'));
  await context.close();
}

await browser.close();
server.close();
finish(results);
