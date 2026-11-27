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
// The last cases open the cell style dialog of src/web-apps on the same model,
// with its widgets stubbed, so they need the web-apps patches as well.
//
// Run: node scripts/apply-patches.mjs apply
//      node scripts/test-sdkjs-cell-styles.mjs [path/to/sdkjs [path/to/web-apps]]
//      node scripts/apply-patches.mjs revert
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
  console.log('Are the sdkjs and web-apps patches applied? node scripts/apply-patches.mjs apply');
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

// --- A font picked by name replaces the theme font ---
// A font that keeps a theme scheme is saved with it, and on opening the theme font wins over the name.
run = openEditor(null);
run(`
  function fontOf(xfs) { return [xfs.getFont2().getName(), xfs.getFont2().getScheme()]; }
  function styleFont(name) { return fontOf(api.wbModel.CellStyles.getStyleByName(name).xfs); }
  function cellFont(ref) { return fontOf(cellXfs(ref)); }
  var noScheme = Asc.EFontScheme.fontschemeNone;
  ws().getRange2('A1').setValue('1');
  select('A1'); api.asc_setCellStyle('Good');
  ws().getRange2('C1').setValue('1');
  var schemesBefore = [api.asc_getCellStyleXfs('Good'), api.asc_getCellStyleXfs('Bad'), ws().getRange2('C1').getXfs(false)].map(function (x) { return x.getFont2().getScheme(); });

  var goodFontXfs = api.asc_getCellStyleXfs('Good');
  goodFontXfs.asc_setFontName('Courier New');
  var goodFontResult = api.asc_modifyCellStyle('Good', goodFontXfs);

  var bad2Xfs = api.asc_getCellStyleXfs('Bad');
  bad2Xfs.asc_setFontName('Courier New');
  var bad2Result = api.asc_addCellStyle('Bad 2', bad2Xfs);
  select('B1'); api.asc_setCellStyle('Bad 2');

  var fromNormalXfs = ws().getRange2('C1').getXfs(false).clone();
  fromNormalXfs.asc_setFontName('Courier New');
  var fromNormalResult = api.asc_addCellStyle('From Normal', fromNormalXfs);
  select('C2'); api.asc_setCellStyle('From Normal');
`);
check('font name: Good, Bad and a Normal cell start with a theme font', run('schemesBefore.every(function (s) { return s !== noScheme; })'), run('JSON.stringify(schemesBefore)'));
same('font name: the styles are saved', run('[goodFontResult, bad2Result, fromNormalResult]'), [No, No, No]);
const courier = run("['Courier New', noScheme]");
same('font name, modified Good: the style has no theme font', run("styleFont('Good')"), courier);
same('font name, modified Good: A1 follows without a theme font', run("cellFont('A1')"), courier);
same('font name, copy of Bad: the style has no theme font', run("styleFont('Bad 2')"), courier);
same('font name, copy of Bad: B1 has no theme font', run("cellFont('B1')"), courier);
same('font name, from a Normal cell: the style has no theme font', run("styleFont('From Normal')"), courier);
same('font name, from a Normal cell: C2 has no theme font', run("cellFont('C2')"), courier);
const fontSaved = run('api.asc_nativeGetFile()');
run = openEditor(fontSaved);
run('function fontOf(xfs) { return [xfs.getFont2().getName(), xfs.getFont2().getScheme()]; }');
same('font name, reopen: styles and cells keep Courier New', run("[api.wbModel.CellStyles.getStyleByName('Good').xfs, api.wbModel.CellStyles.getStyleByName('Bad 2').xfs, api.wbModel.CellStyles.getStyleByName('From Normal').xfs, cellXfs('A1'), cellXfs('B1'), cellXfs('C2')].map(fontOf)"), [courier, courier, courier, courier, courier, courier]);

// --- Border and number format of a predefined style that did not apply them ---
run = openEditor(null);
run(`
  function bottomOf(ref) { return (cellXfs(ref) || new AscCommonExcel.CellXfs()).getBorder().b.s; }
  function flagsOf(name) { var s = api.wbModel.CellStyles.getStyleByName(name); return [s.ApplyBorder, s.ApplyNumberFormat]; }
  ws().getRange2('A1').setValue('1');
  ws().getRange2('A2').setValue('1');
  ws().getRange2('A2').setNumFormat('0.0');
  select('A1:A2'); api.asc_setCellStyle('Good');
  var flagsBefore = flagsOf('Good');
  var fontOnlyXfs = api.asc_getCellStyleXfs('Good');
  fontOnlyXfs.asc_setFontBold(true);
  api.asc_modifyCellStyle('Good', fontOnlyXfs);
  var flagsFontOnly = flagsOf('Good');

  var borderXfs = api.asc_getCellStyleXfs('Good');
  var borders = [];
  borders[Asc.c_oAscBorderOptions.Bottom] = {style: Asc.c_oAscBorderStyles.Thin, color: new Asc.asc_CColor(0, 0, 0)};
  borderXfs.asc_setBorder(borders);
  borderXfs.asc_setNumFormatInfo('0.000');
  var borderResult = api.asc_modifyCellStyle('Good', borderXfs);
`);
const thin = run('Asc.c_oAscBorderStyles.Thin');
same('border and format: Good applies neither at first', run('flagsBefore'), [false, false]);
same('border and format: a font change leaves both off', run('flagsFontOnly'), [false, false]);
same('border and format: setting them turns both on', run("[borderResult, flagsOf('Good')]"), [No, [true, true]]);
same('border and format: A1 takes the border and the format', run("[bottomOf('A1'), describe('A1').num]"), [thin, '0.000']);
same('border and format: A2 takes the border, keeps its own format', run("[bottomOf('A2'), describe('A2').num]"), [thin, '0.0']);
run('AscCommon.History.Undo();');
same('border and format, undo: the flags and A1 are back', run("[flagsOf('Good'), bottomOf('A1'), describe('A1').num]"), [[false, false], run('Asc.c_oAscBorderStyles.None'), 'General']);
run('AscCommon.History.Redo();');
run("ws().getRange2('A3').setValue('1'); select('A3'); api.asc_setCellStyle('Good');");
same('border and format: applying Good now sets both', run("[bottomOf('A3'), describe('A3').num]"), [thin, '0.000']);
const borderSaved = run('api.asc_nativeGetFile()');
run = openEditor(borderSaved);
run('function flagsOf(name) { var s = api.wbModel.CellStyles.getStyleByName(name); return [s.ApplyBorder, s.ApplyNumberFormat]; }');
same('border and format, reopen: Good still applies both', run("flagsOf('Good')"), [true, true]);

