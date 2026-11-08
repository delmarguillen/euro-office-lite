// Checks the start screen settings panel: the gear opens and closes it, Escape
// and a click outside close it, opening a document closes it, focus moves to
// the first control and back to the gear, and the three controls inside keep
// working. Also checks index.html: the controls sit only inside the panel.
//
// Run: node scripts/test-bridge-start-settings.mjs [path/to/bridge.js]
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const FOCUSABLE = new Set(['button', 'select', 'input']);

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
  const content = new FakeElement(doc, 'div', '', start);
  const newDocument = new FakeElement(doc, 'button', '', content);
  const heading = new FakeElement(doc, 'h1', '', content);
  const opening = new FakeElement(doc, 'div', 'opening', doc.body);
  new FakeElement(doc, 'p', 'opening-text', opening);
  return { doc, toggle, panel, select, addLanguage, remember, newDocument, heading };
}

async function setup({ recentFails = false } = {}) {
  const recentCalls = [];
  const bridge = await loadBridge({
    invoke: {
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
  return { bridge, ...screen, recentCalls, addLanguageCalls, callbacks };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

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

{
  const bridge = await loadBridge();
  const missing = bridge.run(`
    ['en', 'es', 'fr', 'de', 'it', 'pt', 'ru', 'uk', 'zh', 'ja'].reduce(function(out, lang) {
      ['settings', 'interfaceLanguage', 'spellCheck', 'addLanguage', 'recentFiles', 'rememberRecent'].forEach(function(key) {
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

  for (const id of ['lang-select', 'add-language', 'recent-remember']) {
    check(results, `#${id} is inside the panel and nowhere else`,
      count(inside, `id="${id}"`) === 1 && count(outside, `id="${id}"`) === 0);
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
