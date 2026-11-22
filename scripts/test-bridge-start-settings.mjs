// Checks the start screen settings panel: the gear opens and closes it, Escape
// and a click outside close it, opening a document closes it, focus moves to
// the first control and back to the gear, and the three controls inside keep
// working. The two folder settings ask the backend to choose or clear a folder
// and show the path it answers. Also checks index.html: the controls sit only
// inside the panel.
//
// Run: node scripts/test-bridge-start-settings.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const FOCUSABLE = new Set(['button', 'select', 'input']);
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

// Just enough of the DOM for the panel: a tree, attributes, focus, events
// that bubble up to the document.
class FakeElement {
  constructor(doc, tag, id, parent) {
    this.doc = doc;
    this.tagName = tag.toUpperCase();
    this.id = id || '';
    this.parent = parent || null;
    this.children = [];
    this.hidden = false;
    this.attributes = new Map();
    this.listeners = new Map();
    this.value = '';
    this.checked = false;
    const classes = new Set();
    this.classList = {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
    };
    if (parent) parent.children.push(this);
    if (id) doc.byId.set(id, this);
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)); }
  getAttribute(name) { return this.attributes.has(name) ? this.attributes.get(name) : null; }
  addEventListener(type, handler) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(handler);
  }
  contains(other) {
    for (let node = other; node; node = node.parent) {
      if (node === this) return true;
    }
    return false;
  }
  focus() { this.doc.activeElement = this; }
  blur() { this.doc.activeElement = this.doc.body; }
  // Only the selector the bridge uses: the first focusable descendant.
  querySelector() {
    for (const child of this.children) {
      if (FOCUSABLE.has(child.tagName.toLowerCase())) return child;
      const found = child.querySelector();
      if (found) return found;
    }
    return null;
  }
}

function dispatch(doc, target, type, extra = {}) {
  const event = {
    type,
    target,
    defaultPrevented: false,
    preventDefault() { this.defaultPrevented = true; },
    ...extra,
  };
  for (let node = target; node; node = node.parent) {
    for (const handler of node.listeners.get(type) || []) handler.call(node, event);
  }
  for (const handler of doc.listeners.get(type) || []) handler.call(doc, event);
  return event;
}

// A click moves focus to a focusable element first, as a browser does.
function click(doc, target) {
  if (FOCUSABLE.has(target.tagName.toLowerCase())) target.focus();
  return dispatch(doc, target, 'click');
}

function pressKey(doc, key) {
  return dispatch(doc, doc.activeElement || doc.body, 'keydown', { key });
}

function buildStartScreen() {
  const doc = {
    byId: new Map(),
    listeners: new Map(),
    activeElement: null,
    readyState: 'complete',
    getElementById(id) { return this.byId.get(id) || null; },
    addEventListener(type, handler) {
      if (!this.listeners.has(type)) this.listeners.set(type, []);
      this.listeners.get(type).push(handler);
    },
    querySelector() { return null; },
    querySelectorAll() { return []; },
  };
  doc.body = new FakeElement(doc, 'body');
  doc.activeElement = doc.body;
  const start = new FakeElement(doc, 'div', 'start-screen', doc.body);
  const settings = new FakeElement(doc, 'div', '', start);
  const toggle = new FakeElement(doc, 'button', 'settings-toggle', settings);
  toggle.setAttribute('aria-expanded', 'false');
  const panel = new FakeElement(doc, 'div', 'settings-panel', settings);
  panel.hidden = true;
  const langSection = new FakeElement(doc, 'div', '', panel);
  new FakeElement(doc, 'label', '', langSection);
  const select = new FakeElement(doc, 'select', 'lang-select', langSection);
  const spellSection = new FakeElement(doc, 'div', '', panel);
  const addLanguage = new FakeElement(doc, 'button', 'add-language', spellSection);
  const recentSection = new FakeElement(doc, 'div', '', panel);
  const remember = new FakeElement(doc, 'input', 'recent-remember', recentSection);
  const folders = {};
  for (const kind of ['documents', 'templates']) {
    const section = new FakeElement(doc, 'div', '', panel);
    folders[kind] = {
      path: new FakeElement(doc, 'p', kind + '-folder-path', section),
      choose: new FakeElement(doc, 'button', kind + '-folder-choose', section),
      reset: new FakeElement(doc, 'button', kind + '-folder-reset', section),
    };
    folders[kind].reset.hidden = true;
  }
  const content = new FakeElement(doc, 'div', '', start);
  const newDocument = new FakeElement(doc, 'button', '', content);
  const heading = new FakeElement(doc, 'h1', '', content);
  const opening = new FakeElement(doc, 'div', 'opening', doc.body);
  new FakeElement(doc, 'p', 'opening-text', opening);
  return { doc, toggle, panel, select, addLanguage, remember, folders, newDocument, heading };
}

