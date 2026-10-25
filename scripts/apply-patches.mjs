// Applies the patches in patches/<module>/ to the submodule trees before they
// are built or staged. Each module's patches are one series, applied one file
// at a time in file-name order: a later patch may build on an earlier one in
// the same file, which a single `git apply` over all of them does not allow.
//
//   node scripts/apply-patches.mjs apply  [--only sdkjs,web-apps] [--record FILE]
//   node scripts/apply-patches.mjs revert [--only sdkjs,web-apps] [--record FILE]
//   node scripts/apply-patches.mjs status [--only sdkjs,web-apps]
//
// apply leaves a module that already carries its whole series as it is, and
// stops on a module whose series fits neither way, after taking back what it
// applied in that run. --record lists the modules a run applied, so
// `revert --record` takes back exactly those and leaves alone a tree that was
// already patched (by hand, for `tauri dev`). Without --record, revert takes
// back every series a tree carries.
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const MODULES = {
  'web-apps': { tree: 'src/web-apps', patches: 'patches/web-apps' },
  sdkjs: { tree: 'src/sdkjs', patches: 'patches/sdkjs' },
  // The sdkjs tag compile-doctrenderer-sdkjs.sh checks out for x2t: other
  // sources, so its own folder.
  'doctrenderer-sdkjs': { tree: 'src/sdkjs', patches: 'patches/doctrenderer-sdkjs' },
};
const DEFAULT_MODULES = ['sdkjs', 'web-apps'];

function report(message) {
  console.log(`[patches] ${message}`);
}

function fail(message) {
  // GitHub Actions shows ::error:: lines as annotations on the run.
  if (process.env.GITHUB_ACTIONS) console.log(`::error::${message}`);
  console.error(`[patches] ERROR: ${message}`);
  process.exit(1);
}

function patchFiles(moduleName) {
  const dir = path.join(projectRoot, MODULES[moduleName].patches);
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => name.endsWith('.patch'))
    .sort()
    .map((name) => path.join(dir, name));
}

function gitApply(moduleName, file, reverse) {
  const tree = path.join(projectRoot, MODULES[moduleName].tree);
  const args = ['-C', tree, 'apply', ...(reverse ? ['--reverse'] : []), file];
  const result = spawnSync('git', args, { encoding: 'utf8' });
  return { ok: result.status === 0, output: `${result.stderr || ''}${result.error || ''}`.trim() };
}

// Applies (or, reversed, takes back) the whole series or nothing: a patch that
// does not fit undoes the ones this call already went through.
function runSeries(moduleName, reverse) {
  const files = reverse ? [...patchFiles(moduleName)].reverse() : patchFiles(moduleName);
  const done = [];
  for (const file of files) {
    const result = gitApply(moduleName, file, reverse);
    if (!result.ok) {
      for (const undo of done.reverse()) gitApply(moduleName, undo, !reverse);
      return { ok: false, output: `${path.basename(file)}: ${result.output}` };
    }
    done.push(file);
  }
  return { ok: true, output: '' };
}

// 'none' (no patches), 'missing', 'applied' or 'conflict' (fits neither way).
// Found by trying, so the tree is put back as it was before returning.
function stateOf(moduleName) {
  if (patchFiles(moduleName).length === 0) return { state: 'none', output: '' };
  const forward = runSeries(moduleName, false);
  if (forward.ok) {
    runSeries(moduleName, true);
    return { state: 'missing', output: '' };
  }
  if (runSeries(moduleName, true).ok) {
    runSeries(moduleName, false);
    return { state: 'applied', output: '' };
  }
  return { state: 'conflict', output: forward.output };
}

function describe(moduleName) {
  const names = patchFiles(moduleName).map((file) => path.basename(file));
  return `${moduleName} (${names.join(', ')})`;
}

export function revertPatches(moduleNames) {
  for (const moduleName of [...moduleNames].reverse()) {
    if (patchFiles(moduleName).length === 0) continue;
    const reverted = runSeries(moduleName, true);
    if (reverted.ok) {
      report(`taken back: ${describe(moduleName)}`);
    } else if (runSeries(moduleName, false).ok) {
      // It applied forward, so the tree did not carry it: put it back out.
      runSeries(moduleName, true);
      report(`not in the tree, nothing to take back: ${describe(moduleName)}`);
    } else {
      fail(`could not take back ${describe(moduleName)}: ${reverted.output}`);
    }
  }
}

export function applyPatches(moduleNames = DEFAULT_MODULES) {
  const applied = [];
  for (const moduleName of moduleNames) {
    if (patchFiles(moduleName).length === 0) continue;
    const forward = runSeries(moduleName, false);
    if (forward.ok) {
      applied.push(moduleName);
      report(`applied: ${describe(moduleName)}`);
      continue;
    }
    if (runSeries(moduleName, true).ok) {
      runSeries(moduleName, false);
      report(`already in the tree: ${describe(moduleName)}`);
      continue;
    }
    revertPatches(applied);
    fail(`${describe(moduleName)} does not apply to ${MODULES[moduleName].tree}: ${forward.output}`);
  }
  return applied;
}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const options = { command, only: DEFAULT_MODULES, record: null };
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === '--only') options.only = (rest[++i] || '').split(',').filter(Boolean);
    else if (rest[i] === '--record') options.record = rest[++i];
    else fail(`unknown argument ${rest[i]}`);
  }
  const unknown = options.only.filter((name) => !MODULES[name]);
  if (unknown.length) fail(`unknown module ${unknown.join(', ')}`);
  return options;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.command === 'apply') {
    // Written first: a failed run must not leave an older record to revert.
    if (options.record) writeFileSync(options.record, '[]\n');
    const applied = applyPatches(options.only);
    if (options.record) writeFileSync(options.record, `${JSON.stringify(applied)}\n`);
  } else if (options.command === 'revert') {
    if (!options.record) {
      revertPatches(options.only);
    } else if (existsSync(options.record)) {
      // Empty when the caller made the file and apply never ran.
      const recorded = readFileSync(options.record, 'utf8').trim();
      revertPatches(recorded ? JSON.parse(recorded) : []);
      writeFileSync(options.record, '[]\n');
    }
  } else if (options.command === 'status') {
    for (const moduleName of options.only) {
      const { state, output } = stateOf(moduleName);
      report(`${state}: ${describe(moduleName)}${output ? ` (${output})` : ''}`);
    }
  } else {
    fail('usage: apply-patches.mjs apply|revert|status [--only MODULES] [--record FILE]');
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
