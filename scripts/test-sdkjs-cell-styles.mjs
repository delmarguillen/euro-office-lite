// Checks editing cell styles in src/sdkjs (asc_addCellStyle, asc_modifyCellStyle,
// asc_getCellStyleXfs) with the sdkjs patches applied. The spreadsheet model
// runs in a node vm: the script loads the files configs/cell.json lists, on
// stubs for the browser objects, and opens an empty workbook as the qunit
// tests in src/sdkjs/tests/cell do. Drawing and the gallery thumbnails are
// stubbed; the model, the history and the binary writer and reader are real.
//
// The save check writes the workbook with BinaryFileWriter, the format the app
// hands to x2t to produce the xlsx, and opens it again with OpenDocumentFromBin.
// The xlsx conversion itself runs in x2t and is not part of this script.
//
// Run: node scripts/apply-patches.mjs apply --only sdkjs
//      node scripts/test-sdkjs-cell-styles.mjs [path/to/sdkjs]
//      node scripts/apply-patches.mjs revert --only sdkjs
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const defaultRoot = fileURLToPath(new URL('../src/sdkjs/', import.meta.url));
const sdkjsRoot = process.argv[2] || defaultRoot;

// An object that has every property and can be called or constructed: enough
// for the DOM and canvas calls the files make while loading.
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

// Same stubs and start-up as src/sdkjs/tests/cell/spreadsheet-calculation/CellSettingsTests.js.
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
  // The gallery gets the style names instead of thumbnails drawn on a canvas.
  AscCommonExcel.WorkbookView.prototype.getCellStyles = function () {
    return this.model.CellStyles.CustomStyles.map(function (style) { return style.Name; });
  };
  var api = new Asc.spreadsheet_api({'id-view': 'editor_sdk'});
  api.FontLoader = { LoadDocumentFonts: function () {} };
  window.Asc.editor = api;
  AscCommon.g_oTableId.init();
  api._onEndLoadSdk();
  api.isOpenOOXInBrowser = false;
  var galleryUpdates = [];
  api.asc_registerCallback('asc_onInitEditorStyles', function (names) { galleryUpdates.push(names); });
  function openWorkbook(data) {
    api.OpenDocumentFromBin(null, data);
    api.initCollaborativeEditing({});
    api.wb = new AscCommonExcel.WorkbookView(api.wbModel, api.controller, api.handlers, api.HtmlElement,
      api.topLineEditorElement, api, api.collaborativeEditing, api.fontRenderingMode);
    api.wbModel.handlers.add('getSelectionState', function () { return null; });
  }
  openWorkbook(window.workbookData || AscCommon.getEmpty());
`;

// Helpers that run inside the vm, on the model.
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
  function describe(ref) {
    var xfs = cellXfs(ref) || new AscCommonExcel.CellXfs();
    return {
      style: ws().getRange2(ref).getStyleName(),
      bold: xfs.getFont2().getBold(),
      italic: xfs.getFont2().getItalic(),
      size: xfs.getFont2().getSize(),
      fill: xfs.asc_getFillColor() ? xfs.asc_getFillColor().asc_getR() + ',' + xfs.asc_getFillColor().asc_getG() + ',' + xfs.asc_getFillColor().asc_getB() : 'none',
      num: xfs.getNum2().getFormat(),
      hor: xfs.getAlign2().getAlignHorizontal(),
      wrap: !!xfs.getAlign2().getWrap(),
    };
  }
  function color(r, g, b) { return new Asc.asc_CColor(r, g, b); }
  function styleNames() { return api.wbModel.CellStyles.CustomStyles.map(function (s) { return s.Name; }); }
  function historyPoints() { return AscCommon.History.Points.length; }
`;

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok });
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : ` (${detail})`}`);
}
function same(name, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  check(name, a === e, `got ${a}, expected ${e}`);
}

// A fresh vm per workbook, as a new editor session: data is a workbook from asc_nativeGetFile.
function openEditor(data) {
  const context = makeContext();
  context.workbookData = data;
  loadSdkjs(context);
  vm.runInContext(START_UP + HELPERS, context);
  return (code) => vm.runInContext(code, context);
}
let run = openEditor(null);
const No = run('Asc.c_oAscError.ID.No');
const errors = run('({ exists: Asc.c_oAscError.ID.CellStyleNameExists, empty: Asc.c_oAscError.ID.CellStyleNameEmpty, notFound: Asc.c_oAscError.ID.CellStyleNotFound, normal: Asc.c_oAscError.ID.CellStyleNormalNotEditable })');
if (!run("typeof api.asc_modifyCellStyle === 'function' && typeof api.asc_addCellStyle === 'function'")) {
  console.log('FAIL asc_addCellStyle and asc_modifyCellStyle are missing');
  console.log('Are the sdkjs patches applied? node scripts/apply-patches.mjs apply --only sdkjs');
  process.exit(1);
}

// --- New style from the selection ---
run(`
  ws().getRange2('A1').setValue('1');
  select('A1');
  ws().getRange2('A1').setBold(true);
  ws().getRange2('A1').setFillColor(AscCommonExcel.createRgbColor(255, 0, 0));
  ws().getRange2('A1').setNumFormat('0.00');
  // What asc_getCellInfo().asc_getXfs() returns for A1 (it needs the drawing layer).
  var selectionXfs = cellXfs('A1').clone();
  var pointsBeforeAdd = historyPoints();
  var galleryBeforeAdd = galleryUpdates.length;
  var addResult = api.asc_addCellStyle('Totals', selectionXfs);
