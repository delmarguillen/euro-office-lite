// Checks how the bridge opens a file by path: a .pdf is read by open_pdf and
// reaches the PDF editor as the file itself (bSerFormat false, no crash
// recovery), while a .docx still goes through open_file as an Editor.bin.
//
// Run: node scripts/test-bridge-pdf-open.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const PDF_BYTES = '%PDF-1.7\n%%EOF\n';
const BIN_BYTES = 'DOCY;v5;0;';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function openPath(filePath) {
  const calls = [];
  const bridge = await loadBridge({
    invoke: {
      open_pdf: (args) => { calls.push('open_pdf ' + args.path); return Buffer.from(PDF_BYTES, 'binary').toString('base64'); },
      open_file: (args) => { calls.push('open_file ' + args.path); return Buffer.from(BIN_BYTES, 'binary').toString('base64'); },
      recovery_begin: () => { calls.push('recovery_begin'); return null; },
    },
  });
  bridge.sandbox.__xhr = (url) => (url.endsWith('doc-key') ? { status: 200, body: '1000-1/2' } : null);
  bridge.run(`
    var __opened = [];
    var __editorDoc = { addEventListener: function() {} };
    window.AscDesktopEditor._editorWindow = {
      Asc: { editor: { openDocument: function(file) { __opened.push(file); } } },
      document: __editorDoc,
      AscCommon: {
        g_oDocumentUrls: { documentUrl: '', getImageUrl: function() {}, getUrl: function() {} },
        OpenFileResult: function() {},
        History: {},
        c_oAscClipboardDataFormat: { Text: 1, Html: 2, Internal: 4 },
        g_clipboardBase: { isUseNewCopy: function() { return false; } },
      },
    };
    var __editors = [];
    window._openEditor = function(docType) {
      __editors.push(docType);
      window.AscDesktopEditor._currentDocType = docType;
      window.AscDesktopEditor.LocalStartOpen();
    };
  `);
  await bridge.run(`_eoOpenFromPath(${JSON.stringify(filePath)})`);
  await sleep(150);
  bridge.stopTimers();
  const opened = bridge.sandbox.__opened[0];
  return {
    calls,
    editors: Array.from(bridge.sandbox.__editors),
    serFormat: opened && opened.bSerFormat,
    data: opened && Buffer.from(opened.data).toString('binary'),
  };
}

let result = await openPath('C:/docs/report.pdf');
check(results, 'a .pdf is read by open_pdf', result.calls[0] === 'open_pdf C:/docs/report.pdf', result.calls.join());
check(results, '... and goes to the PDF editor', result.editors.join() === 'pdf', result.editors.join());
check(results, '... with the bytes of the file', result.data === PDF_BYTES, JSON.stringify(result.data));
check(results, '... marked as not an Editor.bin', result.serFormat === false, String(result.serFormat));
check(results, '... and without a recovery session', !result.calls.includes('recovery_begin'), result.calls.join());

result = await openPath('/home/me/Report.PDF');
check(results, 'the extension is matched in any case', result.calls[0] === 'open_pdf /home/me/Report.PDF', result.calls.join());

result = await openPath('C:/docs/report.docx');
check(results, 'a .docx still goes through open_file', result.calls[0] === 'open_file C:/docs/report.docx', result.calls.join());
check(results, '... to the document editor', result.editors.join() === 'word', result.editors.join());
check(results, '... as an Editor.bin', result.serFormat === true && result.data === BIN_BYTES, String(result.serFormat));
check(results, '... with its recovery session', result.calls.includes('recovery_begin'), result.calls.join());

finish(results);
