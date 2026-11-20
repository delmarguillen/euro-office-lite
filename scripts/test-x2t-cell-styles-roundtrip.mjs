// Round trip of edited cell styles through the real x2t, the way the app saves
// and opens a spreadsheet: x2t converts the file to Editor.bin (8192), the
// editor opens it, asc_nativeGetFile gives the Editor.bin that x2t converts to
// xlsx (257) or ods (259), and that file is opened again the same way.
//
// The editor runs in a node vm as in scripts/test-sdkjs-cell-styles.mjs (same
// stubs; the loader is copied from there). On a workbook converted from
// src-tauri/templates/blank.xlsx it creates a style from a selection, modifies
// a predefined style and duplicates another, each with font, fill, border,
// number format and alignment, applies them to cells and saves. Every reopened
// workbook is compared with the one that was saved: the styles of the
// document, the predefined styles and the cells (value, formula, style and
// formatting).
//
// A second workbook undoes a new style after a predefined one came in, which
// leaves an XfId unused; its xlsx must point only to cellStyleXfs it has. The
// workbook opened from the saved xlsx, without that gap, must be written with
// the same bytes as when each XfId is written as it is.
//
// Needs x2t 10.0.0.169 (scripts/get-x2t-ci.sh) and the sdkjs patches:
//   node scripts/apply-patches.mjs apply --only sdkjs
//   node scripts/test-x2t-cell-styles-roundtrip.mjs path/to/x2t-folder [path/to/sdkjs]
// The x2t folder holds x2t and its libraries. Exit status 1 when something
// differs after a round trip.
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const projectRoot = fileURLToPath(new URL('../', import.meta.url));
const x2tDir = process.argv[2];
const sdkjsRoot = process.argv[3] || path.join(projectRoot, 'src/sdkjs');
if (!x2tDir) {
  console.log('Usage: node scripts/test-x2t-cell-styles-roundtrip.mjs path/to/x2t-folder [path/to/sdkjs]');
  process.exit(2);
}
const workDir = mkdtempSync(path.join(os.tmpdir(), 'cell-styles-roundtrip-'));

// --- x2t, with the params XML and environment of convert_params_xml and convert_env ---

const FORMAT = { bin: 8192, xlsx: 257, ods: 259 };

