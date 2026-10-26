// Checks the Save As that follows a recovery, when the dialog starts from the
// recovered file's old path: an export starts as that name in the format asked
// for, so accepting it never writes over the original. A name the user types
// is saved as typed, as in any Save As.
//
// Run: node scripts/test-bridge-save-suggestion.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const PDF = 513;
const DOCX = 65;

function withExt(bridge, path, ext) {
  if (bridge.run('typeof _eoSuggestionWithExt') !== 'function') return 'missing';
  return bridge.run(`_eoSuggestionWithExt(${JSON.stringify(path)}, ${JSON.stringify(ext)})`);
}

// One LocalFileSave with the dialog answering `answer(defaultPath)`.
async function saveAs({ currentPath, suggestion, fileType, answer }) {
  const calls = { savedTo: [], markedSaved: 0, dialogDefault: undefined };
  const bridge = await loadBridge({
    invoke: {
      write_editor_bin: () => 'ok',
      get_current_path: () => currentPath,
      get_save_suggestion: () => suggestion,
      save_file_as: (args) => { calls.savedTo.push(args.path); return 'ok'; },
      recovery_mark_saved: () => { calls.markedSaved += 1; return null; },
      set_window_title: () => null,
    },
  });
  bridge.sandbox.__TAURI__.dialog = {
    save: (options) => {
      calls.dialogDefault = options.defaultPath;
      return Promise.resolve(answer(options.defaultPath));
    },
    message: () => Promise.resolve(),
  };
  bridge.sandbox.AscDesktopEditor._editorWindow = {
    Asc: { editor: { asc_nativeGetFile: () => 'bin' } },
    DesktopOfflineAppDocumentEndSave: () => {},
  };
  await bridge.run(`AscDesktopEditor.LocalFileSave('saveas=true', '', null, ${fileType}, '')`);
  bridge.stopTimers();
  return { bridge, calls };
}

const pure = await loadBridge();
check(results, 'the suggestion takes the extension asked for',
  withExt(pure, 'C:/docs/report.docx', 'pdf') === 'C:/docs/report.pdf',
  withExt(pure, 'C:/docs/report.docx', 'pdf'));
check(results, '... on a Windows path too',
  withExt(pure, 'C:\\docs\\report.v2.docx', 'pdf') === 'C:\\docs\\report.v2.pdf',
  withExt(pure, 'C:\\docs\\report.v2.docx', 'pdf'));
check(results, '... added when the name has none, even under a dotted folder',
  withExt(pure, '/home/u/my.docs/report', 'odt') === '/home/u/my.docs/report.odt',
  withExt(pure, '/home/u/my.docs/report', 'odt'));
check(results, '... and nothing changes without a path or an extension',
  withExt(pure, null, 'pdf') === null && withExt(pure, '/a/b.docx', null) === '/a/b.docx');

const keepName = (defaultPath) => defaultPath;
let run = await saveAs({ currentPath: null, suggestion: 'C:/docs/report.docx', fileType: PDF, answer: keepName });
check(results, 'a PDF export after a recovery opens the dialog on report.pdf',
  run.calls.dialogDefault === 'C:/docs/report.pdf', String(run.calls.dialogDefault));
check(results, '... accepting the name writes report.pdf, not report.docx',
  run.calls.savedTo.join() === 'C:/docs/report.pdf', run.calls.savedTo.join());
check(results, '... and does not count as a save of the document',
  run.calls.markedSaved === 0, String(run.calls.markedSaved));

run = await saveAs({
  currentPath: null, suggestion: 'C:/docs/report.docx', fileType: PDF,
  answer: () => 'C:/docs/budget.docx',
});
check(results, 'a name typed in a PDF export after a recovery is saved as typed, as in 0.17.41',
  run.calls.savedTo.join() === 'C:/docs/budget.docx', run.calls.savedTo.join());

run = await saveAs({ currentPath: null, suggestion: 'C:/docs/report.docx', fileType: DOCX, answer: keepName });
check(results, 'a Save As after a recovery still offers the old file',
  run.calls.dialogDefault === 'C:/docs/report.docx' && run.calls.savedTo.join() === 'C:/docs/report.docx'
    && run.calls.markedSaved === 1,
  run.calls.savedTo.join());

run = await saveAs({
  currentPath: 'C:/docs/report.docx', suggestion: null, fileType: PDF,
  answer: () => 'C:/docs/other.odt',
});
check(results, 'a plain Save As keeps the extension the user typed',
  run.calls.dialogDefault === undefined && run.calls.savedTo.join() === 'C:/docs/other.odt',
  `${run.calls.dialogDefault} ${run.calls.savedTo.join()}`);

finish(results);