// --- XfIds with a gap ---
// A predefined style comes into the document without a history item, so undoing
// a new style added before it leaves that style's XfId unused.
const GAP_HELPERS = `
  function fillOf(xfs) {
    var fill = xfs && xfs.asc_getFillColor();
    return fill ? [fill.asc_getR(), fill.asc_getG(), fill.asc_getB()].join(',') : 'none';
  }
  function styleFill(name) {
    var style = api.wbModel.CellStyles.getStyleByName(name);
    return api.wbModel.CellStyles.isCustomStyle(style) ? fillOf(style.xfs) : 'not in the document';
  }
  function cellStyleAndFill(ref) { return [describe(ref).style, describe(ref).fill]; }
  function xfIds() {
    return api.wbModel.CellStyles.CustomStyles.map(function (s) { return s.XfId; }).sort(function (a, b) { return a - b; });
  }
  // The bytes the writer gives with each XfId written as it is, as it did before
  // it wrote them as positions in cellStyleXfs.
  var cellStyleXfIndex = AscCommonExcel.StylesForWrite.prototype.getCellStyleXfIndex;
  function writeXfIdsAsTheyAre() {
    AscCommonExcel.StylesForWrite.prototype.getCellStyleXfIndex = function (xfId) { return xfId; };
    try {
      return api.asc_nativeGetFile();
    } finally {
      AscCommonExcel.StylesForWrite.prototype.getCellStyleXfIndex = cellStyleXfIndex;
    }
  }
  function writesXfIdsAsTheyAre() { return api.asc_nativeGetFile() === writeXfIdsAsTheyAre(); }
`;
function openGapEditor(data) {
  const editor = openEditor(data);
  editor(GAP_HELPERS);
  return editor;
}
run = openGapEditor(null);
run(`
  var goodFill = fillOf(api.asc_getCellStyleXfs('Good'));
  var badFill = fillOf(api.asc_getCellStyleXfs('Bad'));
  ws().getRange2('A1').setValue('1');
  var temporaryXfs = new AscCommonExcel.CellXfs();
  temporaryXfs.asc_setFillColor(color(1, 2, 3));
  api.asc_addCellStyle('Temporary', temporaryXfs);
  select('A1'); api.asc_setCellStyle('Good');
  AscCommon.History.Undo();
  AscCommon.History.Undo();
  select('A1'); api.asc_setCellStyle('Good');
`);
const goodFill = run('goodFill');
const badFill = run('badFill');
same('gap: undo leaves XfId 1 unused', run("[styleNames().indexOf('Temporary'), xfIds()]"), [-1, [0, 2]]);
same('gap: A1 has Good', run("cellStyleAndFill('A1')"), ['Good', goodFill]);
check('gap: the writer does not write the XfIds as they are', run('!writesXfIdsAsTheyAre()'), 'same bytes');
let reopened = openGapEditor(run('api.asc_nativeGetFile()'));
same('gap, reopen: Good is in the document with its fill', reopened("styleFill('Good')"), goodFill);
same('gap, reopen: A1 keeps Good', reopened("cellStyleAndFill('A1')"), ['Good', goodFill]);

// Three styles after the gap, each with its own fill.
run(`
  ws().getRange2('B1').setValue('1');
  ws().getRange2('C1').setValue('1');
  select('B1'); api.asc_setCellStyle('Bad');
  var laterXfs = new AscCommonExcel.CellXfs();
  laterXfs.asc_setFillColor(color(4, 5, 6));
  api.asc_addCellStyle('Later', laterXfs);
  select('C1'); api.asc_setCellStyle('Later');
`);
same('gap in the middle: XfId 1 unused among four styles', run('xfIds()'), [0, 2, 3, 4]);
reopened = openGapEditor(run('api.asc_nativeGetFile()'));
same('gap in the middle, reopen: each style keeps its own fill', reopened("['Good', 'Bad', 'Later'].map(styleFill)"), [goodFill, badFill, '4,5,6']);
same('gap in the middle, reopen: each cell keeps its style and fill', reopened("['A1', 'B1', 'C1'].map(cellStyleAndFill)"), [['Good', goodFill], ['Bad', badFill], ['Later', '4,5,6']]);
same('gap in the middle, reopen: the XfIds are 0..3', reopened('xfIds()'), [0, 1, 2, 3]);
reopened(`
  ['D1', 'E1', 'F1'].forEach(function (ref) { ws().getRange2(ref).setValue('1'); });
  select('D1'); api.asc_setCellStyle('Good');
  select('E1'); api.asc_setCellStyle('Bad');
  select('F1'); api.asc_setCellStyle('Later');
`);
same('gap in the middle, reopen: applying each style again gives its fill', reopened("['D1', 'E1', 'F1'].map(cellStyleAndFill)"), [['Good', goodFill], ['Bad', badFill], ['Later', '4,5,6']]);
same('gap in the middle, reopen: the next save writes the XfIds as they are', reopened('writesXfIdsAsTheyAre()'), true);

// --- Without a gap the bytes do not change ---
// A new style, a modified predefined one and a copy of a predefined one, and
// the same workbook opened from the file.
run = openGapEditor(null);
run(`
  ['A1', 'B1', 'C1', 'D1'].forEach(function (ref) { ws().getRange2(ref).setValue('1'); });
  ws().getRange2('A1').setBold(true);
  api.asc_addCellStyle('Totals', cellXfs('A1').clone());
  var noGapGood = api.asc_getCellStyleXfs('Good');
  noGapGood.asc_setFontItalic(true);
  api.asc_modifyCellStyle('Good', noGapGood);
  var noGapBad = api.asc_getCellStyleXfs('Bad');
  noGapBad.asc_setFillColor(color(7, 8, 9));
  api.asc_addCellStyle('Bad 2', noGapBad);
  select('A1'); api.asc_setCellStyle('Totals');
  select('B1'); api.asc_setCellStyle('Good');
  select('C1'); api.asc_setCellStyle('Bad 2');
  select('D1'); api.asc_setCellStyle('Neutral');
`);
same('no gap: the XfIds are 0..n-1', run('xfIds()'), [0, 1, 2, 3, 4]);
same('no gap: the bytes are the same as with the XfIds as they are', run('writesXfIdsAsTheyAre()'), true);
reopened = openGapEditor(run('api.asc_nativeGetFile()'));
same('no gap, opened from the file: the same bytes', reopened('writesXfIdsAsTheyAre()'), true);
reopened = openGapEditor(saved);
same('no gap, the workbook of the save check: the same bytes', reopened('writesXfIdsAsTheyAre()'), true);

