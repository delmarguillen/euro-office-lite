// Checks scripts/check-budget.mjs: a measure within, over and without a
// budget, and that ci/budgets.json holds only positive numbers.
//
// Run: node scripts/test-check-budget.mjs
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { BUDGETS_FILE, budgetFor, check, describe, verdict } from './check-budget.mjs';

const results = [];
function expect(label, ok, detail = '') {
  results.push(ok);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${detail ? '  (' + detail + ')' : ''}`);
}

const BUDGETS = { size_bytes: { deb: 1000 } };

expect('a key in the file has its budget', budgetFor(BUDGETS, 'size_bytes', 'deb') === 1000);
expect('a key the file does not have has none', budgetFor(BUDGETS, 'size_bytes', 'dmg') === null);
expect('a section the file does not have has none', budgetFor({}, 'size_bytes', 'any') === null);
let refused = false;
try { budgetFor({ size_bytes: { deb: '1000' } }, 'size_bytes', 'deb'); } catch { refused = true; }
expect('a budget that is not a number is an error', refused);

expect('the budget itself passes', verdict(1000, 1000) === 'within');
expect('one over the budget fails', verdict(1001, 1000) === 'over');
expect('no budget passes', verdict(5000, null) === 'no budget');

const under = describe('size', 'deb', 900, 1000);
expect('the line gives the measure, the budget and the gap', under === 'size deb: 900 bytes, budget 1000 bytes (90.0% of it, 100 bytes under)', under);
const over = describe('size', 'deb', 1100, 1000);
expect('... and which side of the budget it is on', over.endsWith('(110.0% of it, 100 bytes over)'), over);

const work = await mkdtemp(path.join(os.tmpdir(), 'check-budget-'));
try {
  const installer = path.join(work, 'app.deb');
  await writeFile(installer, Buffer.alloc(1001));
  const tooBig = await check('size', 'deb', installer, BUDGETS);
  expect('an installer over its budget is reported as over', tooBig.over, tooBig.line);
  const noBudget = await check('size', 'dmg', installer, BUDGETS);
  expect('an installer without a budget passes', !noBudget.over, noBudget.line);

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
expect('ci/budgets.json has only the known sections', sections.every((name) => ['size_bytes'].includes(name)), sections.join());
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
