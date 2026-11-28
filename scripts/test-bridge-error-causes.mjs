// Checks that the dialog of a failed open, save, compare or merge says the
// cause when the backend's error starts with one: a text x2t would not read
// whole, an empty file, an x2t that stopped, a full disk. Any other error
// keeps the generic message.
//
// Run: node scripts/test-bridge-error-causes.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const FILE = { token: 'tok-file', name: 'notes.txt' };
const X2T_FAILED = 'x2t conversion failed (exit code 80): ';
const X2T_STOPPED = 'x2t-stopped: x2t was killed by a signal';
const NO_SPACE = 'save-no-space: /home/user/docs/notes.txt: No space left on device (os error 28)';

async function setup(invoke) {
  const bridge = await loadBridge({ invoke });
  const env = { bridge, messages: [], handled: [], endSaves: [] };
  bridge.sandbox.__TAURI__.dialog = {
    message: (text, options) => {
      env.messages.push({ text, title: options && options.title });
      return Promise.resolve();
    },
  };
  env.t = (key) => bridge.run(`_t(${JSON.stringify(key)})`);
  return env;
}

async function settle(env) {
  await new Promise((resolve) => setTimeout(resolve, 20));
  env.bridge.stopTimers();
}

function showsOnly(env, key, title) {
  return env.messages.length === 1 && env.messages[0].text === env.t(key)
    && env.messages[0].title === env.t(title);
}

const OPEN_CAUSES = [
  ['a text that is not valid in its encoding', 'text-not-readable: not valid UTF-16 text: odd length of 5 bytes', 'openTextMsg'],
  ['a text with a NUL', 'text-not-readable: NUL character at code unit 11', 'openTextMsg'],
  ['a TXT outside the BMP', 'text-outside-bmp: a TXT with a character outside the BMP', 'openTextOutsideBmpMsg'],
  ['an empty file', 'empty-file', 'openEmptyMsg'],
  ['an x2t that stopped', X2T_STOPPED, 'openConverterMsg'],
  ['an x2t that refused the document', X2T_FAILED, 'openErrorMsg'],
  ['a file that is not valid', 'This file is not a valid .txt', 'openErrorMsg'],
];

for (const [name, error, key] of OPEN_CAUSES) {
  for (const thrown of [error, new Error(error)]) {
    const env = await setup({ open_file: () => { throw thrown; } });
    env.bridge.sandbox.AscDesktopEditor._currentDocType = null;
    await env.bridge.run(`_eoOpenGranted(${JSON.stringify(FILE)})`);
    await settle(env);
    const how = thrown === error ? '' : ', thrown as an Error';
    check(results, `Open of ${name}${how} shows ${key}`, showsOnly(env, key, 'openFailed'),
      JSON.stringify(env.messages));
  }
}

async function save(failure, failing) {
  const fail = () => { throw failure; };
  const env = await setup({
    write_editor_bin: failing === 'write_editor_bin' ? fail : () => 'ok',
    current_document: () => ({ token: 'tok-current', name: 'notes.txt' }),
    save_file: fail,
    recovery_mark_saved: () => null,
  });
  env.bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
  env.bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: (code) => env.endSaves.push(code),
  };
  await env.bridge.run(`AscDesktopEditor.LocalFileSave('', '', null, 69, '{}')`);
  await settle(env);
  return env;
}

const SAVE_CAUSES = [
  ['a full disk', NO_SPACE, 'save_file', 'saveNoSpaceMsg'],
  ['a full disk when Editor.bin is written', NO_SPACE, 'write_editor_bin', 'saveNoSpaceMsg'],
  ['an x2t that stopped', X2T_STOPPED, 'save_file', 'saveConverterMsg'],
  ['an x2t that refused the format', 'x2t conversion failed (exit code 88): ', 'save_file', 'saveErrorMsg'],
];

for (const [name, error, failing, key] of SAVE_CAUSES) {
  const env = await save(error, failing);
  check(results, `Save failing for ${name} shows ${key}`, showsOnly(env, key, 'saveError'),
    JSON.stringify(env.messages));
  check(results, '... and ends the save as failed', env.endSaves.join() === '1', env.endSaves.join());
}

async function compare(failure) {
  const env = await setup({ convert_for_insert: () => { throw failure; } });
  env.bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
  env.bridge.sandbox.AscDesktopEditor._editorWindow = {
    onDocumentCompare: (...args) => env.handled.push(args),
  };
  const granted = env.bridge.run(`_eoEditorFileName(${JSON.stringify(FILE)})`);
  await env.bridge.run(`AscDesktopEditor.CompareDocumentFile(${JSON.stringify(granted)}, null)`);
  await settle(env);
  return env;
}

const INSERT_CAUSES = [
  ['a TXT x2t is given no options for', 'x2t conversion failed (exit code 89): ', 'insertFailedMsg'],
  ['a text with a NUL', 'text-not-readable: NUL character at code unit 11', 'openTextMsg'],
  ['an empty file', 'empty-file', 'openEmptyMsg'],
];

for (const [name, error, key] of INSERT_CAUSES) {
  const env = await compare(error);
  check(results, `Compare with ${name} shows ${key}`, showsOnly(env, key, 'openFailed'),
    JSON.stringify(env.messages));
  check(results, '... logs the error and hands the editor nothing',
    env.handled.length === 0 && env.bridge.logs.some((line) => line.includes(error)),
    JSON.stringify(env.bridge.logs.slice(-2)));
}

const NEW_KEYS = [
  'openTextMsg', 'openTextOutsideBmpMsg', 'openEmptyMsg', 'openConverterMsg',
  'saveNoSpaceMsg', 'saveConverterMsg', 'insertFailedMsg',
];
const env = await setup({});
const missing = NEW_KEYS.filter((key) => {
  const text = env.bridge.run(`_UI_STRINGS.en[${JSON.stringify(key)}]`);
  return typeof text !== 'string' || text.length === 0;
});
check(results, 'English has every new message', missing.length === 0, missing.join());
env.bridge.stopTimers();

finish(results);