// --- A style with an alignment it does not apply, as a file can have ---
run = openEditor(null);
run(`
  var keptXfs = new AscCommonExcel.CellXfs();
  keptXfs.asc_setHorAlign(AscCommon.align_Center);
  api.asc_addCellStyle('Kept alignment', keptXfs);
  api.wbModel.CellStyles.getStyleByName('Kept alignment').ApplyAlignment = false;
`);
run = openEditor(run('api.asc_nativeGetFile()'));
run(`
  function horOf(ref) { return (cellXfs(ref) || new AscCommonExcel.CellXfs()).getAlign2().getAlignHorizontal(); }
  function keptApplies() { return api.wbModel.CellStyles.getStyleByName('Kept alignment').ApplyAlignment; }
  ['A1', 'A2', 'A3'].forEach(function (ref) {
    ws().getRange2(ref).setValue('1');
    ws().getRange2(ref).setAlignHorizontal(AscCommon.align_Right);
  });
  select('A1'); api.asc_setCellStyle('Kept alignment');
  var keptBefore = [keptApplies(), api.asc_getCellStyleXfs('Kept alignment').getAlign2().getAlignHorizontal(), horOf('A1')];
  var keptBold = api.asc_getCellStyleXfs('Kept alignment');
  keptBold.asc_setFontBold(true);
  var keptBoldResult = api.asc_modifyCellStyle('Kept alignment', keptBold);
  select('A2'); api.asc_setCellStyle('Kept alignment');
`);
same('kept alignment: opened with its alignment, not applying it', run('keptBefore'), [false, center, right]);
same('kept alignment, bold only: A1 turns bold', run("[keptBoldResult, describe('A1').bold]"), [No, true]);
same('kept alignment, bold only: the style still does not apply alignment', run('keptApplies()'), false);
same('kept alignment, bold only: A1 keeps its right alignment', run("horOf('A1')"), right);
same('kept alignment, bold only: applying it to A2 keeps its right alignment', run("horOf('A2')"), right);
run(`
  var keptLeft = api.asc_getCellStyleXfs('Kept alignment');
  keptLeft.asc_setHorAlign(AscCommon.align_Left);
  var keptLeftResult = api.asc_modifyCellStyle('Kept alignment', keptLeft);
  select('A3'); api.asc_setCellStyle('Kept alignment');
`);
same('kept alignment, new alignment: the style applies it', run("[keptLeftResult, keptApplies(), horOf('A3')]"), [No, true, left]);

// --- A font the document has not loaded yet ---
// The cells and the gallery are drawn once the font loads; here the load waits until the test lets it end.
run = openEditor(null);
run(`
  var fontLoads = [];
  var draws = 0;
  api._loadFonts = function (fonts, callback) { fontLoads.push({ fonts: Object.keys(fonts), callback: callback }); };
  AscCommonExcel.WorksheetView.prototype.draw = function () { draws++; };
  function lastFontLoad() { return fontLoads.length ? fontLoads[fontLoads.length - 1] : { fonts: [], callback: function () {} }; }
  function drawState() { return { gallery: galleryUpdates.length, draws: draws }; }
  function drawnSince(before) { return [galleryUpdates.length > before.gallery, draws > before.draws]; }
  ws().getRange2('A1').setValue('1');
  select('A1'); api.asc_setCellStyle('Good');

  var courierXfs = api.asc_getCellStyleXfs('Good');
  courierXfs.asc_setFontName('Courier New');
  var beforeCourier = drawState();
  var courierResult = api.asc_modifyCellStyle('Good', courierXfs);
  var courierWaiting = drawnSince(beforeCourier);
  var courierLoad = lastFontLoad();
  courierLoad.callback.call(api);
  var courierLoaded = drawnSince(beforeCourier);

  var timesXfs = api.asc_getCellStyleXfs('Bad');
  timesXfs.asc_setFontName('Times New Roman');
  var beforeTimes = drawState();
  var timesResult = api.asc_addCellStyle('Bad 2', timesXfs);
  var timesWaiting = drawnSince(beforeTimes);
  var timesLoad = lastFontLoad();
  timesLoad.callback.call(api);
  var timesLoaded = drawnSince(beforeTimes);
`);
same('new font, modify: the result comes at once and the font is loaded', run('[courierResult, courierLoad.fonts]'), [No, ['Courier New']]);
same('new font, modify: no gallery or sheet drawn before the font loads', run('courierWaiting'), [false, false]);
same('new font, modify: both drawn once it loads', run('courierLoaded'), [true, true]);
same('new font, modify: A1 has the font', run("cellXfs('A1').getFont2().getName()"), 'Courier New');
same('new font, duplicate: the result comes at once and the font is loaded', run('[timesResult, timesLoad.fonts]'), [No, ['Times New Roman']]);
same('new font, duplicate: no gallery or sheet drawn before the font loads', run('timesWaiting'), [false, false]);
same('new font, duplicate: both drawn once it loads', run('timesLoaded'), [true, true]);

// --- A copy of a style applies what the original applies ---
// Good applies no number format or border, so a copy of it leaves the cell's own.
const DUPLICATE_HELPERS = `
  function appliesOf(name) {
    var s = api.wbModel.CellStyles.getStyleByName(name);
    return [s.ApplyFont, s.ApplyFill, s.ApplyBorder, s.ApplyNumberFormat, s.ApplyAlignment];
  }
  function bottomOf(ref) { return (cellXfs(ref) || new AscCommonExcel.CellXfs()).getBorder().b.s; }
  function percentCell(ref) {
    ws().getRange2(ref).setValue('0.5');
    ws().getRange2(ref).setNumFormat('0%');
  }
  function borderCell(ref) {
    ws().getRange2(ref).setValue('1');
    var borders = [];
    borders[Asc.c_oAscBorderOptions.Bottom] = {style: Asc.c_oAscBorderStyles.Thin, color: new Asc.asc_CColor(0, 0, 0)};
    var borderXfs = new AscCommonExcel.CellXfs();
    borderXfs.asc_setBorder(borders);
    ws().getRange2(ref).setBorder(borderXfs.border);
  }
`;
run = openEditor(null);
run(DUPLICATE_HELPERS);
run(`
  var goodApplies = appliesOf('Good');
  percentCell('A1');
  borderCell('A2');
  percentCell('A3');
  var copyResult = api.asc_addCellStyle('Good 2', api.asc_getCellStyleXfs('Good'), 'Good');
  select('A1:A2'); api.asc_setCellStyle('Good 2');
  var formatXfs = api.asc_getCellStyleXfs('Good');
  formatXfs.asc_setNumFormatInfo('0.000');
  var formatResult = api.asc_addCellStyle('Good 3', formatXfs, 'Good');
  select('A3'); api.asc_setCellStyle('Good 3');
`);
const goodApplies = run('goodApplies');
same('duplicate: Good applies no border or number format', goodApplies.slice(2, 4), [false, false]);
same('duplicate: the copy applies what Good applies', run("[copyResult, appliesOf('Good 2')]"), [No, goodApplies]);
same('duplicate: A1 keeps its 0%', run("[describe('A1').style, describe('A1').num]"), ['Good 2', '0%']);
same('duplicate: A2 keeps its border set by hand', run("bottomOf('A2')"), thin);
same('duplicate with a number format: the copy applies it', run("[formatResult, appliesOf('Good 3')[3], describe('A3').num]"), [No, true, '0.000']);
run = openEditor(run('api.asc_nativeGetFile()'));
run(DUPLICATE_HELPERS);
run(`
  percentCell('B1');
  select('B1'); api.asc_setCellStyle('Good 2');
`);
same('duplicate, reopen: the copy still applies what Good applies', run("appliesOf('Good 2')"), goodApplies);
same('duplicate, reopen: A1 keeps its 0% and A2 its border', run("[describe('A1').num, bottomOf('A2')]"), ['0%', thin]);
same('duplicate, reopen: applying the copy to B1 keeps its 0%', run("[describe('B1').style, describe('B1').num]"), ['Good 2', '0%']);

