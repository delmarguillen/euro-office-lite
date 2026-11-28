// Fails a CI job when a measure is over the budget ci/budgets.json gives it.
//
//   node scripts/check-budget.mjs size <key> <installer file>
//   node scripts/check-budget.mjs open <key> <harness output folder>
//
// size  the bytes of the installer, against size_bytes.<key>.
// open  the median time of the complete opens in <folder>/*/app.log, in
//       milliseconds from the request to document_ready as
//       scripts/measure-open.mjs counts it, against open_ms.<key>.
//
// A budget is the largest value that passes. A key the file does not have is
// measured, printed and passes. To move a budget, change its number in
// ci/budgets.json in a commit.
//
// Exit status: 0 within the budget or no budget for the key, 1 over the
// budget, 2 the measure could not be taken.
import { readdir, readFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { measureOpen, median, parseLog } from './measure-open.mjs';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
export const BUDGETS_FILE = path.join(scriptDir, '..', 'ci', 'budgets.json');

// kind: the name on the command line, the section of the budgets file and
// the unit of its numbers.
const KINDS = {
  size: { section: 'size_bytes', unit: 'bytes', measure: installerBytes },
  open: { section: 'open_ms', unit: 'ms', measure: medianOpenMs },
};

async function installerBytes(file) {
  const info = await stat(file);
  if (!info.isFile()) throw new Error(`${file} is not a file`);
  return info.size;
}

// The app log of every harness case: <folder>/<case>/app.log.
async function caseLogs(folder) {
  const entries = await readdir(folder, { withFileTypes: true });
  const logs = [];
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const logPath = path.join(folder, entry.name, 'app.log');
    const text = await readFile(logPath, 'utf8').catch(() => null);
    if (text !== null) logs.push({ name: entry.name, text });
  }
  return logs;
}

export function completeOpenTotals(logs) {
  return logs
    .flatMap(({ name, text }) => parseLog(text, name))
    .map(measureOpen)
    .filter((row) => row.status === 'ok')
    .map((row) => row.total);
}

async function medianOpenMs(folder) {
  const totals = completeOpenTotals(await caseLogs(folder));
  if (!totals.length) throw new Error(`no complete open in ${folder}/*/app.log`);
  return Math.round(median(totals));
}

// The budget of a key, or null when the file has none for it.
export function budgetFor(budgets, section, key) {
  const value = budgets[section]?.[key];
  if (value === undefined) return null;
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(`${section}.${key} is not a positive number`);
  }
  return value;
}

export function verdict(measured, budget) {
  if (budget === null) return 'no budget';
  return measured > budget ? 'over' : 'within';
}

export function describe(kind, key, measured, budget) {
  const { unit } = KINDS[kind];
  const head = `${kind} ${key}: ${measured} ${unit}`;
  if (budget === null) return `${head}, no budget in ci/budgets.json`;
  const percent = ((100 * measured) / budget).toFixed(1);
  const gap = Math.abs(budget - measured);
  const side = measured > budget ? 'over' : 'under';
  return `${head}, budget ${budget} ${unit} (${percent}% of it, ${gap} ${unit} ${side})`;
}

// The line to print for one measure, and whether it is over its budget.
export async function check(kind, key, target, budgets) {
  const budget = budgetFor(budgets, KINDS[kind].section, key);
  const measured = await KINDS[kind].measure(target);
  const line = describe(kind, key, measured, budget);
  return { line, over: verdict(measured, budget) === 'over' };
}

async function main(args) {
  const [kind, key, target] = args;
  if (!KINDS[kind] || !key || !target) {
    console.error('usage: node scripts/check-budget.mjs size|open <key> <file or folder>');
    return 2;
  }
  let result;
  try {
    const budgets = JSON.parse(await readFile(BUDGETS_FILE, 'utf8'));
    result = await check(kind, key, target, budgets);
  } catch (error) {
    console.error(`check-budget: ${error.message}`);
    return 2;
  }
  if (!result.over) {
    console.log(result.line);
    return 0;
  }
  // An annotation on the run when this is a GitHub Actions job.
  console.log(`::error::${result.line}`);
  return 1;
}

const runAsScript = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (runAsScript) process.exitCode = await main(process.argv.slice(2));
