// Checks scripts/measure-open.mjs on sample logs: two complete opens, one the
// backend refused and one whose marks stop half way, against the table they
// must give.
//
// Run: node scripts/test-measure-open.mjs
import { execFile } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { backendOffset, measureOpen, median, parseLog, report } from './measure-open.mjs';

const results = [];
function check(label, ok, detail = '') {
  results.push(ok);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}${detail ? '  (' + detail + ')' : ''}`);
}

// As the log file has it: the backend's lines bare, the page's after the time
// _eoLog puts first. A refused open, then a complete one.
const RUN_1 = `[START] instance=1700000000000-1
[2026-11-22T10:00:00.000Z] [OPEN] phase=request open=mwr6aaaa-1 t=0 dt=0 wall=1795312800000
[OPEN] phase=command open=mwr6aaaa-1 t=0 dt=0 wall=1795312800003
[OPEN] start file=broken.docx format=65
[OPEN] phase=start open=mwr6aaaa-1 t=10 dt=10
[OPEN] phase=copy open=mwr6aaaa-1 t=40 dt=30
[OPEN] failed file=broken.docx error=x2t exit code 80
[OPEN] phase=failed open=mwr6aaaa-1 t=2040 dt=2000
[2026-11-22T10:00:02.050Z] [EO] Error opening file: x2t exit code 80
[2026-11-22T10:00:10.000Z] [OPEN] phase=request open=mwr6hjgo-2 t=0 dt=0 wall=1795312810000
[OPEN] phase=command open=mwr6hjgo-2 t=0 dt=0 wall=1795312810002
[OPEN] start file=report.docx format=65
[OPEN] phase=start open=mwr6hjgo-2 t=15 dt=15
[OPEN] phase=copy open=mwr6hjgo-2 t=55 dt=40
[OPEN] phase=convert open=mwr6hjgo-2 t=8255 dt=8200
[OPEN] success editor_bin=1048576 bytes media_files=3
[OPEN] phase=encode open=mwr6hjgo-2 t=8375 dt=120
[OPEN] phase=respond open=mwr6hjgo-2 t=8378 dt=3
[2026-11-22T10:00:19.880Z] [OPEN] phase=result open=mwr6hjgo-2 t=9880 dt=9880
[2026-11-22T10:00:19.900Z] [OPEN] phase=fonts open=mwr6hjgo-2 t=9900 dt=20
[2026-11-22T10:00:19.935Z] [OPEN] phase=iframe open=mwr6hjgo-2 t=9935 dt=35
[2026-11-22T10:00:19.940Z] [EO] iframe diagnostics installed: source=about:blank id=frameEditor
[2026-11-22T10:00:28.935Z] [OPEN] phase=sdk_min open=mwr6hjgo-2 t=18935 dt=9000
[2026-11-22T10:00:34.935Z] [OPEN] phase=sdk_all open=mwr6hjgo-2 t=24935 dt=6000
[2026-11-22T10:00:38.935Z] [OPEN] phase=app_ready open=mwr6hjgo-2 t=28935 dt=4000
[2026-11-22T10:00:45.935Z] [OPEN] phase=document_ready open=mwr6hjgo-2 t=35935 dt=7000
[2026-11-22T10:00:45.936Z] [EO] Document ready: docType=word
`;

// As standard output has it, with Windows line ends: the page's lines after
// "[JS] ". A complete open whose page lines arrive before the backend's, and
// whose onAppReady comes before sdk-all.js has loaded.
const RUN_2 = [
  '[JS] [2026-11-22T10:05:00.000Z] [OPEN] phase=request open=mwr7k2aa-1 t=0 dt=0 wall=1795313100000',
  '[JS] [2026-11-22T10:05:04.608Z] [OPEN] phase=result open=mwr7k2aa-1 t=4608 dt=4608',
  '[OPEN] phase=command open=mwr7k2aa-1 t=0 dt=0 wall=1795313100001',
  '[OPEN] start file=report.docx format=65',
  '[OPEN] phase=start open=mwr7k2aa-1 t=5 dt=5',
  '[OPEN] phase=copy open=mwr7k2aa-1 t=25 dt=20',
  '[OPEN] phase=convert open=mwr7k2aa-1 t=4025 dt=4000',
  '[OPEN] phase=encode open=mwr7k2aa-1 t=4105 dt=80',
  '[OPEN] phase=respond open=mwr7k2aa-1 t=4107 dt=2',
  '[JS] [2026-11-22T10:05:04.618Z] [OPEN] phase=fonts open=mwr7k2aa-1 t=4618 dt=10',
  '[JS] [2026-11-22T10:05:04.638Z] [OPEN] phase=iframe open=mwr7k2aa-1 t=4638 dt=20',
  '[JS] [2026-11-22T10:05:07.638Z] [OPEN] phase=sdk_min open=mwr7k2aa-1 t=7638 dt=3000',
  '[JS] [2026-11-22T10:05:09.638Z] [OPEN] phase=app_ready open=mwr7k2aa-1 t=9638 dt=2000',
  '[JS] [2026-11-22T10:05:10.138Z] [OPEN] phase=sdk_all open=mwr7k2aa-1 t=10138 dt=500',
  '[JS] [2026-11-22T10:05:13.138Z] [OPEN] phase=document_ready open=mwr7k2aa-1 t=13138 dt=3000',
  '',
].join('\r\n');

// The marks stop at the iframe, and the wall clock went back 4 ms between the
// page's reading and the backend's.
const RUN_3 = `[2026-11-22T10:10:00.000Z] [OPEN] phase=request open=mwr8zzzz-1 t=0 dt=0 wall=1795313400000
[OPEN] phase=command open=mwr8zzzz-1 t=0 dt=0 wall=1795313399996
[OPEN] phase=start open=mwr8zzzz-1 t=8 dt=8
[OPEN] phase=copy open=mwr8zzzz-1 t=30 dt=22
[OPEN] phase=convert open=mwr8zzzz-1 t=5030 dt=5000
[OPEN] phase=encode open=mwr8zzzz-1 t=5130 dt=100
[OPEN] phase=respond open=mwr8zzzz-1 t=5132 dt=2
[2026-11-22T10:10:05.800Z] [OPEN] phase=result open=mwr8zzzz-1 t=5800 dt=5800
[2026-11-22T10:10:05.805Z] [OPEN] phase=fonts open=mwr8zzzz-1 t=5805 dt=5
[2026-11-22T10:10:05.835Z] [OPEN] phase=iframe open=mwr8zzzz-1 t=5835 dt=30
`;

const EXPECTED = `log           open        command  start  copy  convert  encode  respond  result  fonts  iframe  sdk_min  sdk_all  app_ready  document_ready      sum    total  status
run-1.log     mwr6aaaa-1        3     10    30        -       -     2000       -      -       -        -        -          -               -     2043        -  failed
run-1.log     mwr6hjgo-2        2     15    40     8200     120        3    1500     20      35     9000     6000       4000            7000    35935    35935  ok
run-2.log     mwr7k2aa-1        1      5    20     4000      80        2     500     10      20     3000      500       2000            3000    13138    13138  ok
run-3.log     mwr8zzzz-1        0      8    22     5000     100        2     668      5      30        -        -          -               -     5835        -  partial
median (n=2)                  1.5     10    30     6100     100      2.5    1000     15    27.5     6000     3250       3000            5000  24536.5  24536.5
clocks: run-3.log mwr8zzzz-1: the wall clocks do not place the backend inside the request; offset corrected
`;

// The script as it is run, on files.
{
  const dir = await mkdtemp(path.join(os.tmpdir(), 'measure-open-'));
  try {
    const logs = { 'run-1.log': RUN_1, 'run-2.log': RUN_2, 'run-3.log': RUN_3 };
    const paths = [];
    for (const [name, text] of Object.entries(logs)) {
      paths.push(path.join(dir, name));
      await writeFile(paths[paths.length - 1], text);
    }
    const script = path.join(path.dirname(fileURLToPath(import.meta.url)), 'measure-open.mjs');
    const run = promisify(execFile);
    const { stdout } = await run(process.execPath, [script, ...paths]);
    check('the sample logs give the expected table', stdout === EXPECTED, stdout === EXPECTED ? '' : '\n' + stdout);

    const missing = await run(process.execPath, [script, path.join(dir, 'none.log')]).catch((error) => error);
    check('a log that cannot be read ends with an error', missing.code === 1 && /cannot read/.test(missing.stderr),
      `code=${missing.code}`);
    const noArguments = await run(process.execPath, [script]).catch((error) => error);
    check('no log at all prints the usage', noArguments.code === 2 && /usage:/.test(noArguments.stderr),
      `code=${noArguments.code}`);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

// Each complete row adds up to its total.
{
  const rows = [...parseLog(RUN_1, 'run-1.log'), ...parseLog(RUN_2, 'run-2.log')].map(measureOpen);
  const complete = rows.filter((row) => row.status === 'ok');
  check('the phases of a complete open add up to its total',
    complete.length === 2 && complete.every((row) => row.sum === row.total),
    complete.map((row) => `${row.sum}/${row.total}`).join());
}

// Where the backend's zero falls on the page's clock.
{
  const marks = (requestWall, commandWall) => ({
    request: { t: 0, wall: requestWall },
    command: { t: 0, wall: commandWall },
    respond: { t: 100, wall: null },
    result: { t: 130, wall: null },
  });
  const same = (actual, offset, corrected) => actual.offset === offset && actual.corrected === corrected;
  check('the offset is the difference of the two wall clocks', same(backendOffset(marks(5000, 5012)), 12, false));
  check('an offset before the request is brought to 0', same(backendOffset(marks(5000, 4990)), 0, true));
  check('an offset past the answer is brought to result - respond', same(backendOffset(marks(5000, 5900)), 30, true));
  check('without the page there is nothing to align',
    same(backendOffset({ command: { t: 0, wall: 5000 } }), 0, false));
}

// A log with nothing to measure.
{
  check('a log without marks says so',
    report([{ name: 'empty.log', text: '[OPEN] start file=a.docx format=65\n[OPEN] success editor_bin=10 bytes media_files=0\n' }])
      === 'no [OPEN] phase marks found');
  check('the median of an even count is the mean of the middle two',
    median([4, 1, 3, 2]) === 2.5 && median([3, 1, 2]) === 2 && median([]) === null);
}

const failed = results.filter((ok) => !ok).length;
console.log(`${results.length - failed} passed, ${failed} failed`);
process.exitCode = failed ? 1 : 0;