// --- Copies and edits of a predefined style applied once before ---
// Applying it puts a copy in the document, whose parts are other objects than
// the ones an edit builds, with the same values.
const APPLIED_HELPERS = DUPLICATE_HELPERS + `
  var NUMBER_FORMAT = 3;
  function redCell(ref) {
    ws().getRange2(ref).setValue('1');
    ws().getRange2(ref).setFillColor(AscCommonExcel.createRgbColor(255, 0, 0));
  }
  function withNumberFormat(applies) {
    var result = applies.slice();
    result[NUMBER_FORMAT] = true;
    return result;
  }
  function predefinedNames() {
    return api.wbModel.CellStyles.DefaultStyles.filter(function (s) { return 0 !== s.BuiltinId; }).map(function (s) { return s.Name; });
  }
`;
run = openEditor(null);
run(APPLIED_HELPERS);
run(`
  var appliedOnce = {};
  var mismatches = [];
  predefinedNames().forEach(function (name, index) {
    // A copy made before the style is in the document, for comparison.
    api.asc_addCellStyle(name + ' early copy', api.asc_getCellStyleXfs(name), name);
    if (JSON.stringify(appliesOf(name + ' early copy')) !== JSON.stringify(appliesOf(name))) {
      mismatches.push('early copy of ' + name + ': ' + JSON.stringify(appliesOf(name + ' early copy')));
    }
    var ref = 'Z' + (index + 1);
    ws().getRange2(ref).setValue('1');
    select(ref); api.asc_setCellStyle(name);
    appliedOnce[name] = appliesOf(name);
    api.asc_addCellStyle(name + ' copy', api.asc_getCellStyleXfs(name), name);
    if (JSON.stringify(appliesOf(name + ' copy')) !== JSON.stringify(appliedOnce[name])) {
      mismatches.push('copy of ' + name + ': ' + JSON.stringify(appliesOf(name + ' copy')));
    }
    var formatXfs = api.asc_getCellStyleXfs(name);
    formatXfs.asc_setNumFormatInfo('0.000');
    api.asc_modifyCellStyle(name, formatXfs);
    if (JSON.stringify(appliesOf(name)) !== JSON.stringify(withNumberFormat(appliedOnce[name]))) {
      mismatches.push('number format of ' + name + ': ' + JSON.stringify(appliesOf(name)));
    }
  });
  redCell('A1'); select('A1'); api.asc_setCellStyle('Percent copy');
  redCell('A2'); select('A2'); api.asc_setCellStyle('Percent');
  redCell('A3'); select('A3'); api.asc_setCellStyle('Title copy');
  redCell('A4'); select('A4'); api.asc_setCellStyle('Title');
`);
same('applied once: Percent and Title apply no fill', run("[appliedOnce['Percent'][1], appliedOnce['Title'][1]]"), [false, false]);
same('applied once: a copy without changes and a number format edit turn on only what changed', run('mismatches'), []);
same('applied once: a red cell keeps its fill with each style', run("['A1', 'A2', 'A3', 'A4'].map(function (ref) { return describe(ref).fill; })"), ['255,0,0', '255,0,0', '255,0,0', '255,0,0']);
const appliedOnce = run('appliedOnce');
run = openEditor(run('api.asc_nativeGetFile()'));
run(APPLIED_HELPERS);
run(`
  redCell('B1'); select('B1'); api.asc_setCellStyle('Percent copy');
  redCell('B2'); select('B2'); api.asc_setCellStyle('Percent');
`);
same('applied once, reopen: the copy and the edited Percent still apply no fill', run("[appliesOf('Percent copy'), appliesOf('Percent')]"), [appliedOnce['Percent'], run(`withNumberFormat(${JSON.stringify(appliedOnce['Percent'])})`)]);
same('applied once, reopen: red cells keep their fill', run("['A1', 'A2', 'B1', 'B2'].map(function (ref) { return describe(ref).fill; })"), ['255,0,0', '255,0,0', '255,0,0', '255,0,0']);

// Percent applies no font: changing only its font reaches its cells and keeps their format.
run = openEditor(null);
run(APPLIED_HELPERS);
run(`
  percentCell('A1');
  ws().getRange2('A2').setValue('0.25');
  select('A2'); api.asc_setCellStyle('Percent');
  var percentFont = api.asc_getCellStyleXfs('Percent');
  percentFont.asc_setFontName('Courier New');
  var percentFontResult = api.asc_modifyCellStyle('Percent', percentFont);
`);
same('Percent, font only: the style applies the font now', run("[percentFontResult, appliesOf('Percent')[0]]"), [No, true]);
same('Percent, font only: A2 takes the font and keeps 0%', run("[cellXfs('A2').getFont2().getName(), describe('A2').num]"), ['Courier New', '0%']);

// --- A style of the document never stands in for a predefined style ---
// The gallery and asc_setCellStyle find the document's copy of a predefined style
// by its builtinId. The real gallery code runs here, with text measuring stubbed.
const PREDEFINED_HELPERS = `
  function galleryEntries() {
    var drawn = [];
    var lastStyle = null;
    var getFont = AscCommonExcel.CCellStyle.prototype.getFont;
    AscCommonExcel.CCellStyle.prototype.getFont = function () {
      lastStyle = this;
      return getFont.call(this);
    };
    var stringRender = {
      measureString: function (fragments) {
        drawn.push({text: fragments[0].getFragmentText(), style: lastStyle});
        return {width: 0, height: 0};
      },
      getEffectiveAlign: function () { return 0; },
      render: function () {}
    };
    var fontManager = document.stubFontManager;
    var view = {model: api.wbModel, stringRender: stringRender, fmgrGraphics: [fontManager, fontManager, fontManager], m_oFont: new AscCommonExcel.Font()};
    var images;
    try {
      images = AscCommonExcel.generateCellStyles(100, 20, view);
    } finally {
      AscCommonExcel.CCellStyle.prototype.getFont = getFont;
    }
    return images.map(function (image, i) {
      return {name: image.asc_getName(), type: image.asc_getType(), text: drawn[i].text, style: drawn[i].style};
    });
  }
  function rgbOf(color) { return color ? color.getRgb() : null; }
  function bordersOf(border) {
    return ['l', 't', 'r', 'b'].map(function (side) {
      var prop = border[side];
      return prop ? prop.s + ':' + prop.getRgbOrNull() : null;
    });
  }
  function formatOf(font, fill, border, num, align) {
    return JSON.stringify([font.getName(), font.getSize(), font.getBold(), font.getItalic(), font.getUnderline(),
      rgbOf(font.getColor()), rgbOf(fill.bg()), bordersOf(border), num,
      align.getAlignHorizontal(), align.getAlignVertical(), align.getWrap()]);
  }
  function styleFormat(style) {
    var applies = [style.ApplyFont, style.ApplyFill, style.ApplyBorder, style.ApplyNumberFormat, style.ApplyAlignment];
    return formatOf(style.getFont(), style.getFill(), style.getBorder(), style.getNumFormatStr(), style.getAlign()) + JSON.stringify(applies);
  }
  function cellFormat(ref) {
    var xfs = cellXfs(ref) || new AscCommonExcel.CellXfs();
    return formatOf(xfs.getFont2(), xfs.getFill2(), xfs.getBorder() || g_oDefaultFormat.Border, xfs.getNum2().getFormat(), xfs.getAlign2());
  }
  function visiblePredefinedNames() {
    return api.wbModel.CellStyles.DefaultStyles.filter(function (s) { return !s.Hidden; }).map(function (s) { return s.Name; });
  }
  function galleryOf(type) {
    return galleryEntries().filter(function (entry) { return type === entry.type; });
  }
  // What the gallery draws for each predefined style, what applying it gives a cell
  // of its own, and the document's section of the gallery after that.
  function predefinedSnapshot() {
    var gallery = {};
    galleryOf(AscCommon.c_oAscStyleImage.Default).forEach(function (entry) {
      gallery[entry.name] = entry.text + ' ' + styleFormat(entry.style);
    });
    var applied = {};
    visiblePredefinedNames().forEach(function (name, index) {
      var ref = 'A' + (index + 1);
      ws().getRange2(ref).setValue('1');
      select(ref);
      api.asc_setCellStyle(name);
      applied[name] = describe(ref).style + ' ' + cellFormat(ref);
    });
    var documentNames = galleryOf(AscCommon.c_oAscStyleImage.Document).map(function (entry) { return entry.name; });
    return {gallery: gallery, applied: applied, documentNames: documentNames};
  }
`;
function predefinedChanges(snapshot, expected, copyName) {
  const changes = [];
  for (const part of ['gallery', 'applied']) {
    for (const name of Object.keys(expected[part])) {
      if (snapshot[part][name] !== expected[part][name]) changes.push(`${copyName} -> ${name} (${part}: ${snapshot[part][name]})`);
    }
  }
  const documentNames = JSON.stringify(snapshot.documentNames);
  if (documentNames !== JSON.stringify([copyName])) changes.push(`${copyName} -> document section ${documentNames}`);
  return changes;
}
run = openEditor(null);
run(PREDEFINED_HELPERS);
const predefinedNames = run('visiblePredefinedNames()');
const expectedPredefined = run('predefinedSnapshot()');
same('predefined styles: applying each one leaves the document section of the gallery empty', expectedPredefined.documentNames, []);
const duplicateChanges = [];
for (const name of predefinedNames) {
  const copyName = `${name} 2`;
  run = openEditor(null);
  run(PREDEFINED_HELPERS);
  run(`api.asc_addCellStyle(${JSON.stringify(copyName)}, api.asc_getCellStyleXfs(${JSON.stringify(name)}), ${JSON.stringify(name)});`);
  duplicateChanges.push(...predefinedChanges(run('predefinedSnapshot()'), expectedPredefined, copyName));
}
same(`duplicate each of the ${predefinedNames.length} predefined styles: the predefined styles draw and apply as before`, duplicateChanges, []);

