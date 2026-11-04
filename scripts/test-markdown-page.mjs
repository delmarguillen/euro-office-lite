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
    localStorage.setItem('eo-markdown-open', JSON.stringify({ token: 'tok-open', name: fake.name }));
  }
  const handlers = {
    read_markdown: (args) => {
      if (args.token !== 'tok-open') throw new Error('Unknown or expired file token');
      if (fake.readError) throw fake.readError;
      return { data: fake.data, name: fake.name };
    },
    save_markdown: (args) => (args.token ? 'copy.md' : fake.name),
    pick_save_target: () => ({ token: 'tok-save', name: 'copy.md' }),
    markdown_image: (args) => {
      const image = (fake.images || {})[args.src];
      if (!image) throw new Error('the image is outside the document\'s folder');
      return image;
    },
    current_document: () => null,
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

// A fresh page with `fake` as the backend. Requests leaving 127.0.0.1 and CSP
// reports are collected.
async function openPage(fake) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const outside = [];
  const csp = [];
  page.on('request', (request) => {
    if (!request.url().startsWith('http://127.0.0.1') && !request.url().startsWith('data:')) {
      outside.push(request.url());
    }
  });
  page.on('console', (message) => {
    if (/Content Security Policy/i.test(message.text())) csp.push(message.text());
  });
  await page.addInitScript(installFakeTauri, fake);
  await page.goto(base);
  await page.waitForFunction(() => window.__calls.some((call) => call.command === 'read_markdown'));
  await page.waitForTimeout(400);
  const calls = () => page.evaluate(() => window.__calls);
  const preview = () => page.evaluate(() => document.getElementById('md-preview').contentDocument.body.innerHTML);
  return { context, page, outside, csp, calls, preview };
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
  const hidden = () => page.evaluate(() => document.getElementById('md-source').hidden);
  check(results, 'the page opens on the preview alone', await hidden());
  await page.click('#md-edit');
  check(results, 'Edit shows the textarea', !(await hidden()));
  await page.focus('#md-source');
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

// ── Closing with changes ──
{
  const { context, page, calls } = await openPage({ name: 'notes.md', data: b64('text\n') });
  await page.click('#md-edit');
  await page.focus('#md-source');
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

// ── Errors and language ──
{
  const { context, page, calls } = await openPage({ name: 'big.md', data: '', readError: 'markdown-too-large: the file is larger than 10485760 bytes' });
  await page.waitForURL('**/index.html');
  const message = (await calls()).find((call) => call.command === 'dialog.message');
  check(results, 'a file over 10 MB gets its own message', message && message.args.text.includes('10 MB'));
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
  check(results, 'the toolbar is in Spanish', labels.join('|') === 'Cerrar|Editar|Guardar|Guardar como…', labels.join('|'));
  await context.close();
}

await browser.close();
server.close();
finish(results);
