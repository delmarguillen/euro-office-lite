// Checks that the PDF of a spreadsheet, exported or printed, is sent with the
// locale chosen in the editor (asc_getLocale), and that nothing is sent for a
// text document, a presentation, or a spreadsheet whose editor has no locale yet.
//
// Run: node scripts/test-bridge-sheet-pdf-locale.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const PDF = 513;
const XLSX = 257;
const SWEDISH = 1053;

async function editorBridge(docType, locale, dialogAnswer) {
  const sent = { save_file_as: [], print_document: [] };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      current_document: () => ({ token: 'open-token', name: 'book.xlsx' }),
      pick_save_target: () => ({ token: 'save-token', name: dialogAnswer }),
      save_file_as: (args) => { sent.save_file_as.push(args); return 'ok'; },
      print_document: (args) => { sent.print_document.push(args); return null; },
      recovery_mark_saved: () => null,
      set_window_title: () => null,
      set_document_modified: () => null,
      open_pdf_viewer: () => null,
    },
  });
  bridge.sandbox.__TAURI__.dialog = { message: () => Promise.resolve() };
  bridge.sandbox.AscDesktopEditor._currentDocType = docType;
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin', asc_getLocale: () => locale } },
    DesktopOfflineAppDocumentEndSave() {},
  };
  return { bridge, sent };
}

async function exportArgs(docType, locale, dialogAnswer = 'book.pdf', fileType = PDF) {
  const run = await editorBridge(docType, locale, dialogAnswer);
  await run.bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${fileType}, '')`);
  run.bridge.stopTimers();
  return run.sent.save_file_as[0];
}

async function printArgs(docType, locale) {
  const run = await editorBridge(docType, locale, null);
  await run.bridge.run(`AscDesktopEditor.Print('')`);
  run.bridge.stopTimers();
  return run.sent.print_document[0];
}

function show(args) {
  return JSON.stringify(args);
}

let args = await exportArgs('cell', SWEDISH);
check(results, 'a spreadsheet PDF export sends the locale chosen in the editor',
  args && args.lcid === SWEDISH && args.token === 'save-token', show(args));

args = await printArgs('cell', SWEDISH);
check(results, 'a spreadsheet print sends the locale chosen in the editor',
  args && args.lcid === SWEDISH, show(args));

for (const docType of ['word', 'slide']) {
  args = await exportArgs(docType, SWEDISH);
  check(results, `a ${docType} PDF export sends no locale`,
    args && !('lcid' in args), show(args));
  args = await printArgs(docType, SWEDISH);
  check(results, `a ${docType} print sends no locale`,
    args && !('lcid' in args), show(args));
}

for (const locale of [null, 0, -1, '1053']) {
  args = await exportArgs('cell', locale);
  check(results, `a spreadsheet PDF export with locale ${show(locale)} sends no locale`,
    args && !('lcid' in args), show(args));
  args = await printArgs('cell', locale);
  check(results, `a spreadsheet print with locale ${show(locale)} sends no locale`,
    args && !('lcid' in args), show(args));
}

args = await exportArgs('cell', SWEDISH, 'book.xlsx', XLSX);
check(results, 'a spreadsheet Save As to XLSX sends no locale',
  args && !('lcid' in args), show(args));

finish(results);