// A style from the selection, as a file from another application brings, and Accent3.
run = openEditor(null);
run(PREDEFINED_HELPERS);
run(`
  ws().getRange2('A1').setValue('1');
  ws().getRange2('A1').setFillColor(AscCommonExcel.createRgbColor(255, 0, 0));
  api.asc_addCellStyle('Mine', cellXfs('A1').clone());
  var accent3Drawn = galleryOf(AscCommon.c_oAscStyleImage.Default).filter(function (entry) { return 'Accent3' === entry.name; })[0].style;
  ws().getRange2('B1').setValue('1');
  select('B1'); api.asc_setCellStyle('Accent3');
  var mineXfs = api.asc_getCellStyleXfs('Mine');
  var accent3Xfs = api.asc_getCellStyleXfs('Accent3');
  accent3Xfs.asc_setFontBold(true);
  var accent3Result = api.asc_modifyCellStyle('Accent3', accent3Xfs);
`);
same('Accent3 with a style of the document: the gallery draws Accent3', run('accent3Drawn.Name'), 'Accent3');
same('Accent3 with a style of the document: applying it gives Accent3', run("[describe('B1').style, describe('B1').fill, describe('B1').bold]"), ['Accent3', '155,187,89', true]);
same('Accent3 with a style of the document: modifying Accent3 leaves Mine as it was', run("[accent3Result, describe('A1').fill, api.asc_getCellStyleXfs('Mine').getFont2().getBold(), mineXfs.asc_getFillColor().asc_getR()]"), [No, '255,0,0', false, 255]);
run = openEditor(run('api.asc_nativeGetFile()'));
run(PREDEFINED_HELPERS);
same('Accent3, reopen: B1 keeps Accent3 and the document keeps it as a predefined style', run("[describe('B1').style, describe('B1').fill, api.wbModel.CellStyles.getStyleByName('Accent3').BuiltinId]"), ['Accent3', '155,187,89', 37]);
same('Accent3, reopen: the document section lists Mine only', run('galleryOf(AscCommon.c_oAscStyleImage.Document).map(function (entry) { return entry.name; })'), ['Mine']);

// --- Files that carry Accent3 without builtinId ---
// Editors whose preset Accent3 had no builtinId saved the copy a cell took that way.
// The real writer gives the same record once the copy has no builtinId.
const ACCENT3_FILE_HELPERS = PREDEFINED_HELPERS + `
  function documentStyles() {
    return api.wbModel.CellStyles.CustomStyles.map(function (s) { return [s.Name, s.BuiltinId]; });
  }
  function documentStyle(name) {
    return api.wbModel.CellStyles.CustomStyles.filter(function (s) { return name === s.Name; })[0];
  }
  function dropBuiltinId(name) { documentStyle(name).BuiltinId = null; }
  function renameStyle(from, to) { documentStyle(from).Name = to; }
  function addPlainStyle(name, ref) {
    ws().getRange2(ref).setValue('1');
    ws().getRange2(ref).setFillColor(AscCommonExcel.createRgbColor(0, 0, 255));
    api.asc_addCellStyle(name, cellXfs(ref).clone());
  }
  function accent3Names() {
    return documentStyles().filter(function (s) { return 'Accent3' === s[0]; });
  }
`;
run = openEditor(null);
run(ACCENT3_FILE_HELPERS);
run(`
  ['A1', 'A2'].forEach(function (ref) {
    ws().getRange2(ref).setValue('1');
    select(ref); api.asc_setCellStyle('Accent3');
  });
  var redAccent3 = api.asc_getCellStyleXfs('Accent3');
  redAccent3.asc_setFillColor(color(255, 0, 0));
  api.asc_modifyCellStyle('Accent3', redAccent3);
  dropBuiltinId('Accent3');
`);
same('old Accent3 file: written without builtinId and red', run("[documentStyles(), describe('A1').fill]"), [[['Normal', 0], ['Accent3', null]], '255,0,0']);
run = openEditor(run('api.asc_nativeGetFile()'));
run(ACCENT3_FILE_HELPERS);
run(`
  ws().getRange2('B1').setValue('1');
  select('B1'); api.asc_setCellStyle('Accent3');
  var boldAccent3 = api.asc_getCellStyleXfs('Accent3');
  boldAccent3.asc_setFontBold(true);
  var oldModifyResult = api.asc_modifyCellStyle('Accent3', boldAccent3);
  var oldCopyResult = api.asc_addCellStyle('Accent3 2', api.asc_getCellStyleXfs('Accent3'), 'Accent3');
  ws().getRange2('C1').setValue('1');
  select('C1'); api.asc_setCellStyle('Accent3 2');
`);
same('old Accent3 file, reopen: the record is Accent3 again', run('accent3Names()'), [['Accent3', 37]]);
same('old Accent3 file: applying Accent3 gives its red', run("[describe('B1').style, describe('B1').fill]"), ['Accent3', '255,0,0']);
same('old Accent3 file: modifying Accent3 reaches the cells it had', run("[oldModifyResult, describe('A1').bold, describe('A2').bold, describe('B1').bold]"), [No, true, true, true]);
same('old Accent3 file: a copy of Accent3 takes its red and bold', run("[oldCopyResult, describe('C1').style, describe('C1').fill, describe('C1').bold]"), [No, 'Accent3 2', '255,0,0', true]);
run = openEditor(run('api.asc_nativeGetFile()'));
run(ACCENT3_FILE_HELPERS);
same('old Accent3 file, saved again: one Accent3 with builtinId 37', run('accent3Names()'), [['Accent3', 37]]);
same('old Accent3 file, saved again: the cells keep Accent3 in red and bold', run("['A1', 'B1'].map(function (ref) { return [describe(ref).style, describe(ref).fill, describe(ref).bold]; })"), [['Accent3', '255,0,0', true], ['Accent3', '255,0,0', true]]);

