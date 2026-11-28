// Checks scripts/check-budget.mjs: a measure within, over and without a
// budget, the median open of harness logs, and that ci/budgets.json holds
// only positive numbers.
//
// Run: node scripts/test-check-budget.mjs
import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { BUDGETS_FILE, budgetFor, check, completeOpenTotals, describe, verdict } from './check-budget.mjs';

const results = [];
function expect(label, ok, detail = '') {
  results.push(ok);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${detail ? '  (' + detail + ')' : ''}`);
}

// One open as the app log has it, from the request to document_ready.
function openLog(id, totalMs) {
  return [
    `[JS] [2026-11-22T10:00:00.000Z] [OPEN] phase=request open=${id} t=0 dt=0 wall=1795312800000`,
    `[OPEN] phase=command open=${id} t=0 dt=0 wall=1795312800001`,
    `[OPEN] phase=respond open=${id} t=50 dt=50`,
    `[JS] [2026-11-22T10:00:00.060Z] [OPEN] phase=result open=${id} t=60 dt=60`,
    `[JS] [2026-11-22T10:00:03.000Z] [OPEN] phase=document_ready open=${id} t=${totalMs} dt=${totalMs - 60}`,
  ].join('\n');
}

// An open the backend refused: it never reaches document_ready.
const REFUSED = [
  '[JS] [2026-11-22T10:00:00.000Z] [OPEN] phase=request open=mwr6aaaa-1 t=0 dt=0 wall=1795312800000',
  '[OPEN] phase=command open=mwr6aaaa-1 t=0 dt=0 wall=1795312800001',
  '[OPEN] phase=failed open=mwr6aaaa-1 t=2040 dt=2040',
].join('\n');

const BUDGETS = { size_bytes: { deb: 1000 }, open_ms: { 'harness-linux-aarch64': 3000 } };

expect('a key in the file has its budget', budgetFor(BUDGETS, 'size_bytes', 'deb') === 1000);
expect('a key the file does not have has none', budgetFor(BUDGETS, 'size_bytes', 'dmg') === null);
expect('a section the file does not have has none', budgetFor({}, 'open_ms', 'any') === null);
let refused = false;
try { budgetFor({ size_bytes: { deb: '1000' } }, 'size_bytes', 'deb'); } catch { refused = true; }
expect('a budget that is not a number is an error', refused);

expect('the budget itself passes', verdict(1000, 1000) === 'within');
expect('one over the budget fails', verdict(1001, 1000) === 'over');
expect('no budget passes', verdict(5000, null) === 'no budget');

const under = describe('size', 'deb', 900, 1000);
expect('the line gives the measure, the budget and the gap', under === 'size deb: 900 bytes, budget 1000 bytes (90.0% of it, 100 bytes under)', under);
const over = describe('open', 'harness-linux-aarch64', 3300, 3000);
expect('... and which side of the budget it is on', over.endsWith('(110.0% of it, 300 ms over)'), over);

const totals = completeOpenTotals([
  { name: 'a', text: openLog('mwr6bbbb-1', 3000) },
  { name: 'b', text: REFUSED },
  { name: 'c', text: openLog('mwr6cccc-1', 3400) },
]);
expect('only complete opens are counted', totals.join() === '3000,3400', totals.join());

const work = await mkdtemp(path.join(os.tmpdir(), 'check-budget-'));
try {
  const installer = path.join(work, 'app.deb');
  await writeFile(installer, Buffer.alloc(1001));
  const tooBig = await check('size', 'deb', installer, BUDGETS);
  expect('an installer over its budget is reported as over', tooBig.over, tooBig.line);
  const noBudget = await check('size', 'dmg', installer, BUDGETS);
  expect('an installer without a budget passes', !noBudget.over, noBudget.line);

  for (const [name, total] of [['one', 2900], ['two', 3050], ['three', 2950]]) {
    await mkdir(path.join(work, 'harness', name), { recursive: true });
    await writeFile(path.join(work, 'harness', name, 'app.log'), openLog(`mwr6${name}-1`, total));
  }
  await mkdir(path.join(work, 'harness', 'no-log'));
  const opens = await check('open', 'harness-linux-aarch64', path.join(work, 'harness'), BUDGETS);
  expect('the opens of a harness run give their median', opens.line.startsWith('open harness-linux-aarch64: 2950 ms'), opens.line);
  expect('... which passes under the budget', !opens.over);

  const script = path.join(path.dirname(fileURLToPath(import.meta.url)), 'check-budget.mjs');
  const run = (args) => promisify(execFile)(process.execPath, [script, ...args])
    .then(({ stdout }) => ({ code: 0, stdout }), (error) => ({ code: error.code, stdout: error.stdout }));
  const passing = await run(['size', 'not-a-key-of-the-file', installer]);
  expect('the command passes a key without a budget', passing.code === 0 && passing.stdout.includes('no budget'), passing.stdout.trim());
  const missing = await run(['size', 'deb', path.join(work, 'missing.deb')]);
  expect('the command exits 2 when the file is missing', missing.code === 2, String(missing.code));
  const usage = await run(['size']);
  expect('the command exits 2 without its arguments', usage.code === 2, String(usage.code));
} finally {
  await rm(work, { recursive: true, force: true });
}

const versioned = JSON.parse(await readFile(BUDGETS_FILE, 'utf8'));
const sections = Object.keys(versioned);
expect('ci/budgets.json has only the known sections', sections.every((name) => ['size_bytes', 'open_ms'].includes(name)), sections.join());
let positive = true;
for (const section of sections) {
  for (const key of Object.keys(versioned[section])) {
    try { budgetFor(versioned, section, key); } catch { positive = false; }
  }
}
expect('... and every budget in it is a positive number', positive);

const failed = results.filter((ok) => !ok).length;
console.log(`${results.length - failed} passed, ${failed} failed`);
process.exitCode = failed ? 1 : 0;
