// Checks New from template. A template handed over by the system (a double
// click, a launch argument, the Finder: newDocument from take_pending_open_file)
// starts a new document with create_from_template, in this window on the start
// screen and through new_document_in_new_window with a document open. File >
// Open and Recent hand over no such mark, so a template opened there is edited
// as before. The start screen's button asks pick_template for one. The new
// document has no file and is titled like any new one; its first Save As offers
// the template's name in the format it saves, as a name only.
//
// Run: node scripts/test-bridge-new-from-template.mjs [path/to/bridge.js]
import fs from 'node:fs';
import path from 'node:path';
import { check, finish, loadBridge, projectRoot } from './bridge-test-harness.mjs';

const results = [];
const LETTER = { token: 'tok-letter', name: 'Letter.dotx' };
const BUDGET = { token: 'tok-budget', name: 'Budget.xltx' };

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function setup({ docType = null, pending = [], picked = LETTER, created = 'bin', recent = [] } = {}) {
  const calls = [];
  const queue = [...pending];
  const record = (command, answer) => (args) => {
    calls.push({ command, args });
    return typeof answer === 'function' ? answer(args) : answer;
  };
  const bridge = await loadBridge({
    invoke: {
      take_pending_open_file: () => queue.shift() || null,
      create_from_template: record('create_from_template', () => {
        if (created instanceof Error) throw created;
        return created;
      }),
      open_file: record('open_file', 'bin'),
      open_in_new_window: record('open_in_new_window', 'opened'),
      new_document_in_new_window: record('new_document_in_new_window', 'opened'),
      pick_template: record('pick_template', picked),
      recent_files_state: () => ({ enabled: true, files: recent }),
    },
  });
  const env = { bridge, calls, editors: [], sameWindow: [], messages: [] };
  bridge.sandbox.__TAURI__.dialog = {
    message: (text) => { env.messages.push(text); return Promise.resolve(); },
  };
  bridge.sandbox._openEditor = (type) => { env.editors.push(type); };
  bridge.sandbox._eoOpenPath = (file) => { env.sameWindow.push(file); return Promise.resolve(true); };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  env.pendingData = () => bridge.sandbox._pendingFileData || null;
  return env;
}

async function settle(env) {
  await sleep(30);
  env.bridge.stopTimers();
}

function commands(env) {
  return env.calls.map((call) => call.command).join();
}

// Double click or launch argument on the start screen.
{
  const env = await setup({ pending: [{ ...LETTER, newDocument: true }] });
  await settle(env);
  const data = env.pendingData();
  check(results, 'a template handed over on the start screen starts a new document here',
    commands(env) === 'create_from_template' && env.calls[0].args.token === LETTER.token
      && env.editors.join() === 'word' && env.sameWindow.length === 0,
    commands(env) + ' editors=' + env.editors.join());
  check(results, '... with no file and the title of a new document',
    data && data.path === null && data.name === 'New document', JSON.stringify(data));
}

{
  const env = await setup({ pending: [{ ...BUDGET, newDocument: true }] });
  await settle(env);
  const data = env.pendingData();
  check(results, '... in the editor of its family, titled as a new spreadsheet',
    env.editors.join() === 'cell' && data && data.name === 'New spreadsheet', JSON.stringify(data));
}

// The Finder with a document open.
{
  const env = await setup({ docType: 'word', pending: [{ ...LETTER, newDocument: true }] });
  await settle(env);
  check(results, 'with a document open it starts the new document in a new window',
    commands(env) === 'new_document_in_new_window' && env.calls[0].args.token === LETTER.token,
    commands(env));
}

// File > Open hands over no mark: the template is edited.
{
  const env = await setup();
  await env.bridge.run(`AscDesktopEditor.LocalFileOpen(${JSON.stringify(LETTER)})`);
  await settle(env);
  check(results, 'File > Open of a template on the start screen edits it',
    env.sameWindow.length === 1 && env.sameWindow[0].token === LETTER.token
      && !commands(env).includes('create_from_template'), commands(env));
}

{
  const env = await setup({ docType: 'word' });
  await env.bridge.run(`AscDesktopEditor.LocalFileOpen(${JSON.stringify(LETTER)})`);
  await settle(env);
  check(results, 'File > Open of a template with a document open edits it in a new window',
    commands(env) === 'open_in_new_window', commands(env));
}

{
  const entry = { path: '/home/u/Templates/Letter.dotx', token: 'tok-recent', name: 'Letter.dotx' };
  const env = await setup({ recent: [entry] });
  await env.bridge.run(`_eoOpenRecent(${JSON.stringify(entry.path)})`);
  await settle(env);
  check(results, 'a template in Recent is edited',
    env.sameWindow.length === 1 && env.sameWindow[0].token === 'tok-recent'
      && !commands(env).includes('create_from_template'), commands(env));
}

