// Checks the dialog a save shows when the backend refuses it for lack of disk
// space: Save and Save As say so, and any other failure keeps the generic
// save error.
//
// Run: node scripts/test-bridge-save-no-space.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const NO_ROOM = 'save-no-space: /tmp/euro-office-lite-1000/1/save_output/0: the save needs about 14 MB free';
const FILLED = 'save-no-space: /tmp/euro-office-lite-1000/1/save_output/0: the disk filled up during the save';
const OTHER = 'x2t did not write the whole document: the PDF has no end mark';

async function setup(failure) {
  const fail = () => { throw failure; };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'tok-current', name: 'report.docx' }),
      save_file: fail,
      save_file_as: fail,
      pick_save_target: () => ({ token: 'tok-copy', name: 'copy.docx' }),
      recovery_mark_saved: () => null,
    },
  });
  const messages = [];
  const endSaves = [];
  bridge.sandbox.__TAURI__.dialog = {
    message: (text, options) => { messages.push({ text, title: options && options.title }); return Promise.resolve(); },
  };
  bridge.sandbox.AscDesktopEditor._currentDocType = 'word';
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: (code) => endSaves.push(code),
  };
  return { bridge, messages, endSaves, t: (key) => bridge.run(`_t(${JSON.stringify(key)})`) };
}

async function save(param, failure) {
  const env = await setup(failure);
  await env.bridge.run(`AscDesktopEditor.LocalFileSave(${JSON.stringify(param)}, '', null, 65, '{}')`);
  env.bridge.stopTimers();
  return env;
}

function showsOnly(env, key) {
  return env.messages.length === 1 && env.messages[0].text === env.t(key)
    && env.messages[0].title === env.t('saveError');
}

for (const [name, param] of [['Save', ''], ['Save As', 'saveas=true;']]) {
  let env = await save(param, NO_ROOM);
  check(results, `${name} refused for lack of room says so`, showsOnly(env, 'saveNoSpaceMsg'),
    JSON.stringify(env.messages));
  check(results, '... and ends the save as failed', env.endSaves.join() === '1', env.endSaves.join());

  env = await save(param, new Error(FILLED));
  check(results, `${name} refused because the disk filled up, thrown as an Error, says so too`,
    showsOnly(env, 'saveNoSpaceMsg'), JSON.stringify(env.messages));

  env = await save(param, OTHER);
  check(results, `${name} failing for another reason keeps the generic message`,
    showsOnly(env, 'saveErrorMsg'), JSON.stringify(env.messages));
}

const env = await setup(OTHER);
const english = env.bridge.run("_UI_STRINGS.en.saveNoSpaceMsg");
check(results, 'the message is there in English', english === env.t('saveNoSpaceMsg') && english.length > 0, english);
check(results, '... and is not the generic one', english !== env.t('saveErrorMsg'), english);
env.bridge.stopTimers();

finish(results);