// A file with the predefined Accent3 and a loose record named Accent3: both stay.
run = openEditor(null);
run(ACCENT3_FILE_HELPERS);
run(`
  ws().getRange2('A1').setValue('1');
  select('A1'); api.asc_setCellStyle('Accent3');
  addPlainStyle('Loose', 'B1');
  renameStyle('Loose', 'Accent3');
`);
run = openEditor(run('api.asc_nativeGetFile()'));
run(ACCENT3_FILE_HELPERS);
same('Accent3 and a loose Accent3: both stay as they were', run('accent3Names()'), [['Accent3', 37], ['Accent3', null]]);

// Several records named Accent3 without builtinId: only the first becomes Accent3.
run = openEditor(null);
run(ACCENT3_FILE_HELPERS);
run(`
  addPlainStyle('First', 'A1');
  addPlainStyle('Second', 'B1');
  renameStyle('First', 'Accent3');
  renameStyle('Second', 'Accent3');
`);
run = openEditor(run('api.asc_nativeGetFile()'));
run(ACCENT3_FILE_HELPERS);
same('two loose Accent3: only the first takes builtinId 37', run('accent3Names()'), [['Accent3', 37], ['Accent3', null]]);

// Other names stay styles of the user.
run = openEditor(null);
run(ACCENT3_FILE_HELPERS);
run(`
  addPlainStyle('Lower', 'A1');
  renameStyle('Lower', 'accent3');
  addPlainStyle('Accent3 2', 'B1');
`);
run = openEditor(run('api.asc_nativeGetFile()'));
run(ACCENT3_FILE_HELPERS);
same('accent3 and Accent3 2 keep no builtinId', run('documentStyles()'), [['Normal', 0], ['accent3', null], ['Accent3 2', null]]);

// --- The cell style dialog of web-apps on the model ---
// CellStyleDialog.js runs as it is. Its widgets are plain objects that keep a
// value and run the handlers the dialog registers; there is no DOM. Picking a
// color triggers color:select as the color button does.
const DIALOG_STUBS = `
  function FakeWidget(options) {
    this.options = options || {};
    this.handlers = {};
    this.value = undefined;
    this.pressed = false;
    this.data = this.options.data || [];
  }
  FakeWidget.prototype.on = function (event, handler) {
    (this.handlers[event] = this.handlers[event] || []).push(handler);
    return this;
  };
  FakeWidget.prototype.trigger = function (event) {
    var args = Array.prototype.slice.call(arguments, 1);
    (this.handlers[event] || []).forEach(function (handler) { handler.apply(null, args); });
  };
  FakeWidget.prototype.setValue = function (value) { this.value = value; };
  FakeWidget.prototype.getValue = function () { return this.value; };
  FakeWidget.prototype.getRawValue = function () { return String(this.value); };
  FakeWidget.prototype.setData = function (data) { this.data = data; };
  FakeWidget.prototype.fillFonts = function () {};
  FakeWidget.prototype.toggle = function (pressed) { this.pressed = pressed; };
  FakeWidget.prototype.setColor = function (color) { this.color = color; };
  FakeWidget.prototype.setMenu = function () {};
  FakeWidget.prototype.getPicker = function () {
    return { options: { transparent: this.options.transparent }, updateColors: function () {} };
  };
  function FakeCheckBox(options) {
    FakeWidget.call(this, options);
    this.value = 'unchecked';
  }
  FakeCheckBox.prototype = Object.create(FakeWidget.prototype);
  FakeCheckBox.prototype.setValue = function (value) { this.value = value ? 'checked' : 'unchecked'; };
  function DialogBase() {}
  DialogBase.prototype.initialize = function () {};
  DialogBase.prototype.render = function () {};
  DialogBase.extend = function (proto) {
    function Dialog(options) { this.initialize(options); }
    Dialog.prototype = Object.assign(Object.create(DialogBase.prototype), proto);
    return Dialog;
  };
  window.Common = {
    UI: { InputField: FakeWidget, ComboBox: FakeWidget, ComboBoxFonts: FakeWidget, Button: FakeWidget, ButtonColored: FakeWidget, CheckBox: FakeCheckBox },
    Views: { AdvancedSettingsWindow: DialogBase },
    Collections: { Fonts: FakeWidget },
    Utils: {
      String: { parseFloat: parseFloat },
      ThemeColor: {
        getEffectColors: function () { return []; },
        getStandartColors: function () { return []; },
        selectPickerColorByEffect: function () {},
        // As in apps/common/main/lib/util/utils.js.
        getHexColor: function (r, g, b) {
          return [r, g, b].map(function (v) { return ('0' + v.toString(16)).slice(-2); }).join('');
        },
        getRgbColor: function (clr) {
          var color = parseInt(((typeof clr == 'object') ? clr.color : clr).replace(/#/, ''), 16);
          var c = new Asc.asc_CColor();
          c.put_type((typeof clr == 'object' && clr.effectId !== undefined) ? Asc.c_oAscColor.COLOR_TYPE_SCHEME : Asc.c_oAscColor.COLOR_TYPE_SRGB);
          c.put_r(color >> 16);
          c.put_g((color & 0xff00) >> 8);
          c.put_b(color & 0xff);
          c.put_a(0xff);
          if (clr.effectId !== undefined) c.put_value(clr.effectId);
          return c;
        }
      }
    }
  };
  window.SSE = { Views: {} };
  window.define = function (deps, factory) { factory(); };
`;
const DIALOG_HELPERS = `
  var SIDES = ['Top', 'Bottom', 'Left', 'Right'];
  // The dialog as the toolbar opens it, rendered.
  function openDialog(isModify, name, xfs, valuesXfs) {
    var dialog = new SSE.Views.CellStyleDialog({api: api, fontStore: null, langId: 0x0409, isModify: isModify, name: name, xfs: xfs, valuesXfs: valuesXfs});
    dialog.render();
    return dialog;
  }
  function sideControls(dialog, name) {
    return dialog.borderSides.filter(function (item) { return Asc.c_oAscBorderOptions[name] === item.side; })[0];
  }
  function chooseLine(dialog, name, style) { sideControls(dialog, name).combo.setValue(Asc.c_oAscBorderStyles[style]); }
  function pickColor(button, hex) { button.trigger('color:select', button, hex); }
  function pickBorderColor(dialog, name, hex) { pickColor(sideControls(dialog, name).button, hex); }
  function lineName(style) {
    return Object.keys(Asc.c_oAscBorderStyles).filter(function (key) { return Asc.c_oAscBorderStyles[key] === style; })[0];
  }
  function buttonHex(button) { return typeof button.currentColor === 'object' ? button.currentColor.color : button.currentColor; }
  // What the dialog shows for each side: line style and color.
  function shownSides(dialog) {
    return SIDES.map(function (name) {
      var item = sideControls(dialog, name);
      return name + '=' + lineName(item.combo.getValue()) + ':' + buttonHex(item.button);
    });
  }
  // The four sides of a border: line style and color, as the file keeps them.
  function sidesOf(xfs) {
    var fields = {Top: 't', Bottom: 'b', Left: 'l', Right: 'r'};
    var border = (xfs && xfs.border) || AscCommonExcel.g_oDefaultFormat.Border;
    return SIDES.map(function (name) {
      var prop = border[fields[name]];
      if (!prop || prop.s === Asc.c_oAscBorderStyles.None) return name + '=None';
      return name + '=' + lineName(prop.s) + ':' + ('000000' + prop.c.getRgb().toString(16)).slice(-6);
    });
  }
  function styleSides(name) { return sidesOf(api.wbModel.CellStyles.getStyleByName(name).xfs); }
  function cellSides(ref) { return sidesOf(cellXfs(ref)); }
  // OK on the dialog, as the toolbar's handler does.
  function addFromDialog(dialog) {
    var settings = dialog.getSettings();
    return api.asc_addCellStyle(settings.name, settings.xfs);
  }
  function modifyFromDialog(dialog, name) { return api.asc_modifyCellStyle(name, dialog.getSettings().xfs); }
`;
const webAppsRoot = process.argv[3] || fileURLToPath(new URL('../src/web-apps/', import.meta.url));
const dialogSource = readFileSync(path.join(webAppsRoot, 'apps/spreadsheeteditor/main/app/view/CellStyleDialog.js'), 'utf8');
const underscoreSource = readFileSync(path.join(webAppsRoot, 'vendor/underscore/underscore-min.js'), 'utf8');
function openDialogEditor(data) {
  const editor = openEditor(data);
  editor(underscoreSource);
  editor(DIALOG_STUBS);
  editor(dialogSource);
  editor(DIALOG_HELPERS);
  return editor;
}
// A case that throws, as on a dialog without a control it uses, fails instead of stopping the run.
function attempt(code) {
  try {
    return run(code);
  } catch (error) {
    return 'error: ' + error.message;
  }
}

