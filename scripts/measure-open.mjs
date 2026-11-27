// Turns the phase marks of the app log into one table row per document open.
//
//   node scripts/measure-open.mjs <log> [<log> ...]
//
// The marks are the lines
//
//   [OPEN] phase=<name> open=<id> t=<ms> dt=<ms> [wall=<ms since the Unix epoch>]
//
// written by the backend (src-tauri/src/open_phases.rs) and by the page
// (src/bridge.js, _eoOpenPhases) under the same open=<id>. Any text before
// "[OPEN]" on the line is ignored, and so is every other line.
//
// Columns: the time each phase took, in milliseconds, counted from the mark
// before it in time.
//
//   command         backend  the request reached the open command
//   start           backend  previous document closed, format detected
//   copy            backend  private copy for x2t written
//   convert         backend  x2t finished
//   encode          backend  result read and encoded as base64
//   respond         backend  the command answered (for a failed open: gave up)
//   result          page     the answer reached the page
//   fonts           page     font cache settled, system font list fetched
//   iframe          page     editor iframe created
//   sdk_min         page     min part of sdkjs evaluated (sdk-all-min.js in a release)
//   sdk_all         page     rest of sdkjs evaluated (sdk-all.js in a release)
//   app_ready       page     onAppReady
//   doc_info        page     sdkjs has the document's info, asks for its permissions
//   doc_load        page     editor UI set for those permissions (asc_LoadDocument)
//   doc_request     page     sdkjs asked for the document
//   doc_decoded     page     base64 of the document decoded into bytes
//   doc_parsed      page     sdkjs read the document (openDocument returned)
//   doc_fonts       page     fonts of the document loaded
//   doc_images      page     images of the document loaded; "-" without images
//   content_ready   page     sdkjs about to send asc_onDocumentContentReady
//   ui_ready        page     the handlers of that event returned
//   layout          page     the task that sent it ended
//   microtasks      page     microtasks queued ahead of the language list ran
//   spell_init      page     editor UI took the spellcheck languages
//   frame           page     first animation frame after that task began
//   doc_drawn       page     sdkjs's paint loop ran in that frame
//   document_ready  page     onDocumentReady reached the page
//   sum                      the phase columns of the row added up
//   total                    from the request to document_ready; "-" if not reached
//   status                   ok, failed (the backend refused) or partial (the marks stop)
//
// The columns keep this order whatever the order of the marks. sdkjs loads
// the rest of itself while the editor UI starts, so sdk_all can come after
// app_ready: its column is then the time since app_ready, and sdk_min +
// app_ready + sdk_all is the time from sdk_min to the later of the two.
//
// A phase with no mark shows "-", and its time goes to the next phase that
// has one. With two or more complete opens a last row gives the median of
// each column over them.
//
// The two clocks
//
// The backend's t counts from its `command` mark on a monotonic clock
// (Instant); the page's t counts from its `request` mark on performance.now().
// Both marks also log the wall clock, and wall(command) - wall(request) is
// where the backend's zero falls on the page's clock. That offset is the
// `command` column. Only this one pair of wall-clock readings is used, so a
// clock step during the open moves nothing else. The offset cannot be
// negative, nor larger than the page's `result` minus the backend's `respond`
// (the answer cannot arrive before it is sent): outside that range it is
// brought to the nearest bound and the row is listed under the table.
//
// Measuring on a Raspberry Pi
//
// The app prints its log to standard output. Start it from a terminal with
// the document as its argument and the output redirected to a file, wait for
// the document to show, close the app. One log per run:
//
//   <app> <document> > cold-1.log 2>&1
//
//   Cold, 5 runs: before each start, empty the page cache:
//       sync; echo 3 | sudo tee /proc/sys/vm/drop_caches
//   Warm, 5 runs: start again without emptying it. Leave out the first run
//       after a cold one if it was not measured as cold.
//
// Alongside, to tell a slow phase from a machine short of memory:
//
//   vmstat 1 > vmstat.log      in a second terminal for the whole session:
//                              si/so (swapping), wa (waiting for the disk), free
//   cat /proc/pressure/memory  before and after each run: the "some" and
//                              "full" totals grow by the time stalled on memory
//   smem -k                    with the document open: PSS of the app, of the
//                              web processes and, during `convert`, of x2t
//
// Then:
//
//   node scripts/measure-open.mjs cold-*.log
//   node scripts/measure-open.mjs warm-*.log
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const BACKEND_PHASES = ['command', 'start', 'copy', 'convert', 'encode', 'respond'];
const PAGE_PHASES = [
  'request', 'result', 'fonts', 'iframe', 'sdk_min', 'sdk_all', 'app_ready',
  'doc_info', 'doc_load', 'doc_request', 'doc_decoded', 'doc_parsed', 'doc_fonts', 'doc_images',
  'content_ready', 'ui_ready', 'layout', 'microtasks', 'spell_init',
  'frame', 'doc_drawn', 'document_ready',
];
// In the order an open goes through them. `request` is the origin, so it has
// no column of its own.
const TIMELINE = ['request', ...BACKEND_PHASES, ...PAGE_PHASES.slice(1)];
const COLUMNS = TIMELINE.slice(1);
// What the backend marks instead of `respond` when the open fails.
const FAILED = 'failed';

const MARK = /\[OPEN\] phase=(\S+) open=(\S+) t=(\d+) dt=(\d+)(?: wall=(\d+))?\s*$/;