`);
same('add: returns No', run('addResult'), No);
check('add: the style is in the document', run("styleNames().indexOf('Totals') !== -1"), run('styleNames().join()'));
same('add: one history point', run('historyPoints() - pointsBeforeAdd'), 1);
check('add: the gallery is sent again with the style', run("galleryUpdates.length > galleryBeforeAdd && galleryUpdates[galleryUpdates.length - 1].indexOf('Totals') !== -1"), run('JSON.stringify(galleryUpdates[galleryUpdates.length - 1])'));
run("var totalsXfs = api.asc_getCellStyleXfs('Totals');");
same('add: the style keeps the selection\'s values', run("[totalsXfs.getFont2().getBold(), totalsXfs.asc_getFillColor().asc_getR(), totalsXfs.getNum2().getFormat()]"), [true, 255, '0.00']);
same('add: the style xfs has no XfId of its own', run('api.wbModel.CellStyles.getStyleByName("Totals").xfs.XfId'), null);
same('add: the style takes the next XfId', run('api.wbModel.CellStyles.getStyleByName("Totals").XfId'), 1);

// --- Repeated or empty names ---
run('var pointsBeforeRepeat = historyPoints();');
same('repeated name: error, nothing added', run("[api.asc_addCellStyle('Totals', selectionXfs), styleNames().filter(function (n) { return n === 'Totals'; }).length]"), [errors.exists, 1]);
same('repeated name in another case: error', run("api.asc_addCellStyle('TOTALS', selectionXfs)"), errors.exists);
same('name of a predefined style: error', run("api.asc_addCellStyle('Good', selectionXfs)"), errors.exists);
same('empty name: error', run("api.asc_addCellStyle('  ', selectionXfs)"), errors.empty);
same('asc_checkCellStyleName agrees', run("[api.asc_checkCellStyleName('Totals'), api.asc_checkCellStyleName('Totals 2')]"), [errors.exists, No]);
same('rejected names add no history point', run('historyPoints() - pointsBeforeRepeat'), 0);
same('modify an unknown style: error', run("api.asc_modifyCellStyle('Nope', selectionXfs)"), errors.notFound);
same('modify Normal: error', run("api.asc_modifyCellStyle('Normal', selectionXfs)"), errors.normal);

// --- Applying a style: font, fill, border and number format; Totals has no alignment ---
run(`
  select('B1:B3');
  api.asc_setCellStyle('Totals');
  select('C1');
  api.asc_setCellStyle('Good');
`);
same('apply: B1 takes the style', run("describe('B1')"), { style: 'Totals', bold: true, italic: false, size: 11, fill: '255,0,0', num: '0.00', hor: null, wrap: false });

// Changes by hand: B2 not bold, B3 blue fill.
run(`
  ws().getRange2('B2').setBold(false);
  ws().getRange2('B3').setFillColor(AscCommonExcel.createRgbColor(0, 0, 255));
  var goodBefore = describe('C1');
`);

// --- Modify the style ---
run(`
  var newXfs = api.asc_getCellStyleXfs('Totals');
  newXfs.asc_setFontItalic(true);
  newXfs.asc_setFontSize(14);
  newXfs.asc_setFillColor(color(0, 255, 0));
  newXfs.asc_setNumFormatInfo('0.000');
  newXfs.asc_setHorAlign(AscCommon.align_Center);
  newXfs.asc_setWrapText(true);
  var oldStyleXfs = api.wbModel.CellStyles.getStyleByName('Totals').xfs;
  var oldFill = oldStyleXfs.fill;
  var pointsBeforeModify = historyPoints();
  var galleryBeforeModify = galleryUpdates.length;
  var modifyResult = api.asc_modifyCellStyle('Totals', newXfs);