const NO_FOLDERS = { documents: null, templates: null };

// stored: what the backend holds at start. picked: what the folder dialog
// answers, null for Cancel.
async function setup({ recentFails = false, stored = NO_FOLDERS, picked = null, folderFails = false } = {}) {
  const recentCalls = [];
  const folderCalls = [];
  let held = { ...stored };
  const folderCommand = (command, change) => (args) => {
    folderCalls.push({ command, args });
    if (folderFails) throw new Error('disk full');
    held = change(args);
    return { ...held };
  };
  const bridge = await loadBridge({
    invoke: {
      default_folders_state: () => ({ ...held }),
      pick_default_folder: folderCommand('pick_default_folder',
        (args) => (picked ? { ...held, [args.kind]: picked } : held)),
      clear_default_folder: folderCommand('clear_default_folder',
        (args) => ({ ...held, [args.kind]: null })),
      set_recent_files_enabled: (args) => {
        recentCalls.push(args);
        if (recentFails) throw new Error('disk full');
        return null;
      },
    },
  });
  const screen = buildStartScreen();
  bridge.sandbox.document = screen.doc;
  const addLanguageCalls = [];
  bridge.sandbox._eoAddLanguage = (...args) => { addLanguageCalls.push(args); return Promise.resolve(null); };
  const callbacks = { language: 0, recent: 0 };
  bridge.sandbox.__options = {
    onLanguageChange: () => { callbacks.language += 1; },
    onRecentChange: () => { callbacks.recent += 1; },
  };
  bridge.run('_eoInitStartSettings(window.__options)');
  await settle();
  return { bridge, ...screen, recentCalls, folderCalls, addLanguageCalls, callbacks };
}

{
  const env = await setup();
  check(results, 'the panel starts closed', env.panel.hidden && env.toggle.getAttribute('aria-expanded') === 'false');

  click(env.doc, env.toggle);
  check(results, 'a click on the gear opens the panel',
    !env.panel.hidden && env.toggle.getAttribute('aria-expanded') === 'true');
  check(results, '... and focuses its first control (the language list)', env.doc.activeElement === env.select);

  click(env.doc, env.toggle);
  check(results, 'a second click on the gear closes it',
    env.panel.hidden && env.toggle.getAttribute('aria-expanded') === 'false');
  check(results, '... and gives the focus back to the gear', env.doc.activeElement === env.toggle);
}

{
  const env = await setup();
  click(env.doc, env.toggle);
  const event = pressKey(env.doc, 'Escape');
  check(results, 'Escape closes the panel', env.panel.hidden && event.defaultPrevented);
  check(results, '... and gives the focus back to the gear', env.doc.activeElement === env.toggle);

  const closed = pressKey(env.doc, 'Escape');
  check(results, 'Escape with the panel closed is left alone', !closed.defaultPrevented);

  click(env.doc, env.toggle);
  pressKey(env.doc, 'Tab');
  check(results, 'other keys leave the panel open', !env.panel.hidden);
}

{
  const env = await setup();
  click(env.doc, env.toggle);
  click(env.doc, env.addLanguage);
  check(results, 'a click inside the panel keeps it open', !env.panel.hidden);

  click(env.doc, env.newDocument);
  check(results, 'a click on a button outside closes the panel', env.panel.hidden);
  check(results, '... and leaves the focus on that button', env.doc.activeElement === env.newDocument);

  click(env.doc, env.toggle);
  click(env.doc, env.heading);
  check(results, 'a click on plain text outside closes the panel', env.panel.hidden);
  check(results, '... and gives the focus back to the gear', env.doc.activeElement === env.toggle);
}