function xmlEscape(text) {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function convert(input, output, formatTo) {
  const scratch = mkdtempSync(path.join(workDir, 'x2t-'));
  const params = path.join(scratch, 'params.xml');
  writeFileSync(params, '<?xml version="1.0" encoding="utf-8"?>\n'
    + '<TaskQueueDataConvert xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">\n'
    + '<m_sFileFrom>' + xmlEscape(input) + '</m_sFileFrom>\n'
    + '<m_sFileTo>' + xmlEscape(output) + '</m_sFileTo>\n'
    + '<m_nFormatTo>' + formatTo + '</m_nFormatTo>\n'
    + '<m_sTempDir>' + xmlEscape(scratch) + '</m_sTempDir>\n'
    + '<options>\n<allowPrivateIP>false</allowPrivateIP>\n<allowNetworkRequest>false</allowNetworkRequest>\n</options>\n'
    + '</TaskQueueDataConvert>');
  const env = { ...process.env, APPLICATION_NAME: 'Euro-Office-Lite', LD_LIBRARY_PATH: x2tDir };
  const result = spawnSync(path.join(x2tDir, 'x2t'), [params], { env, encoding: 'utf8' });
  if (result.status !== 0) {
    throw new Error('x2t ' + path.basename(input) + ' -> ' + path.basename(output) + ' exited ' + result.status + ': ' + result.stderr + result.stdout);
  }
  return output;
}

// --- The editor in a vm (from scripts/test-sdkjs-cell-styles.mjs) ---

function anything() {
  const target = function () {
    return anything();
  };
  return new Proxy(target, {
    get(object, property) {
      if (property === Symbol.toPrimitive) return () => 0;
      if (property === 'toString' || property === 'valueOf') return () => '';
      if (property === 'length') return 0;
      if (!(property in object)) object[property] = anything();
      return object[property];
    },
    construct() {
      return anything();
    },
  });
}

function makeContext() {
  const context = {
    console,
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    document: anything(),
    jQuery: anything(),
    $: anything(),
    navigator: { userAgent: 'node', platform: 'Linux', language: 'en', languages: ['en'], appVersion: '' },
    location: { href: 'http://localhost/', search: '', protocol: 'http:', hostname: 'localhost', origin: 'http://localhost' },
    devicePixelRatio: 1,
    Image: function () {},
    XMLHttpRequest: function () {},
    FileReader: function () {},
    Blob: function () {},
    Worker: function () {},
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    addEventListener() {},
    removeEventListener() {},
    getComputedStyle: () => anything(),
    requestAnimationFrame: () => 0,
    postMessage() {},
  };
  context.window = context;
  context.self = context;
  context.parent = context;
  context.top = context;
  vm.createContext(context);
  return context;
}

function loadSdkjs(context) {
  const config = JSON.parse(readFileSync(path.join(sdkjsRoot, 'configs/cell.json'), 'utf8')).sdk;
  const files = ['vendor/polyfill.js', 'vendor/xregexp-all-min.js', 'common/applyDocumentChanges.js', ...config.min, ...config.common];
  for (const file of files) {
    vm.runInContext(readFileSync(path.join(sdkjsRoot, file), 'utf8'), context, { filename: file });
  }
}

// The bridge opens an Editor.bin as the bytes of the file (OpenFileResult.data).
const START_UP = `
  Asc.spreadsheet_api.prototype._init = function () {};
  Asc.spreadsheet_api.prototype._loadFonts = function (fonts, callback) { callback(); };
  AscCommonExcel.WorkbookView.prototype._calcMaxDigitWidth = function () {};
  AscCommonExcel.WorkbookView.prototype._init = function () {};
  AscCommonExcel.WorkbookView.prototype._onWSSelectionChanged = function () {};
  AscCommonExcel.WorkbookView.prototype.showWorksheet = function () {};
  AscCommonExcel.WorkbookView.prototype.restoreFocus = function () {};
  AscCommonExcel.WorkbookView.prototype.recalculateDrawingObjects = function () {};
  AscCommonExcel.WorksheetView.prototype._init = function () {};
  AscCommonExcel.WorksheetView.prototype._onUpdateFormatTable = function () {};
  AscCommonExcel.WorksheetView.prototype.setSelection = function () {};
  AscCommonExcel.WorksheetView.prototype.draw = function () {};
  AscCommonExcel.WorksheetView.prototype._prepareDrawingObjects = function () {};
  AscCommonExcel.WorksheetView.prototype._reinitializeScroll = function () {};
  AscCommonExcel.WorksheetView.prototype.getZoom = function () {};
  AscCommon.baseEditorsApi.prototype._onEndLoadSdk = function () {};
  Asc.ReadDefTableStyles = function () {};
  AscCommonExcel.WorkbookView.prototype.getCellStyles = function () {
    return this.model.CellStyles.CustomStyles.map(function (style) { return style.Name; });
  };
  var api = new Asc.spreadsheet_api({'id-view': 'editor_sdk'});
  api.FontLoader = { LoadDocumentFonts: function () {} };
  window.Asc.editor = api;
  AscCommon.g_oTableId.init();
  api._onEndLoadSdk();
  api.isOpenOOXInBrowser = false;
  api.OpenDocumentFromBin(null, new Uint8Array(window.fileBytes));
  api.initCollaborativeEditing({});
  api.wb = new AscCommonExcel.WorkbookView(api.wbModel, api.controller, api.handlers, api.HtmlElement,
    api.topLineEditorElement, api, api.collaborativeEditing, api.fontRenderingMode);
  api.wbModel.handlers.add('getSelectionState', function () { return null; });
`;

// Helpers that run inside the vm, on the model. snapshot() is what a round
// trip is judged by.
const HELPERS = `
  function ws() { return api.wbModel.aWorksheets[0]; }
  function select(ref) {
    var range = ws().getRange2(ref).bbox;
    ws().selectionRange.ranges = [range];
    ws().selectionRange.activeCell = new AscCommon.CellBase(range.r1, range.c1);
  }
  function cellXfs(ref) {
    var xfs = null;
    ws().getRange2(ref)._foreachNoEmpty(function (cell) { xfs = cell.xfs; });
    return xfs;
  }
  function color(r, g, b) { return new Asc.asc_CColor(r, g, b); }
  function hex(c) { return c ? ('000000' + c.getRgb().toString(16)).slice(-6) : null; }
  // A border with no color is automatic, drawn black. The xlsx writer saves a
  // black border as automatic (indexed 64), so both read as black here.
  function borderHex(c) { return hex(c) || '000000'; }
  function line(c) { return new Asc.asc_CBorder(Asc.c_oAscBorderStyles.Thin, c); }
  function borders(sides, c) {
    var list = [];
    sides.forEach(function (side) { list[Asc.c_oAscBorderOptions[side]] = line(c); });
    return list;
  }
  // A border with no line on any side reads as no border: x2t gives a style
  // without one the empty border of the file.
  function describeBorder(border) {
    if (!border) return 'none';
    var sides = ['l', 't', 'r', 'b'].filter(function (side) {
      return border[side] && border[side].s !== Asc.c_oAscBorderStyles.None;
    });
    if (sides.length === 0) return 'none';
    return ['l', 't', 'r', 'b'].map(function (side) {
      return side + '=' + (sides.indexOf(side) !== -1 ? border[side].s + ':' + borderHex(border[side].c) : 'none');
    }).join(' ');
  }
  function describeXfs(xfs) {
    xfs = xfs || new AscCommonExcel.CellXfs();
    var font = xfs.getFont2();
    var fill = xfs.asc_getFillColor();
    var align = xfs.getAlign2();
    return {
      font: [font.getName(), font.getSize(), !!font.getBold(), !!font.getItalic(), font.getUnderline(), hex(font.getColor())].join(' '),
      fill: fill ? [fill.asc_getR(), fill.asc_getG(), fill.asc_getB()].join(',') : 'none',
      border: describeBorder(xfs.border),
      numFormat: xfs.getNum2().getFormat(),
      alignment: [align.getAlignHorizontal(), align.getAlignVertical(), !!align.getWrap(), align.getIndent(), align.getAngle()].join(' '),
    };
  }
  function describeStyle(style) {
    var description = describeXfs(style.xfs);
    description.builtinId = style.BuiltinId;
    description.applies = ['Number', 'Font', 'Fill', 'Border', 'Alignment'].filter(function (part) {
      return style['Apply' + (part === 'Number' ? 'NumberFormat' : part)] !== false;
    }).join(',');
    return description;
  }
  function snapshot(refs) {
    var styles = {};
    api.wbModel.CellStyles.CustomStyles.forEach(function (style) { styles[style.Name] = describeStyle(style); });
    var predefined = {};
    api.wbModel.CellStyles.DefaultStyles.forEach(function (style) { predefined[style.Name] = describeStyle(style); });
    var cells = {};
    refs.forEach(function (ref) {
      var range = ws().getRange2(ref);
      var description = describeXfs(cellXfs(ref));
      description.value = range.getValue();
      description.formula = range.getFormula();
      description.style = range.getStyleName();
      cells[ref] = description;
    });
    return { styles: styles, predefined: predefined, cells: cells };
  }
`;

function openEditor(binPath) {
  const context = makeContext();
  context.fileBytes = Array.from(readFileSync(binPath));
  loadSdkjs(context);
  vm.runInContext(START_UP + HELPERS, context);
  return (code) => vm.runInContext(code, context);
}

function openFile(file) {
  return openEditor(convert(file, file + '.bin', FORMAT.bin));
}

// asc_nativeGetFile gives a string or bytes; the bridge sends either as base64
// and the backend writes the decoded bytes as Editor.bin.
function saveFile(run, output) {
  const data = run('(function (d) { return typeof d === "string" ? d : Array.from(new Uint8Array(d)); })(api.asc_nativeGetFile())');
  const bytes = typeof data === 'string' ? Buffer.from(data, 'latin1') : Buffer.from(data);
  const editorBin = output + '.Editor.bin';
  writeFileSync(editorBin, bytes);
  return convert(editorBin, output, FORMAT[path.extname(output).slice(1)]);
}

// --- The edits ---

const CELLS = ['A1', 'A2', 'A3', 'A4', 'B1', 'B2', 'C1', 'C2', 'D1', 'D2', 'E1', 'E2', 'F1', 'G1'];

// A1:A4 data with no style; B, C and D the three edited styles; E, F and G
// predefined styles that are not edited.
const EDIT = `
  ws().getRange2('A1').setValue('1234.5');
  ws().getRange2('A2').setValue('Plain text');
  ws().getRange2('A3').setValue('3/5/2026');
  ws().getRange2('A4').setValue('=A1*2');
  ['B1', 'C1', 'D1', 'E1', 'F1', 'G1'].forEach(function (ref) { ws().getRange2(ref).setValue('0.25'); });
  ['B2', 'C2', 'D2', 'E2'].forEach(function (ref) { ws().getRange2(ref).setValue('Label'); });

  // New style from the selection: B1 formatted by hand, then the dialog adds
  // border and alignment to what the selection has.
  ws().getRange2('B1').setFontname('Arial');
  ws().getRange2('B1').setFontsize(14);
  ws().getRange2('B1').setBold(true);
  ws().getRange2('B1').setFontcolor(AscCommonExcel.createRgbColor(0, 0, 255));
  ws().getRange2('B1').setFillColor(AscCommonExcel.createRgbColor(255, 255, 0));
  ws().getRange2('B1').setNumFormat('0.00');
  var totals = cellXfs('B1').clone();
  totals.asc_setBorder(borders(['Bottom'], color(255, 0, 0)));
  totals.asc_setHorAlign(AscCommon.align_Center);
  totals.asc_setVertAlign(Asc.c_oAscVAlign.Top);
  totals.asc_setWrapText(true);
  var addTotals = api.asc_addCellStyle('Totals', totals);

  // Modify a predefined style.
  var good = api.asc_getCellStyleXfs('Good');
  good.asc_setFontName('Courier New');
  good.asc_setFontSize(12);
  good.asc_setFontItalic(true);
  good.asc_setFontColor(color(128, 0, 128));
  good.asc_setFillColor(color(204, 255, 204));
  good.asc_setBorder(borders(['Top', 'Bottom', 'Left', 'Right'], color(0, 0, 0)));
  good.asc_setNumFormatInfo('#,##0.000');
  good.asc_setHorAlign(AscCommon.align_Right);
  good.asc_setVertAlign(Asc.c_oAscVAlign.Center);
  var modifyGood = api.asc_modifyCellStyle('Good', good);

  // Duplicate another predefined style under a new name, with changes.
  var badCopy = api.asc_getCellStyleXfs('Bad');
  badCopy.asc_setFontName('Times New Roman');
  badCopy.asc_setFontUnderline(true);
  badCopy.asc_setFillColor(color(255, 204, 153));
  badCopy.asc_setBorder(borders(['Top', 'Bottom'], color(0, 0, 0)));
  badCopy.asc_setNumFormatInfo('0%');
  badCopy.asc_setHorAlign(AscCommon.align_Left);
  badCopy.asc_setWrapText(true);
  var addBadCopy = api.asc_addCellStyle('Bad 2', badCopy);

  select('B1:B2'); api.asc_setCellStyle('Totals');
  select('C1:C2'); api.asc_setCellStyle('Good');
  select('D1:D2'); api.asc_setCellStyle('Bad 2');
  select('E1:E2'); api.asc_setCellStyle('Bad');
  select('F1'); api.asc_setCellStyle('Neutral');
  select('G1'); api.asc_setCellStyle('Percent');
  // A change by hand on top of a style.
  ws().getRange2('B2').setItalic(true);
  [addTotals, modifyGood, addBadCopy];
`;

// --- Comparison ---

const differences = [];

function compareGroup(label, group, saved, reopened) {
  for (const name of Object.keys(saved)) {
    if (!(name in reopened)) {
      differences.push({ label, what: group + ' ' + name, property: 'present', saved: 'yes', reopened: 'missing' });
      continue;
    }
    for (const property of Object.keys(saved[name])) {
      const before = JSON.stringify(saved[name][property]);
      const after = JSON.stringify(reopened[name][property]);
      if (before !== after) differences.push({ label, what: group + ' ' + name, property, saved: before, reopened: after });
    }
  }
  for (const name of Object.keys(reopened)) {
    if (!(name in saved)) differences.push({ label, what: group + ' ' + name, property: 'present', saved: 'missing', reopened: 'yes' });
  }
}

function compare(label, saved, reopened) {
  const before = differences.length;
  compareGroup(label, 'style', saved.styles, reopened.styles);
  compareGroup(label, 'predefined', saved.predefined, reopened.predefined);
  compareGroup(label, 'cell', saved.cells, reopened.cells);
  const found = differences.length - before;
  console.log((found === 0 ? 'ok   ' : 'DIFF ') + label + (found === 0 ? '' : ' (' + found + ' differences)'));
}

function snapshotOf(run, refs = CELLS) {
  return JSON.parse(run('JSON.stringify(snapshot(' + JSON.stringify(refs) + '))'));
}

function zipPart(file, part) {
  return execFileSync('unzip', ['-p', file, part], { encoding: 'utf8' });
}

// The names of the cellStyle elements of xl/styles.xml.
function xlsxStyleNames(file) {
  return [...zipPart(file, 'xl/styles.xml').matchAll(/<cellStyle [^>]*name="([^"]*)"/g)].map((m) => m[1]);
}

// The table-cell styles of styles.xml, by display name when there is one.
function odsStyleNames(file) {
  return [...zipPart(file, 'styles.xml').matchAll(/<style:style\b[^>]*>/g)]
    .map((m) => m[0])
    .filter((tag) => tag.includes('style:family="table-cell"'))
    .map((tag) => (tag.match(/style:display-name="([^"]*)"/) || tag.match(/style:name="([^"]*)"/))[1]);
}

function xmlSection(xml, name) {
  const match = xml.match(new RegExp('<' + name + '\\b[^>]*>([\\s\\S]*?)</' + name + '>'));
  return match ? match[1] : '';
}

function xfIdsIn(xml) {
  return [...xml.matchAll(/\bxfId="(\d+)"/g)].map((m) => Number(m[1]));
}

// The cellStyleXfs of xl/styles.xml and the xfId values of cellXfs and
// cellStyles: each one is a position in cellStyleXfs.
function xfIdReferences(file) {
  const styles = zipPart(file, 'xl/styles.xml');
  const count = (xmlSection(styles, 'cellStyleXfs').match(/<xf\b/g) || []).length;
  const used = [...xfIdsIn(xmlSection(styles, 'cellXfs')), ...xfIdsIn(xmlSection(styles, 'cellStyles'))];
  return { count, used, broken: used.filter((id) => id >= count) };
}

// A predefined style comes into the document without a history item, so
// undoing a new style added before it leaves that style's XfId unused. The
// result is the XfIds of the workbook.
const GAP_CELLS = ['A1', 'A2', 'A3'];
const GAP_EDIT = `
  ['A1', 'A2', 'A3'].forEach(function (ref) { ws().getRange2(ref).setValue('0.25'); });
  var temporary = new AscCommonExcel.CellXfs();
  temporary.asc_setFillColor(color(1, 2, 3));
  api.asc_addCellStyle('Temporary', temporary);
  select('A1'); api.asc_setCellStyle('Good');
  AscCommon.History.Undo();
  AscCommon.History.Undo();
  select('A1'); api.asc_setCellStyle('Good');
  select('A2'); api.asc_setCellStyle('Bad');
  var later = new AscCommonExcel.CellXfs();
  later.asc_setFillColor(color(4, 5, 6));
  api.asc_addCellStyle('Later', later);
  select('A3'); api.asc_setCellStyle('Later');
  api.wbModel.CellStyles.CustomStyles.map(function (s) { return s.XfId; }).sort(function (a, b) { return a - b; }).join(',');
`;

// Whether the writer gives the same bytes as with each XfId written as it is,
// the way it wrote them before it wrote positions in cellStyleXfs.
const WRITES_XFIDS_AS_THEY_ARE = `(function () {
  var prototype = AscCommonExcel.StylesForWrite.prototype;
  var cellStyleXfIndex = prototype.getCellStyleXfIndex;
  var written = api.asc_nativeGetFile();
  prototype.getCellStyleXfIndex = function (xfId) { return xfId; };
  try {
    return written === api.asc_nativeGetFile();
  } finally {
    prototype.getCellStyleXfIndex = cellStyleXfIndex;
  }
})()`;

function checkSameBytes(label, run) {
  const same = run(WRITES_XFIDS_AS_THEY_ARE);
  if (!same) differences.push({ label, what: 'Editor.bin', property: 'bytes', saved: 'XfIds as they are', reopened: 'other bytes' });
  console.log((same ? 'ok   ' : 'DIFF ') + label);
}

// --- Run ---

const blank = path.join(workDir, 'blank.xlsx');
writeFileSync(blank, readFileSync(path.join(projectRoot, 'src-tauri/templates/blank.xlsx')));
const editor = openFile(blank);
const results = editor(EDIT);
const No = editor('Asc.c_oAscError.ID.No');
if (results.some((result) => result !== No)) {
  console.log('FAIL the edits returned ' + JSON.stringify(results) + ', expected ' + No);
  process.exit(1);
}
const saved = snapshotOf(editor);
writeFileSync(path.join(workDir, 'saved.json'), JSON.stringify(saved, null, 1));

const savedXlsx = saveFile(editor, path.join(workDir, 'saved.xlsx'));
const savedOds = saveFile(editor, path.join(workDir, 'saved.ods'));
console.log('saved xlsx, cellStyles: ' + xlsxStyleNames(savedXlsx).join(' | '));
console.log('saved ods, table-cell styles: ' + odsStyleNames(savedOds).join(' | '));

const fromXlsx = openFile(savedXlsx);
compare('xlsx', saved, snapshotOf(fromXlsx));

const fromOds = openFile(savedOds);
compare('ods', saved, snapshotOf(fromOds));

const xlsxAgain = saveFile(fromXlsx, path.join(workDir, 'xlsx-again.xlsx'));
compare('xlsx -> xlsx', saved, snapshotOf(openFile(xlsxAgain)));

const odsToXlsx = saveFile(fromOds, path.join(workDir, 'ods-then.xlsx'));
compare('xlsx -> ods -> xlsx', saved, snapshotOf(openFile(odsToXlsx)));

// Opened from an xlsx with cell styles and no gap in the XfIds.
checkSameBytes('xlsx opened: the bytes of the XfIds as they are', fromXlsx);

const gapEditor = openFile(blank);
console.log('after undo, XfIds of the workbook: ' + gapEditor(GAP_EDIT));
const gapSaved = snapshotOf(gapEditor, GAP_CELLS);
const gapXlsx = saveFile(gapEditor, path.join(workDir, 'gap.xlsx'));
const references = xfIdReferences(gapXlsx);
console.log('after undo, xlsx: ' + references.count + ' cellStyleXfs, xfId used ' + [...new Set(references.used)].join(','));
if (references.broken.length > 0) {
  differences.push({ label: 'xlsx after undo', what: 'styles.xml', property: 'xfId', saved: 'below ' + references.count, reopened: references.broken.join(',') });
}
compare('xlsx after undo', gapSaved, snapshotOf(openFile(gapXlsx), GAP_CELLS));

for (const d of differences) {
  console.log('  ' + d.label + ': ' + d.what + ' ' + d.property + ': ' + d.saved + ' -> ' + d.reopened);
}
console.log('\nFiles in ' + workDir);
process.exit(differences.length === 0 ? 0 : 1);