// --- Line style and color of each border side (#76) ---
run = openDialogEditor(null);
attempt(`
  ws().getRange2('A1').setValue('1');
  var frameDialog = openDialog(false, 'Frame', ws().getRange2('A1').getXfs(false).clone());
  chooseLine(frameDialog, 'Top', 'Thick');
  pickBorderColor(frameDialog, 'Top', 'ff0000');
  chooseLine(frameDialog, 'Bottom', 'Double');
  pickBorderColor(frameDialog, 'Bottom', '0000ff');
  chooseLine(frameDialog, 'Left', 'Dotted');
  pickBorderColor(frameDialog, 'Left', '00ff00');
  chooseLine(frameDialog, 'Right', 'Dashed');
  var frameResult = addFromDialog(frameDialog);
  ws().getRange2('B1').setValue('1');
  select('B1'); api.asc_setCellStyle('Frame');
`);
const frameSides = ['Top=Thick:ff0000', 'Bottom=Double:0000ff', 'Left=Dotted:00ff00', 'Right=Dashed:000000'];
same('border sides, new style: each side keeps its own line and color', attempt("[frameResult, styleSides('Frame')]"), [No, frameSides]);
same('border sides, new style: B1 takes the four sides', attempt("cellSides('B1')"), frameSides);
same('border sides, new style: the style applies its border', attempt("api.wbModel.CellStyles.getStyleByName('Frame').ApplyBorder"), true);
same('border sides, modify: the dialog opens with the borders the style has', attempt("shownSides(openDialog(true, 'Frame', api.asc_getCellStyleXfs('Frame')))"), frameSides);
attempt(`
  var untouchedXfs = api.asc_getCellStyleXfs('Frame');
  var untouchedBorder = untouchedXfs.border;
  openDialog(true, 'Frame', untouchedXfs).getSettings();
`);
same('border sides, modify without changes: the border is not written', attempt('untouchedXfs.border === untouchedBorder'), true);
attempt(`
  var pointsBeforeFrame = historyPoints();
  var frameModify = openDialog(true, 'Frame', api.asc_getCellStyleXfs('Frame'));
  chooseLine(frameModify, 'Bottom', 'Medium');
  pickBorderColor(frameModify, 'Left', 'ff8000');
  var frameModifyResult = modifyFromDialog(frameModify, 'Frame');
`);
const frameModified = ['Top=Thick:ff0000', 'Bottom=Medium:0000ff', 'Left=Dotted:ff8000', 'Right=Dashed:000000'];
same('border sides, modify one line and one color: the other sides and colors stay', attempt("[frameModifyResult, styleSides('Frame')]"), [No, frameModified]);
same('border sides, modify: one history point', attempt('historyPoints() - pointsBeforeFrame'), 1);
same('border sides, modify: B1 follows the style', attempt("cellSides('B1')"), frameModified);
attempt("ws().getRange2('B2').setValue('1'); select('B2'); api.asc_setCellStyle('Frame');");
same('border sides, modify: applying the style to B2 gives the four sides', attempt("cellSides('B2')"), frameModified);

// Good applies no border: one side turns the border on, and the font stays the theme font.
attempt(`
  ws().getRange2('A2').setValue('1');
  select('A2'); api.asc_setCellStyle('Good');
  var goodFontBefore = [api.asc_getCellStyleXfs('Good').getFont2().getName(), api.asc_getCellStyleXfs('Good').getFont2().getScheme()];
  var goodBorderDialog = openDialog(true, 'Good', api.asc_getCellStyleXfs('Good'));
  chooseLine(goodBorderDialog, 'Bottom', 'Thick');
  var goodBorderResult = modifyFromDialog(goodBorderDialog, 'Good');
  var goodStyle = api.wbModel.CellStyles.getStyleByName('Good');
`);
const bottomThick = ['Top=None', 'Bottom=Thick:000000', 'Left=None', 'Right=None'];
same('border sides, predefined Good: one side turns ApplyBorder on, the number format stays off', attempt('[goodBorderResult, goodStyle.ApplyBorder, goodStyle.ApplyNumberFormat]'), [No, true, false]);
same('border sides, predefined Good: A2 takes the bottom line only', attempt("cellSides('A2')"), bottomThick);
same('border sides, predefined Good: the font is not written', attempt('[goodStyle.xfs.getFont2().getName(), goodStyle.xfs.getFont2().getScheme()]'), attempt('goodFontBefore'));
attempt(`
  var colorOnlyDialog = openDialog(true, 'Good', api.asc_getCellStyleXfs('Good'));
  pickBorderColor(colorOnlyDialog, 'Top', 'ff0000');
  var colorOnlyShown = shownSides(colorOnlyDialog)[0];
  modifyFromDialog(colorOnlyDialog, 'Good');
`);
same('border sides, a color picked for a side without a line gives it a thin one', attempt("[colorOnlyShown, styleSides('Good')[0], cellSides('A2')[0]]"), ['Top=Thin:ff0000', 'Top=Thin:ff0000', 'Top=Thin:ff0000']);