{
  const env = await setup();
  click(env.doc, env.toggle);
  env.bridge.run("_eoShowOpening('report.docx')");
  check(results, 'opening a document closes the panel', env.panel.hidden);
  check(results, '... without moving the focus to the gear', env.doc.activeElement !== env.toggle);

  env.bridge.run('_eoCloseSettings(false)');
  check(results, 'closing a closed panel does nothing', env.panel.hidden);
}

{
  const env = await setup();
  click(env.doc, env.toggle);
  env.select.value = 'fr';
  dispatch(env.doc, env.select, 'change');
  check(results, 'the language list still sets the UI language',
    env.bridge.run('window._eoCurrentLang') === 'fr'
    && env.bridge.sandbox.localStorage.getItem('eo-ui-lang') === 'fr');
  check(results, '... and runs the start screen update', env.callbacks.language === 1);
  check(results, '... and the new labels follow it', env.bridge.run("_t('settings')") === 'Paramètres');
  check(results, 'changing the language keeps the panel open', !env.panel.hidden);
}

{
  const env = await setup();
  click(env.doc, env.toggle);
  click(env.doc, env.addLanguage);
  check(results, 'the spell check button still starts _eoAddLanguage()',
    env.addLanguageCalls.length === 1 && env.addLanguageCalls[0].length === 0, JSON.stringify(env.addLanguageCalls));
}

{
  const env = await setup();
  env.remember.checked = false;
  dispatch(env.doc, env.remember, 'change');
  await settle();
  check(results, 'the recent files switch still stores the preference',
    env.recentCalls.length === 1 && env.recentCalls[0].enabled === false, JSON.stringify(env.recentCalls));
  check(results, '... and refreshes the list', env.callbacks.recent === 1);
}

{
  const env = await setup({ recentFails: true });
  env.remember.checked = true;
  dispatch(env.doc, env.remember, 'change');
  await settle();
  await settle();
  check(results, 'a failed switch is logged and the list is not refreshed',
    env.callbacks.recent === 0 && env.bridge.logs.some((line) => line.includes('Recent files toggle failed: disk full')));
}

// The folders the file dialogs start in.
const DOCS = '/home/u/Documents/Work';
const TEMPLATES = '/home/u/Templates';

function shownFolder(env, kind) {
  const parts = env.folders[kind];
  return { text: parts.path.textContent, title: parts.path.title, reset: !parts.reset.hidden };
}

{
  const env = await setup();
  for (const kind of ['documents', 'templates']) {
    const shown = shownFolder(env, kind);
    check(results, `with no ${kind} folder set the panel says so and offers no Reset`,
      shown.text === 'Not set' && shown.title === '' && !shown.reset, JSON.stringify(shown));
  }
  check(results, '... and nothing was sent to the backend', env.folderCalls.length === 0);
}

{
  const env = await setup({ stored: { documents: DOCS, templates: null } });
  const documents = shownFolder(env, 'documents');
  check(results, 'a folder already set is shown with its Reset',
    documents.text === DOCS && documents.title === DOCS && documents.reset, JSON.stringify(documents));
  check(results, '... and the other one stays not set',
    shownFolder(env, 'templates').text === 'Not set' && !shownFolder(env, 'templates').reset);
}

{
  const env = await setup({ picked: DOCS });
  click(env.doc, env.toggle);
  click(env.doc, env.folders.documents.choose);
  await settle();
  check(results, 'Choose asks the backend for the documents folder, with the dialog title and no path',
    env.folderCalls.length === 1 && env.folderCalls[0].command === 'pick_default_folder'
      && JSON.stringify(env.folderCalls[0].args) === '{"kind":"documents","title":"Documents folder"}',
    JSON.stringify(env.folderCalls));
  const shown = shownFolder(env, 'documents');
  check(results, '... and shows the folder the backend answers, with Reset',
    shown.text === DOCS && shown.title === DOCS && shown.reset, JSON.stringify(shown));
  check(results, '... leaving the templates folder as it was', shownFolder(env, 'templates').text === 'Not set');
  check(results, '... and the panel open', !env.panel.hidden);
}

{
  const env = await setup({ picked: TEMPLATES });
  click(env.doc, env.folders.templates.choose);
  await settle();
  check(results, 'Choose on the templates folder names that one',
    JSON.stringify(env.folderCalls[0].args) === '{"kind":"templates","title":"Templates folder"}',
    JSON.stringify(env.folderCalls));
  check(results, '... and shows it', shownFolder(env, 'templates').text === TEMPLATES
    && shownFolder(env, 'documents').text === 'Not set');
}