// The marks of every open in a log, in the order their first mark appears.
// A phase logged twice for one open keeps its first mark.
export function parseLog(text, logName) {
  const opens = new Map();
  for (const line of text.split(/\r?\n/)) {
    const match = MARK.exec(line);
    if (!match) continue;
    const [, phase, id, t, , wall] = match;
    if (!opens.has(id)) opens.set(id, { log: logName, id, marks: {} });
    const marks = opens.get(id).marks;
    if (phase in marks) continue;
    marks[phase] = { t: Number(t), wall: wall === undefined ? null : Number(wall) };
  }
  return [...opens.values()];
}

// Where the backend's t=0 falls on the page's clock, and whether the wall
// clocks had to be corrected to get there.
export function backendOffset(marks) {
  const { request, command, result, respond } = marks;
  if (!request || !command) return { offset: 0, corrected: false };
  if (request.wall === null || command.wall === null) return { offset: 0, corrected: true };

  const measured = command.wall - request.wall;
  const answered = result && respond;
  const upper = answered ? Math.max(0, result.t - respond.t) : Infinity;
  const offset = Math.min(Math.max(measured, 0), upper);
  return { offset, corrected: offset !== measured };
}

// The marks of an open on one clock, sorted by time. The backend's `failed`
// stands where its `respond` would.
function timeline(marks, offset) {
  const points = [];
  for (const phase of TIMELINE) {
    const mark = phase === 'respond' ? (marks.respond || marks[FAILED]) : marks[phase];
    if (!mark) continue;
    const onBackend = BACKEND_PHASES.includes(phase);
    points.push({ phase, t: onBackend ? mark.t + offset : mark.t });
  }
  // Stable: marks at the same millisecond keep the order of TIMELINE.
  return points.sort((a, b) => a.t - b.t);
}

function statusOf(marks) {
  if (marks.document_ready) return 'ok';
  if (marks[FAILED]) return FAILED;
  return 'partial';
}

// One row of the table: the time each phase took, counted from the mark
// before it in time.
export function measureOpen(open) {
  const { offset, corrected } = backendOffset(open.marks);
  const points = timeline(open.marks, offset);
  const durations = {};
  let sum = 0;
  for (let index = 1; index < points.length; index++) {
    const duration = points[index].t - points[index - 1].t;
    durations[points[index].phase] = duration;
    sum += duration;
  }
  // A backend-only log has no request: its origin is `command`.
  if (points.length && points[0].phase === 'command') durations.command = 0;

  const status = statusOf(open.marks);
  const first = points.length ? points[0].t : 0;
  const total = status === 'ok' ? open.marks.document_ready.t - first : null;
  return { log: open.log, id: open.id, durations, sum, total, status, corrected };
}

export function median(values) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  if (sorted.length % 2) return sorted[middle];
  return (sorted[middle - 1] + sorted[middle]) / 2;
}

// The medians over the complete opens, or null with fewer than two.
function medianRow(rows) {
  const complete = rows.filter((row) => row.status === 'ok');
  if (complete.length < 2) return null;
  const durations = {};
  for (const phase of COLUMNS) {
    const values = complete.map((row) => row.durations[phase]).filter((value) => value !== undefined);
    const value = median(values);
    if (value !== null) durations[phase] = value;
  }
  return {
    log: `median (n=${complete.length})`,
    id: '',
    durations,
    sum: median(complete.map((row) => row.sum)),
    total: median(complete.map((row) => row.total)),
    status: '',
  };
}

function cell(value) {
  if (value === undefined || value === null) return '-';
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

function cellsOf(row) {
  const phases = COLUMNS.map((phase) => cell(row.durations[phase]));
  return [row.log, row.id, ...phases, cell(row.sum), cell(row.total), row.status];
}

// Text columns: the two names to the left, the numbers to the right.
export function formatTable(rows) {
  const header = ['log', 'open', ...COLUMNS, 'sum', 'total', 'status'];
  const body = rows.map(cellsOf);
  const medians = medianRow(rows);
  if (medians) body.push(cellsOf(medians));

  const lines = [header, ...body];
  const widths = header.map((_, column) => Math.max(...lines.map((line) => line[column].length)));
  const leftAligned = (column) => column < 2 || column === header.length - 1;
  const format = (line) => line
    .map((text, column) => (leftAligned(column) ? text.padEnd(widths[column]) : text.padStart(widths[column])))
    .join('  ')
    .trimEnd();
  return lines.map(format).join('\n');
}

function correctionNotes(rows) {
  return rows
    .filter((row) => row.corrected)
    .map((row) => `clocks: ${row.log} ${row.id}: the wall clocks do not place the backend inside the request; offset corrected`);
}

export function report(logs) {
  const rows = logs.flatMap(({ name, text }) => parseLog(text, name)).map(measureOpen);
  if (!rows.length) return 'no [OPEN] phase marks found';
  return [formatTable(rows), ...correctionNotes(rows)].join('\n');
}

async function main(paths) {
  if (!paths.length) {
    console.error('usage: node scripts/measure-open.mjs <log> [<log> ...]');
    return 2;
  }
  const logs = [];
  for (const logPath of paths) {
    try {
      logs.push({ name: path.basename(logPath), text: await readFile(logPath, 'utf8') });
    } catch (error) {
      console.error(`cannot read ${logPath}: ${error.message}`);
      return 1;
    }
  }
  console.log(report(logs));
  return 0;
}

const runAsScript = process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href;
if (runAsScript) process.exitCode = await main(process.argv.slice(2));