// The start screen's button.
{
  const env = await setup();
  await env.bridge.run('_eoNewFromTemplate()');
  await settle(env);
  check(results, 'New from template asks pick_template with the templates label',
    env.calls[0] && env.calls[0].command === 'pick_template'
      && JSON.stringify(env.calls[0].args) === '{"templates":"Templates"}', JSON.stringify(env.calls));
  check(results, '... and starts a new document from the chosen template',
    commands(env) === 'pick_template,create_from_template' && env.editors.join() === 'word', commands(env));
}

{
  const env = await setup({ picked: null });
  await env.bridge.run('_eoNewFromTemplate()');
  await settle(env);
  check(results, 'cancelling the dialog starts nothing',
    commands(env) === 'pick_template' && env.editors.length === 0, commands(env));
}

{
  const env = await setup({ created: new Error('Letter.dotx is not a template') });
  await env.bridge.run('_eoNewFromTemplate()');
  await settle(env);
  check(results, 'a template that does not convert shows the open error and mounts no editor',
    env.editors.length === 0 && env.messages.length === 1, env.messages.join());
}

// The start screen's button: in index.html next to Open file, wired in
// editor-patches.js to _eoNewFromTemplate, which asks in the interface language.
{
  const html = fs.readFileSync(path.join(projectRoot, 'src/index.html'), 'utf8');
  const button = html.match(/<button class="btn btn-open" data-new-from-template="true">[\s\S]*?<\/button>/);
  check(results, 'the start screen has a New from template button beside Open file',
    button !== null && button[0].includes('data-i18n="newFromTemplate"')
      && /<div class="open-row">\s*<button class="btn btn-open" data-open="true">/.test(html),
    button ? 'found' : 'missing');
  const patches = fs.readFileSync(path.join(projectRoot, 'src/editor-patches.js'), 'utf8');
  check(results, '... and a click on it calls _eoNewFromTemplate',
    /querySelector\('\.btn\[data-new-from-template\]'\)\.addEventListener\('click'[\s\S]{0,120}window\._eoNewFromTemplate\(\)/.test(patches));
}

for (const [lang, label] of [['es', 'Plantillas'], ['ja', 'テンプレート']]) {
  const env = await setup({ picked: null });
  env.bridge.run(`window._eoCurrentLang = ${JSON.stringify(lang)}`);
  await env.bridge.run('_eoNewFromTemplate()');
  await settle(env);
  check(results, `in ${lang} the template dialog's filter is "${label}"`,
    env.calls[0] && env.calls[0].args.templates === label, JSON.stringify(env.calls[0]));
}

{
  const env = await setup();
  const langs = JSON.parse(env.bridge.run('JSON.stringify(Object.keys(_UI_STRINGS))'));
  const missing = JSON.parse(env.bridge.run(`JSON.stringify(Object.keys(_UI_STRINGS).filter(function(lang) {
    return !_UI_STRINGS[lang].newFromTemplate || !_UI_STRINGS[lang].templates;
  }))`));
  env.bridge.stopTimers();
  check(results, `the ${langs.length} interface languages all have both texts`,
    langs.length === 10 && missing.length === 0, 'missing: ' + missing.join());
}

// The first Save As of the new document.
async function firstSaveAs({ template, docType, fileType, saveAs = false }) {
  const picked = [];
  const bridge = await loadBridge({
    invoke: {
      create_from_template: () => 'bin',
      write_editor_bin: () => 'ok',
      current_document: () => null,
      pick_save_target: (args) => { picked.push(args); return null; },
    },
  });
  bridge.sandbox.__TAURI__.dialog = { message: () => Promise.resolve() };
  bridge.sandbox._openEditor = () => {};
  if (template) await bridge.run(`_eoCreateFromTemplate(${JSON.stringify(template)})`);
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: () => {},
  };
  const param = saveAs ? 'saveas=true' : '';
  await bridge.run(`AscDesktopEditor.LocalFileSave('${param}', '', null, ${fileType}, '')`);
  bridge.stopTimers();
  return picked[0] || {};
}

{
  const picked = await firstSaveAs({ template: LETTER, docType: 'word', fileType: 65 });
  check(results, 'Ctrl+S on it asks where, offering the template\'s name as a .docx',
    picked.suggestedName === 'Letter.docx' && picked.requestedExt === 'docx', JSON.stringify(picked));
}

{
  const picked = await firstSaveAs({ template: { token: 't', name: 'Q3 Budget.v2.xltx' }, docType: 'cell', fileType: 257 });
  check(results, '... a spreadsheet one as a .xlsx, keeping the dots of its name',
    picked.suggestedName === 'Q3 Budget.v2.xlsx', JSON.stringify(picked));
}

{
  const picked = await firstSaveAs({ template: LETTER, docType: 'word', fileType: 513, saveAs: true });
  check(results, '... and a PDF export as a .pdf', picked.suggestedName === 'Letter.pdf', JSON.stringify(picked));
}

{
  const picked = await firstSaveAs({ template: null, docType: 'word', fileType: 65 });
  check(results, 'a blank new document offers no name', !('suggestedName' in picked), JSON.stringify(picked));
}

finish(results);