{
  const env = await setup({ stored: { documents: DOCS, templates: null }, picked: null });
  click(env.doc, env.folders.documents.choose);
  await settle();
  check(results, 'cancelling the folder dialog keeps the folder shown',
    env.folderCalls.length === 1 && shownFolder(env, 'documents').text === DOCS
      && shownFolder(env, 'documents').reset);
}

{
  const env = await setup({ stored: { documents: DOCS, templates: TEMPLATES } });
  click(env.doc, env.toggle);
  click(env.doc, env.folders.documents.reset);
  await settle();
  check(results, 'Reset asks the backend to clear that folder',
    env.folderCalls.length === 1 && env.folderCalls[0].command === 'clear_default_folder'
      && JSON.stringify(env.folderCalls[0].args) === '{"kind":"documents"}', JSON.stringify(env.folderCalls));
  const shown = shownFolder(env, 'documents');
  check(results, '... and the panel shows no folder set and no Reset',
    shown.text === 'Not set' && shown.title === '' && !shown.reset, JSON.stringify(shown));
  check(results, '... with the focus moved to Choose', env.doc.activeElement === env.folders.documents.choose);
  check(results, '... and the templates folder kept', shownFolder(env, 'templates').text === TEMPLATES
    && shownFolder(env, 'templates').reset);
  check(results, '... and the panel open', !env.panel.hidden);
}

{
  const env = await setup({ stored: { documents: DOCS, templates: null }, picked: TEMPLATES, folderFails: true });
  click(env.doc, env.folders.documents.choose);
  click(env.doc, env.folders.documents.reset);
  await settle();
  await settle();
  check(results, 'a folder the backend could not store or clear is logged',
    env.bridge.logs.filter((line) => line.includes('Default folder not changed: disk full')).length === 2,
    env.bridge.logs.join(' | '));
  check(results, '... and the panel keeps showing the folder that is set',
    shownFolder(env, 'documents').text === DOCS && shownFolder(env, 'documents').reset);
}

{
  const env = await setup({ stored: { documents: null, templates: TEMPLATES } });
  env.select.value = 'es';
  dispatch(env.doc, env.select, 'change');
  check(results, 'changing the language repaints "Not set"', shownFolder(env, 'documents').text === 'Sin definir');
  check(results, '... and leaves a folder path as it is', shownFolder(env, 'templates').text === TEMPLATES);
}

// The start screen's own light or dark look.
function buildThemeScreen(bridge) {
  const makeButton = (choice) => {
    const attributes = new Map();
    return {
      dataset: { themeChoice: choice },
      setAttribute: (name, value) => attributes.set(name, String(value)),
      getAttribute: (name) => (attributes.has(name) ? attributes.get(name) : null),
    };
  };
  const dark = makeButton('dark');
  const light = makeButton('light');
  const documentElement = { dataset: {} };
  bridge.sandbox.document = {
    documentElement,
    querySelectorAll: (selector) => (selector === '[data-theme-choice]' ? [dark, light] : []),
  };
  return { dark, light, documentElement };
}

function failingStorage(bridge, method) {
  const storage = bridge.sandbox.localStorage;
  storage[method] = () => { throw new Error('storage blocked'); };
}

{
  const bridge = await loadBridge();
  check(results, 'with nothing stored the start theme is dark', bridge.run('_eoStoredTheme()') === 'dark');

  bridge.sandbox.localStorage.setItem('eo-start-theme', 'light');
  check(results, 'a stored light theme is read back', bridge.run('_eoStoredTheme()') === 'light');

  bridge.sandbox.localStorage.setItem('eo-start-theme', 'blue');
  check(results, 'an unknown stored theme falls back to dark', bridge.run('_eoStoredTheme()') === 'dark');
}

{
  const bridge = await loadBridge();
  failingStorage(bridge, 'getItem');
  let theme = null;
  let threw = false;
  try {
    theme = bridge.run('_eoStoredTheme()');
  } catch (error) {
    threw = true;
  }
  check(results, 'a storage that throws on read gives dark without throwing', !threw && theme === 'dark');
}