`);
same('modify: returns No', run('modifyResult'), No);
same('modify: one history point', run('historyPoints() - pointsBeforeModify'), 1);
check('modify: the gallery is sent again', run('galleryUpdates.length > galleryBeforeModify'), 'no asc_onInitEditorStyles');
check('modify: the style gets a new xfs, the old one is not changed', run("api.wbModel.CellStyles.getStyleByName('Totals').xfs !== oldStyleXfs && oldStyleXfs.fill === oldFill && oldStyleXfs.getFont2().getItalic() === false"), 'old xfs changed');
same('modify: the XfId stays', run("api.wbModel.CellStyles.getStyleByName('Totals').XfId"), 1);
const center = run('AscCommon.align_Center');
const right = run('AscCommon.align_Right');
same('modify: B1 takes every new value', run("describe('B1')"), { style: 'Totals', bold: true, italic: true, size: 14, fill: '0,255,0', num: '0.000', hor: center, wrap: true });
same('modify: B2 keeps bold off, set by hand', run("describe('B2')"), { style: 'Totals', bold: false, italic: true, size: 14, fill: '0,255,0', num: '0.000', hor: center, wrap: true });
same('modify: B3 keeps its blue fill, set by hand', run("describe('B3')"), { style: 'Totals', bold: true, italic: true, size: 14, fill: '0,0,255', num: '0.000', hor: center, wrap: true });
same('modify: a cell with another style does not change', run("describe('C1')"), run('goodBefore'));
same('modify: the selection cell A1 (no style) does not change', run("describe('A1').style"), 'Normal');

// --- Undo and redo ---
run('AscCommon.History.Undo();');
same('undo: B1 is back', run("describe('B1')"), { style: 'Totals', bold: true, italic: false, size: 11, fill: '255,0,0', num: '0.00', hor: null, wrap: false });
same('undo: B3 is back', run("describe('B3').fill"), '0,0,255');
check('undo: the style xfs is back', run("api.wbModel.CellStyles.getStyleByName('Totals').xfs === oldStyleXfs"), 'xfs differs');
run('var galleryBeforeRedo = galleryUpdates.length; AscCommon.History.Redo();');
same('redo: B1 has the new values again', run("describe('B1')"), { style: 'Totals', bold: true, italic: true, size: 14, fill: '0,255,0', num: '0.000', hor: center, wrap: true });
same('redo: B2 keeps bold off', run("describe('B2').bold"), false);
check('redo: the gallery is sent again', run('galleryUpdates.length > galleryBeforeRedo'), 'no asc_onInitEditorStyles');

// --- Predefined style not in the document yet ---
run(`
  var stylesBeforeBad = api.wbModel.CellStyles.CustomStyles.length;
  var badXfs = api.asc_getCellStyleXfs('Bad');
  badXfs.asc_setFontBold(true);
  var badResult = api.asc_modifyCellStyle('Bad', badXfs);
  var badCopy = api.wbModel.CellStyles.getStyleByName('Bad');
`);
same('modify predefined: returns No', run('badResult'), No);
check('modify predefined: copied into the document with its BuiltinId', run("api.wbModel.CellStyles.isCustomStyle(badCopy) && badCopy.BuiltinId === api.wbModel.CellStyles.DefaultStyles.filter(function (s) { return s.Name === 'Bad'; })[0].BuiltinId && badCopy.xfs.getFont2().getBold()"), 'no copy');
check('modify predefined: the predefined list does not change', run("!api.wbModel.CellStyles.DefaultStyles.filter(function (s) { return s.Name === 'Bad'; })[0].xfs.getFont2().getBold()"), 'default style changed');
run('AscCommon.History.Undo();');
same('modify predefined, undo: the copy goes away', run("[api.wbModel.CellStyles.CustomStyles.length - stylesBeforeBad, api.wbModel.CellStyles.isCustomStyle(api.wbModel.CellStyles.getStyleByName('Bad'))]"), [0, false]);
run('AscCommon.History.Redo();');
same('modify predefined, redo: the copy is back', run("api.wbModel.CellStyles.getStyleByName('Bad').xfs.getFont2().getBold()"), true);

// --- Predefined style already used in the document ---
run(`
  var goodXfs = api.asc_getCellStyleXfs('Good');
  goodXfs.asc_setFontItalic(true);
  var goodResult = api.asc_modifyCellStyle('Good', goodXfs);
