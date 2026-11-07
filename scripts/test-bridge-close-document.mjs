// Checks that leaving a document for the start screen tells the backend the
// document is closed, after the session ends and before the page reloads, and
// that no failure on the way stops the reload.
//
// Run: node scripts/test-bridge-close-document.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const CHAIN = ['recovery_end', 'set_document_modified', 'close_document'];
const EXPECTED = [...CHAIN, 'reload'].join(' ');

const HOME = ['title:button', JSON.stringify({ click: 'home' })];
const FOLDER = ['go:folder', ''];
const FILE_CLOSE = ['editor:event', JSON.stringify({ action: 'file:close' })];

function step(steps, command, failing) {
  return () => {
    steps.push(command);
    if (failing.includes(command)) throw new Error(`${command} failed`);
    return null;
  };
}

async function leaveDocument({ entry = FOLDER, failing = [] } = {}) {
  const steps = [];
  const invoke = {};
  for (const command of CHAIN) invoke[command] = step(steps, command, failing);

  const bridge = await loadBridge({ invoke });
  bridge.sandbox.location = { reload: () => steps.push('reload') };
  bridge.sandbox.AscDesktopEditor._isModified = false;
  bridge.sandbox.AscDesktopEditor.execCommand(...entry);

  await new Promise((resolve) => setTimeout(resolve, 20));
  bridge.stopTimers();
  return steps.join(' ');
}

{
  const steps = await leaveDocument();
  check(results, 'Closes the document last, right before the reload', steps === EXPECTED, steps);
}

for (const [label, entry] of [['Home', HOME], ['File > Close', FILE_CLOSE]]) {
  const steps = await leaveDocument({ entry });
  check(results, `${label} goes through the same steps`, steps === EXPECTED, steps);
}

for (const command of CHAIN) {
  const steps = await leaveDocument({ failing: [command] });
  check(results, `A failing ${command} stops nothing`, steps === EXPECTED, steps);
}

{
  const steps = await leaveDocument({ failing: CHAIN });
  check(results, 'The page reloads even if every step fails', steps === EXPECTED, steps);
}

finish(results);
