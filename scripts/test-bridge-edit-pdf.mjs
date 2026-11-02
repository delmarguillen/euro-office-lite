// Checks the start screen's Edit PDF: it asks pick_pdf for a file, opens the
// chosen token with open_pdf, and does nothing when the dialog is cancelled.
// The button's handler in editor-patches.js only calls _eoEditPdf, so this
// drives that bridge function.
//
// Run: node scripts/test-bridge-edit-pdf.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const PDF_BYTES = '%PDF-1.7\n%%EOF\n';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function clickEditPdf(picked) {
  const calls = [];
  const bridge = await loadBridge({
    invoke: {
      pick_pdf: (args) => { calls.push({ command: 'pick_pdf', args }); return picked; },
      open_pdf: (args) => { calls.push({ command: 'open_pdf', args }); return Buffer.from(PDF_BYTES, 'binary').toString('base64'); },
      open_file: (args) => { calls.push({ command: 'open_file', args }); return ''; },
    },
  });
  bridge.run(`
    var __editors = [];
    window._openEditor = function(docType) { __editors.push(docType); };
  `);
  await bridge.run('_eoEditPdf()');
  await sleep(50);
  bridge.stopTimers();
  return { calls, editors: Array.from(bridge.sandbox.__editors) };
}

let result = await clickEditPdf({ token: 'tok-1', name: 'report.pdf' });
const pick = result.calls[0];
check(results, 'Edit PDF asks pick_pdf for a file', pick && pick.command === 'pick_pdf', JSON.stringify(result.calls));
check(results, '... with a PDF filter and an all-files filter',
  pick && pick.args.pdfLabel === 'PDF' && pick.args.all === 'All', JSON.stringify(pick && pick.args));
check(results, '... and sends no path', pick && Object.keys(pick.args).every((key) => key === 'pdfLabel' || key === 'all'),
  JSON.stringify(pick && pick.args));
const open = result.calls[1];
check(results, 'the chosen file opens by token with open_pdf',
  open && open.command === 'open_pdf' && open.args.token === 'tok-1', JSON.stringify(result.calls));
check(results, '... in the PDF editor', result.editors.join() === 'pdf', result.editors.join());

result = await clickEditPdf(null);
check(results, 'cancelling opens nothing', result.calls.length === 1 && result.editors.length === 0, JSON.stringify(result.calls));

result = await clickEditPdf({ token: 'tok-2', name: 'notes.docx' });
check(results, 'a non-PDF chosen under All opens as Open would (open_file)',
  result.calls[1] && result.calls[1].command === 'open_file' && result.editors.join() === 'word', JSON.stringify(result.calls));

finish(results);
