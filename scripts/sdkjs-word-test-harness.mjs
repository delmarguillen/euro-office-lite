// Runs the document editor model of src/sdkjs in a node vm, for the
// scripts/test-sdkjs-*.mjs checks that need a laid out document. It loads the
// files configs/word.json lists on stubs for the browser objects, then the
// helpers the qunit tests in src/sdkjs/tests/word use (tests/word/common): a
// document without a canvas and a measurer that gives every character the same
// width. The model, the numbering, the sections, the fields and the page
// layout are real; fonts and drawing are not.
//
// sdkjs has no reader for a .docx: x2t turns it into the editor's own format,
// and x2t is not part of these checks. A check builds its document with the
// model instead.
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const defaultRoot = fileURLToPath(new URL('../src/sdkjs/', import.meta.url));

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

// The editor lays out the pages after the first ones on a timer. The vm gets
// timers that wait until runTimers is called, so a check reads a document that
// is laid out to its last page, and the script ends when its last line does.
function makeTimers() {
  const pending = new Map();
  let nextId = 1;
  return {
    setTimeout(callback) {
      pending.set(nextId, callback);
      return nextId++;
    },
    clearTimeout(id) {
      pending.delete(id);
    },
    runTimers() {
      // A timer may set another one: the limit stops one that always does.
      for (let count = 0; pending.size > 0 && count < 10000; ++count) {
        const [id, callback] = pending.entries().next().value;
        pending.delete(id);
        if (typeof callback === 'function') callback();
      }
    },
  };
}

function makeContext() {
  const timers = makeTimers();
  const context = {
    console,
    setTimeout: timers.setTimeout,
    clearTimeout: timers.clearTimeout,
    setInterval: () => 0,
    clearInterval() {},
    runTimers: timers.runTimers,
    performance,
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
    // tests/word/common/common.js adds an assertion to QUnit when it is there.
    QUnit: { assert: { close() {} } },
  };
  context.window = context;
  context.self = context;
  context.parent = context;
  context.top = context;
  vm.createContext(context);
  return context;
}

// Helpers that run inside the vm, on the model.
const HELPERS = `
  var doc = AscTest.CreateLogicDocument();
  var styles = doc.GetStyleManager();
  doc.RemoveFromContent(0, doc.GetElementsCount(), false);

  function heading(level) { return styles.GetDefaultHeading(level - 1); }
  // A paragraph at the end of the document. outlineLevel makes it a heading without a heading style.
  function addParagraph(text, styleId, outlineLevel) {
    var p = AscTest.CreateParagraph();
    doc.PushToContent(p);
    if (styleId) p.SetParagraphStyleById(styleId);
    if (undefined !== outlineLevel) p.SetOutlineLvl(outlineLevel);
    if (text) addText(p, text);
    return p;
  }
  function addText(p, text) {
    var run = new AscWord.CRun();
    p.AddToContent(p.GetElementsCount() - 1, run);
    run.AddText(text);
  }
  // A complex field (begin, instruction, separate, end) at the end of the paragraph.
  function addField(p, instruction) {
    p.MoveCursorToEndPos();
    return doc.addFieldWithInstructionToParagraph(p, instruction);
  }
  // Ends a section at this paragraph, as w:sectPr in its w:pPr does.
  // format is an Asc.c_oAscNumberingFormat; start is w:pgNumType w:start, or undefined to go on counting.
  function endSection(p, format, start) {
    var sectPr = new AscWord.SectPr(doc);
    sectPr.Copy(doc.GetFinalSectPr());
    setPageNumbers(sectPr, format, start);
    p.Set_SectionPr(sectPr);
    return sectPr;
  }
  function setPageNumbers(sectPr, format, start) {
    sectPr.Set_Type(Asc.c_oAscSectionBreakType.NextPage);
    sectPr.SetPageNumFormat(format);
    sectPr.SetPageNumStart(undefined === start ? -1 : start);
  }
  // Heading 1 to 9 numbered by one list: formats is the w:lvlText of each level ("Chapter %1.", "%1.%2.").
  function numberHeadings(formats, numFormat) {
    var numbering = doc.GetNumberingManager();
    var num = numbering.CreateNum();
    for (var level = 0; level < formats.length; ++level) {
      num.SetLvlByFormat(level, undefined === numFormat ? Asc.c_oAscNumberingFormat.Decimal : numFormat, formats[level], AscCommon.align_Left);
      num.GetLvl(level).Start = 1;
      num.GetLvl(level).SetPStyle(heading(level + 1));
      styles.Get(heading(level + 1)).SetNumPr(num.GetId(), level);
    }
    numbering.AddNum(num);
    return num;
  }
  function recalculate() {
    AscTest.Recalculate();
    runTimers();
  }
  function updateFields(fields) {
    for (var index = 0; index < fields.length; ++index) fields[index].Update(false, false);
    recalculate();
  }
  // The text of a paragraph, with | for a tab and the numbering in front.
  function textOf(p) { return p.GetText({ ParaSeparator: '', TabSymbol: '|' }); }
  function documentText() {
    var lines = [];
    for (var index = 0; index < doc.GetElementsCount(); ++index) lines.push(textOf(doc.GetElement(index)));
    return lines;
  }
`;

// Loads the editor and returns a function that runs code in it. A fresh vm
// per call, as a new editor session.
export function openWordEditor(sdkjsRoot = defaultRoot) {
  const context = makeContext();
  const config = JSON.parse(readFileSync(path.join(sdkjsRoot, 'configs/word.json'), 'utf8')).sdk;
  const files = [
    'vendor/polyfill.js',
    'vendor/xregexp-all-min.js',
    'common/applyDocumentChanges.js',
    ...config.min,
    ...config.common,
    'tests/word/common/common.js',
    'tests/word/common/editor.js',
    'tests/word/common/document.js',
    'tests/word/common/measurer.js',
  ];
  for (const file of files) {
    vm.runInContext(readFileSync(path.join(sdkjsRoot, file), 'utf8'), context, { filename: file });
  }
  vm.runInContext(HELPERS, context);
  return (code) => vm.runInContext(code, context);
}

// check and same print one line per check; finish prints the count and sets the exit code.
export function makeReport() {
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
  function finish() {
    const failed = results.filter((r) => !r.ok).length;
    console.log(`\n${results.length - failed} passed, ${failed} failed`);
    if (failed > 0) {
      console.log('Are the sdkjs patches applied? node scripts/apply-patches.mjs apply --only sdkjs');
      process.exit(1);
    }
  }
  return { check, same, finish };
}