`);
same('modify used predefined: returns No', run('goodResult'), No);
same('modify used predefined: C1 follows', run("describe('C1').italic"), true);

// --- Undo of an added style ---
run(`
  var stylesBeforeExtra = styleNames().length;
  var extraXfs = selectionXfs.clone();
  extraXfs.asc_setHorAlign(AscCommon.align_Right);
  extraXfs.asc_setVertAlign(Asc.c_oAscVAlign.Top);
  api.asc_addCellStyle('Extra', extraXfs);
  var extraAlign = api.asc_getCellStyleXfs('Extra').getAlign2();
  var extraAlignValues = [extraAlign.getAlignHorizontal(), extraAlign.getAlignVertical()];
  AscCommon.History.Undo();
`);
same('add with alignment: the style keeps it', run('extraAlignValues'), run('[AscCommon.align_Right, Asc.c_oAscVAlign.Top]'));
check('alignment setters change a copy, not the shared Align', run('selectionXfs.align === null || selectionXfs.align !== extraXfs.align'), 'shared Align changed');
same('add, undo: the style goes away', run("[styleNames().length - stylesBeforeExtra, styleNames().indexOf('Extra')]"), [0, -1]);
run('AscCommon.History.Redo();');
same('add, redo: the style is back', run("styleNames().indexOf('Extra') !== -1"), true);

// --- Alignment of a style, applied as Excel applies cellStyleXfs applyAlignment ---
run(`
  function alignOf(ref) {
    var align = (cellXfs(ref) || new AscCommonExcel.CellXfs()).getAlign2();
    return [align.getAlignHorizontal(), align.getAlignVertical(), !!align.wrap, align.getIndent(), align.getAngle()];
  }
  var defaultAlign = alignOf('D1');
  var centeredXfs = new AscCommonExcel.CellXfs();
  centeredXfs.asc_setHorAlign(AscCommon.align_Center);
  centeredXfs.asc_setVertAlign(Asc.c_oAscVAlign.Top);
  centeredXfs.asc_setWrapText(true);
  centeredXfs.align.setAngle(45);
  api.asc_addCellStyle('Centered', centeredXfs);
  api.asc_addCellStyle('No alignment', new AscCommonExcel.CellXfs());
  select('D1:D3');
  var pointsBeforeCentered = historyPoints();
  api.asc_setCellStyle('Centered');
`);
const top = run('Asc.c_oAscVAlign.Top');
const centeredAlign = [center, top, true, 0, 45];
same('align flag: an added style applies alignment only when it has one', run("[api.wbModel.CellStyles.getStyleByName('Centered').ApplyAlignment, api.wbModel.CellStyles.getStyleByName('No alignment').ApplyAlignment]"), [true, false]);
same('align flag: of the predefined styles only Normal applies alignment', run("api.wbModel.CellStyles.DefaultStyles.filter(function (s) { return false !== s.ApplyAlignment; }).map(function (s) { return s.Name; })"), ['Normal']);
same('apply centered style: D1 is centered, top, wrapped, at 45 degrees', run("alignOf('D1')"), centeredAlign);
same('apply centered style: one history point', run('historyPoints() - pointsBeforeCentered'), 1);
run('AscCommon.History.Undo();');
same('apply centered style, undo: D1 alignment is back', run("[describe('D1').style, alignOf('D1')]"), ['Normal', run('defaultAlign')]);
run('AscCommon.History.Redo();');
same('apply centered style, redo: D1 is centered again', run("alignOf('D1')"), centeredAlign);
run(`
  ws().getRange2('E1').setAlignHorizontal(AscCommon.align_Right);
  ws().getRange2('E2').setAlignHorizontal(AscCommon.align_Right);
  ws().getRange2('E2').setAngle(45);
  var handAlignE2 = alignOf('E2');
  select('E1'); api.asc_setCellStyle('Title');
  select('E2'); api.asc_setCellStyle('Good');
  select('E3'); api.asc_setCellStyle('No alignment');
