// Checks the dialogs a failed PDF export or print shows: the backend's
// font-cache-unavailable error gets its own message, and a print that fails
// for any reason says so instead of doing nothing.
//
// Run: node scripts/test-bridge-pdf-errors.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const FONT_ERROR = 'font-cache-unavailable: the font cache could not be built (GaveUp)';

async function setup(failures) {
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      get_current_path: () => 'C:/docs/report.docx',
      save_file_as: () => { throw failures.saveAs; },
      print_document: () => { throw failures.print; },
    },
  });
  const messages = [];
  const endSaves = [];
  bridge.sandbox.__TAURI__.dialog = {
    save: () => Promise.resolve('C:/docs/report.pdf'),
    message: (text, options) => { messages.push({ text, title: options && options.title }); return Promise.resolve(); },
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => (failures.noData ? null : 'bin') } },
    DesktopOfflineAppDocumentEndSave: (code) => endSaves.push(code),
  };
  return { bridge, messages, endSaves, t: (key) => bridge.run(`_t(${JSON.stringify(key)})`) };
}

async function exportPdf(failure) {
  const env = await setup({ saveAs: failure });
  await env.bridge.run("AscDesktopEditor.LocalFileSave('saveas=true', '', null, 513, '')");
  env.bridge.stopTimers();
  return env;
}

async function print(failure, noData = false) {
  const env = await setup({ print: failure, noData });
  await env.bridge.run("AscDesktopEditor.Print('')");
  env.bridge.stopTimers();
  return env;
}

let env = await exportPdf(FONT_ERROR);
check(results, 'a PDF export without the font cache explains why',
  env.messages.length === 1 && env.messages[0].text === env.t('pdfFontsErrorMsg'),
  JSON.stringify(env.messages));
check(results, '... under the usual save error title', env.messages[0] && env.messages[0].title === env.t('saveError'));
check(results, '... and ends the save as failed', env.endSaves.join() === '1', env.endSaves.join());

env = await exportPdf('x2t conversion failed (exit code 89): ');
check(results, 'any other export error keeps the generic message',
  env.messages.length === 1 && env.messages[0].text === env.t('saveErrorMsg'),
  JSON.stringify(env.messages));

env = await print('x2t conversion failed (exit code 80): ');
check(results, 'a failed print shows a dialog',
  env.messages.length === 1 && env.messages[0].text === env.t('printErrorMsg'),
  JSON.stringify(env.messages));
check(results, '... titled as a print error', env.messages[0] && env.messages[0].title === env.t('printError'));
check(results, '... and ends the print as failed', env.endSaves.join() === '1', env.endSaves.join());

env = await print(FONT_ERROR);
check(results, 'a print without the font cache explains why',
  env.messages.length === 1 && env.messages[0].text === env.t('pdfFontsErrorMsg'),
  JSON.stringify(env.messages));
check(results, '... titled as a print error', env.messages[0] && env.messages[0].title === env.t('printError'));

env = await print(new Error('Editor.bin not found'));
check(results, 'a print error thrown as an Error also shows a dialog',
  env.messages.length === 1 && env.messages[0].text === env.t('printErrorMsg'),
  JSON.stringify(env.messages));

env = await print(null, true);
check(results, 'a print the editor gives no data for shows a dialog',
  env.messages.length === 1 && env.messages[0].text === env.t('printErrorMsg')
    && env.messages[0].title === env.t('printError'),
  JSON.stringify(env.messages));
check(results, '... and ends the print as failed', env.endSaves.join() === '1', env.endSaves.join());

finish(results);