{
  const bridge = await loadBridge();
  const screen = buildThemeScreen(bridge);
  bridge.run("_eoChooseTheme('light')");
  check(results, 'choosing light stores it', bridge.sandbox.localStorage.getItem('eo-start-theme') === 'light');
  check(results, '... sets data-theme on the document element', screen.documentElement.dataset.theme === 'light');
  check(results, '... and presses only the light button',
    screen.light.getAttribute('aria-pressed') === 'true' && screen.dark.getAttribute('aria-pressed') === 'false');
}

{
  const bridge = await loadBridge();
  const screen = buildThemeScreen(bridge);
  failingStorage(bridge, 'setItem');
  let threw = false;
  try {
    bridge.run("_eoChooseTheme('light')");
  } catch (error) {
    threw = true;
  }
  check(results, 'a storage that throws on write does not stop choosing the theme', !threw);
  check(results, '... and the theme is still applied',
    screen.documentElement.dataset.theme === 'light' && screen.light.getAttribute('aria-pressed') === 'true');
}

{
  const bridge = await loadBridge();
  const missing = bridge.run(`
    ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'uk', 'zh', 'ja'].reduce(function(out, lang) {
      ['settings', 'interfaceLanguage', 'spellCheck', 'addLanguage', 'recentFiles', 'rememberRecent',
        'documentsFolder', 'templatesFolder', 'chooseFolder', 'resetFolder', 'folderNotSet'].forEach(function(key) {
        if (!_UI_STRINGS[lang][key]) out.push(lang + '.' + key);
      });
      return out;
    }, [])
  `);
  check(results, 'every panel label is translated in the ten languages', missing.length === 0, missing.join(', '));
}

// index.html: where each control is.
{
  const html = await readFile(path.join(projectRoot, 'src', 'index.html'), 'utf8');

  // The panel's markup: from its opening tag to the </div> that closes it.
  function elementSpan(source, marker) {
    const start = source.lastIndexOf('<div', source.indexOf(marker));
    const tags = /<div\b|<\/div>/g;
    tags.lastIndex = start;
    let depth = 0;
    let match;
    while ((match = tags.exec(source))) {
      depth += match[0] === '</div>' ? -1 : 1;
      if (depth === 0) return { start, end: match.index + match[0].length };
    }
    throw new Error('unclosed ' + marker);
  }

  const panel = elementSpan(html, 'id="settings-panel"');
  const inside = html.slice(panel.start, panel.end);
  const outside = html.slice(0, panel.start) + html.slice(panel.end);
  const count = (text, needle) => text.split(needle).length - 1;

  const folderIds = ['documents', 'templates'].flatMap(
    (kind) => ['path', 'choose', 'reset'].map((part) => `${kind}-folder-${part}`));
  for (const id of ['lang-select', 'add-language', 'recent-remember', ...folderIds]) {
    check(results, `#${id} is inside the panel and nowhere else`,
      count(inside, `id="${id}"`) === 1 && count(outside, `id="${id}"`) === 0);
  }
  for (const kind of ['documents', 'templates']) {
    check(results, `the ${kind} folder path is not a translated label, so a language change keeps it`,
      new RegExp(`<p class="folder-path" id="${kind}-folder-path"></p>`).test(inside));
    check(results, `the ${kind} folder's two buttons are named as a group by its translated label`,
      new RegExp(`class="folder-actions" role="group" data-i18n-label="${kind}Folder" aria-label="[^"]+">\\s*`
        + `<button[^>]*id="${kind}-folder-choose"`).test(inside));
    check(results, `the ${kind} folder's Reset starts hidden`,
      new RegExp(`id="${kind}-folder-reset"[^>]*\\bhidden\\b`).test(inside));
  }
  check(results, 'no add-language link is left outside the panel', !outside.includes('add-language'));
  check(results, 'the Clear button stays with the recent list',
    count(outside, 'id="recent-clear"') === 1 && count(inside, 'id="recent-clear"') === 0);
  check(results, 'the panel starts hidden', /id="settings-panel"[^>]*\bhidden\b/.test(html));
  check(results, 'the gear has a translated label',
    /id="settings-toggle"[^>]*data-i18n-label="settings"/.test(html)
    && /id="settings-toggle"[^>]*aria-label="Settings"/.test(html));
  check(results, 'the old top right language selector is gone', !html.includes('lang-selector'));
}

finish(results);