`);
same('apply Title (no alignment): E1 keeps its right alignment', run("[describe('E1').style, alignOf('E1')[0]]"), ['Title', right]);
same('apply Good (no alignment): E2 keeps alignment and angle', run("[describe('E2').style, alignOf('E2')]"), ['Good', run('handAlignE2')]);
run("select('D3'); api.asc_setCellStyle('Normal');");
same('apply Normal (applies alignment): D3 back to the default alignment', run("alignOf('D3')"), run('defaultAlign'));
run("select('F1:F1048576'); api.asc_setCellStyle('Centered');");
same('apply to a whole column: the column takes the alignment', run("(function (a) { return [a.getAlignHorizontal(), a.getAlignVertical()]; })(ws().aCols[5].xfs.getAlign2())"), [center, top]);
run(`
  ws().getRange2('D2').setAlignHorizontal(AscCommon.align_Right);
  var centeredNew = api.asc_getCellStyleXfs('Centered');
  centeredNew.asc_setHorAlign(AscCommon.align_Left);
  centeredNew.asc_setVertAlign(Asc.c_oAscVAlign.Center);
  api.asc_modifyCellStyle('Centered', centeredNew);
`);
const left = run('AscCommon.align_Left');
const vcenter = run('Asc.c_oAscVAlign.Center');
same('modify alignment: D1 takes the new alignment', run("alignOf('D1')"), [left, vcenter, true, 0, 45]);
same('modify alignment: D2 keeps its right alignment, set by hand', run("alignOf('D2')"), [right, vcenter, true, 0, 45]);
run(`
  var badAlignXfs = api.asc_getCellStyleXfs('Good');
  badAlignXfs.asc_setFontBold(true);
  api.asc_modifyCellStyle('Good', badAlignXfs);
`);
same('modify without alignment: Good still does not apply it, E2 keeps its own', run("[api.wbModel.CellStyles.getStyleByName('Good').ApplyAlignment, alignOf('E2')]"), [false, run('handAlignE2')]);

// --- Save and open again ---
const saved = run('api.asc_nativeGetFile()');
run = openEditor(saved);
same('reopen: the new styles are there', run("['Totals', 'Extra', 'Bad', 'Good'].map(function (n) { return api.wbModel.CellStyles.isCustomStyle(api.wbModel.CellStyles.getStyleByName(n)); })"), [true, true, true, true]);
same('reopen: the modified style keeps its values', run("(function (x) { return [x.getFont2().getItalic(), x.getFont2().getSize(), x.asc_getFillColor().asc_getG(), x.getNum2().getFormat(), x.getAlign2().getAlignHorizontal(), !!x.getAlign2().getWrap()]; })(api.wbModel.CellStyles.getStyleByName('Totals').xfs)"), [true, 14, 255, '0.000', center, true]);
same('reopen: the added style keeps its alignment', run("api.wbModel.CellStyles.getStyleByName('Extra').xfs.getAlign2().getAlignHorizontal()"), right);
same('reopen: the modified predefined style keeps its values', run("api.wbModel.CellStyles.getStyleByName('Bad').xfs.getFont2().getBold()"), true);
same('reopen: B1 still uses the style', run("describe('B1')"), { style: 'Totals', bold: true, italic: true, size: 14, fill: '0,255,0', num: '0.000', hor: center, wrap: true });
same('reopen: B2 keeps bold off', run("describe('B2').bold"), false);
same('reopen: XfIds are 0..n-1', run('api.wbModel.CellStyles.CustomStyles.map(function (s) { return s.XfId; }).sort(function (a, b) { return a - b; }).join()'), run('api.wbModel.CellStyles.CustomStyles.map(function (s, i) { return i; }).join()'));
run(`
  var afterOpenXfs = api.asc_getCellStyleXfs('Totals');
  afterOpenXfs.asc_setFontBold(false);
  var afterOpenResult = api.asc_modifyCellStyle('Totals', afterOpenXfs);
`);
same('reopen: the applyAlignment flags stay', run("['Centered', 'No alignment', 'Totals', 'Good', 'Title'].map(function (n) { return api.wbModel.CellStyles.getStyleByName(n).ApplyAlignment; })"), [true, false, true, false, false]);
same('reopen: D1 keeps the style alignment', run("[describe('D1').style, describe('D1').hor]"), ['Centered', left]);
run(`
  ws().getRange2('G1').setAlignHorizontal(AscCommon.align_Right);
  select('G1'); api.asc_setCellStyle('Good');
  select('G2'); api.asc_setCellStyle('Centered');
`);
same('reopen: Good leaves G1 alignment, Centered aligns G2', run("[describe('G1').hor, describe('G2').hor]"), [right, left]);
same('reopen: modify still works and reaches B1', run("[afterOpenResult, describe('B1').bold]"), [No, false]);

const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
if (failed > 0) {
  console.log('Are the sdkjs patches applied? node scripts/apply-patches.mjs apply --only sdkjs');
  process.exit(1);
}
