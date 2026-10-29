// Checks how src/sdkjs/pdf/src/forms/formActions.js handles JavaScript actions
// from a PDF, with the sdkjs patches applied: the script is never run, and the
// action queue goes on to the next action as if the script had ended.
//
//   - Do (field and link actions run from the queue): a script that calls a
//     spy and writes a global does neither; the hide/show action after it runs.
//   - RunScript (Keystroke, Format, Validate, Calculate): the script is not
//     run and the event comes back with its defaults.
//
// The document, field and queue are stubs; the actions and triggers are the
// real ones from formActions.js.
//
// Run: node scripts/apply-patches.mjs apply --only sdkjs
//      node scripts/test-sdkjs-pdf-no-javascript.mjs [path/to/formActions.js]
//      node scripts/apply-patches.mjs revert --only sdkjs
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const defaultPath = fileURLToPath(new URL('../src/sdkjs/pdf/src/forms/formActions.js', import.meta.url));
const source = readFileSync(process.argv[2] || defaultPath, 'utf8');
const SCRIPT = 'pdfScriptSpy(); globalThis.pdfScriptRan = true; event.rc = false;';

function loadActions() {
  const context = { console: { log: () => {}, clear: () => {} }, spyCalls: 0 };
  context.window = context;
  context.pdfScriptSpy = () => { context.spyCalls++; };
  vm.createContext(context);
  vm.runInContext(source, context);
  // EvalScript, unpatched, reads these when it builds the script's arguments.
  context.AscPDF.Api = { Types: { color: {} }, Functions: {} };
  return context;
}

// Runs the queued actions one after another, like CActionQueue.Continue.
function makeQueue(actions) {
  const queue = { actions, curActionIdx: 0, finished: false };
  queue.SetCurAction = () => {};
  queue.Continue = () => {
    const next = queue.actions[queue.curActionIdx + 1];
    if (!next) {
      queue.finished = true;
      return;
    }
    queue.curActionIdx += 1;
    next.Do();
  };
  return queue;
}

function makeDoc() {
  const doc = { eventsStack: [], hideShowCalls: [], activeForm: null };
  doc.CreateEvent = (pr) => ({ name: pr.name, target: pr.target, rc: pr.rc, value: pr.value });
  doc.GetDocumentApi = () => ({});
  doc.HideShowForms = (hidden, names) => doc.hideShowCalls.push({ hidden, names });
  return doc;
}

function makeField(doc) {
  return { GetDocument: () => doc, GetFormApi: () => ({}) };
}

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok });
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : ` (${detail})`}`);
}

// Do: a MouseUp trigger with a script and then a hide/show action.
{
  const context = loadActions();
  const { AscPDF } = context;
  const doc = makeDoc();
  const script = new AscPDF.CActionRunScript(SCRIPT);
  const hideShow = new AscPDF.CActionHideShow(true, ['Field1']);
  const trigger = new AscPDF.CPdfTrigger(AscPDF.PDF_TRIGGERS_TYPES.MouseUp, [script, hideShow]);
  trigger.SetParentField(makeField(doc));
  const queue = makeQueue(trigger.Actions);
  doc.GetActionsQueue = () => queue;

  script.Do();

  check('Do: the script does not call the spy', context.spyCalls === 0, `${context.spyCalls} calls`);
  check('Do: the script does not write a global', context.pdfScriptRan === undefined,
    `pdfScriptRan = ${context.pdfScriptRan}`);
  check('Do: the next action in the queue runs',
    doc.hideShowCalls.length === 1 && doc.hideShowCalls[0].hidden === true,
    JSON.stringify(doc.hideShowCalls));
}

// RunScript: a Keystroke script, called directly as the text field does.
{
  const context = loadActions();
  const { AscPDF } = context;
  const doc = makeDoc();
  const script = new AscPDF.CActionRunScript(SCRIPT);
  const trigger = new AscPDF.CPdfTrigger(AscPDF.PDF_TRIGGERS_TYPES.Keystroke, [script]);
  trigger.SetParentField(makeField(doc));

  const event = script.RunScript({ name: 'Keystroke', target: {}, rc: true, value: 'typed' });

  check('RunScript: the script does not call the spy', context.spyCalls === 0, `${context.spyCalls} calls`);
  check('RunScript: the event keeps its defaults', event.rc === true && event.value === 'typed',
    JSON.stringify(event));
}

const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
if (failed > 0) {
  console.log('Are the sdkjs patches applied? node scripts/apply-patches.mjs apply --only sdkjs');
  process.exit(1);
}