// A line style the dialog does not list, as a file can have, is shown and kept.
attempt(`
  var hairXfs = new AscCommonExcel.CellXfs();
  hairXfs.asc_setBorderSide(Asc.c_oAscBorderOptions.Left, Asc.c_oAscBorderStyles.Hair, color(0, 0, 255));
  api.asc_addCellStyle('Hairline', hairXfs);
  var hairDialog = openDialog(true, 'Hairline', api.asc_getCellStyleXfs('Hairline'));
  var hairLeft = sideControls(hairDialog, 'Left');
  var hairShown = [lineName(hairLeft.combo.getValue()), hairLeft.combo.data[hairLeft.combo.data.length - 1].displayValue];
  chooseLine(hairDialog, 'Top', 'Thin');
  modifyFromDialog(hairDialog, 'Hairline');
`);
same('border sides, a line not in the list: shown as another style', attempt('hairShown'), ['Hair', 'Other style']);
same('border sides, a line not in the list: kept when another side changes', attempt("styleSides('Hairline')"), ['Top=Thin:000000', 'Bottom=None', 'Left=Hair:0000ff', 'Right=None']);
run = openDialogEditor(attempt('api.asc_nativeGetFile()'));
same('border sides, reopen: the style and B1 keep the four sides', attempt("[styleSides('Frame'), cellSides('B1')]"), [frameModified, frameModified]);

// --- Update a style from the selection (#75) ---
// The toolbar opens the Modify dialog of the style with the active cell's format
// (asc_getCellInfo().asc_getXfs(), the cell's xfs cloned) as valuesXfs.
run(`
  function cellValues(ref) { return ws().getRange2(ref).getXfs(false).clone(); }
  function openUpdateDialog(name, ref) { return openDialog(true, name, api.asc_getCellStyleXfs(name), cellValues(ref)); }
  function fontOf(xfs) { return [xfs.getFont2().getName(), xfs.getFont2().getScheme()]; }
  function setCellSide(ref, side, style, c) {
    var sideXfs = new AscCommonExcel.CellXfs();
    sideXfs.border = ws().getRange2(ref).getXfs(false).border;
    sideXfs.asc_setBorderSide(Asc.c_oAscBorderOptions[side], Asc.c_oAscBorderStyles[style], c);
    ws().getRange2(ref).setBorder(sideXfs.border);
  }
`);
// C5 uses Frame and has a format of its own on top: bold, yellow fill, 0.0% and a green bottom line.
// It is away from B1: a border set by hand also changes the side a neighbour shares.
attempt(`
  ws().getRange2('C5').setValue('0.5');
  select('C5'); api.asc_setCellStyle('Frame');
  ws().getRange2('C5').setBold(true);
  ws().getRange2('C5').setFillColor(AscCommonExcel.createRgbColor(255, 255, 0));
  ws().getRange2('C5').setNumFormat('0.0%');
  setCellSide('C5', 'Bottom', 'Thick', color(0, 255, 0));
  var frameFontBefore = fontOf(api.asc_getCellStyleXfs('Frame'));
  var pointsBeforeUpdate = historyPoints();
  var updateDialog = openUpdateDialog('Frame', 'C5');
  var updateShown = [updateDialog.btnBold.pressed, buttonHex(updateDialog.btnFillColor), updateDialog.format, shownSides(updateDialog)[1]];
  var beforeOk = [historyPoints() - pointsBeforeUpdate, api.asc_getCellStyleXfs('Frame').getFont2().getBold()];
  var updateResult = modifyFromDialog(updateDialog, 'Frame');
  var frameUpdated = api.asc_getCellStyleXfs('Frame');
`);
const frameFromC5 = ['Top=Thick:ff0000', 'Bottom=Thick:00ff00', 'Left=Dotted:ff8000', 'Right=Dashed:000000'];
same('update from the selection: the dialog shows the cell\'s format', attempt('updateShown'), [true, 'ffff00', '0.0%', 'Bottom=Thick:00ff00']);
same('update from the selection: nothing is saved before OK', attempt('beforeOk'), [0, false]);
same('update from the selection, OK: the style takes the cell\'s format', attempt("[updateResult, frameUpdated.getFont2().getBold(), frameUpdated.asc_getFillColor().asc_getB(), frameUpdated.getNum2().getFormat(), styleSides('Frame')]"), [No, true, 0, '0.0%', frameFromC5]);
same('update from the selection, OK: one history point', attempt('historyPoints() - pointsBeforeUpdate'), 1);
same('update from the selection, OK: the font the cell shares is not written', attempt('fontOf(frameUpdated)'), attempt('frameFontBefore'));
same('update from the selection, OK: B1, with the style only, follows', attempt("[describe('B1').bold, describe('B1').fill, describe('B1').num, cellSides('B1')]"), [true, '255,255,0', '0.0%', frameFromC5]);
attempt('AscCommon.History.Undo();');
same('update from the selection, undo: the style and B1 are back', attempt("[api.asc_getCellStyleXfs('Frame').getFont2().getBold(), describe('B1').bold, cellSides('B1')]"), [false, false, frameModified]);
attempt('AscCommon.History.Redo();');

// The cell does not need to have the style: D9 is plain with italic text.
attempt(`
  ws().getRange2('D9').setValue('1');
  ws().getRange2('D9').setItalic(true);
  var plainUpdate = modifyFromDialog(openUpdateDialog('Good', 'D9'), 'Good');
  var goodAfter = api.asc_getCellStyleXfs('Good');
`);
const noSides = ['Top=None', 'Bottom=None', 'Left=None', 'Right=None'];
same('update from a cell without the style: the style takes its format, D9 keeps no style', attempt("[plainUpdate, goodAfter.getFont2().getItalic(), !!goodAfter.getFill2().notEmpty(), styleSides('Good'), describe('D9').style]"), [No, true, false, noSides, 'Normal']);
same('update from a cell without the style: A2, with Good, follows', attempt("[describe('A2').italic, !!cellXfs('A2').getFill2().notEmpty(), cellSides('A2')]"), [true, false, noSides]);
run = openDialogEditor(attempt('api.asc_nativeGetFile()'));
same('update from the selection, reopen: Frame keeps the cell\'s format', attempt("(function (x) { return [x.getFont2().getBold(), x.getNum2().getFormat(), styleSides('Frame')]; })(api.asc_getCellStyleXfs('Frame'))"), [true, '0.0%', frameFromC5]);

const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
if (failed > 0) {
  console.log('Are the sdkjs and web-apps patches applied? node scripts/apply-patches.mjs apply');
  process.exit(1);
}
