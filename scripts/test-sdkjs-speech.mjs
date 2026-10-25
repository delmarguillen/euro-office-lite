// Checks how src/sdkjs/common/workerspeech.js fills the screen reader element,
// with the sdkjs patches applied: the text goes in as text, so a string with
// tags reaches the element as those characters and never as markup. A value
// repeated on purpose still gets its alternating addon, as a node.
//
// The element and document are stubs that record what they receive.
//
// Run: node scripts/apply-patches.mjs apply --only sdkjs
//      node scripts/test-sdkjs-speech.mjs [path/to/workerspeech.js]
//      node scripts/apply-patches.mjs revert --only sdkjs
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const defaultPath = fileURLToPath(new URL('../src/sdkjs/common/workerspeech.js', import.meta.url));
const source = readFileSync(process.argv[2] || defaultPath, 'utf8');
const NO_BREAK_SPACE = String.fromCharCode(0xa0);
const TAGGED = '<b>bold</b><img src="x">';

// Records markup writes apart from text writes; children are plain objects.
function makeElement() {
  const element = { text: '', children: [], markupWrites: [] };
  Object.defineProperty(element, 'innerHTML', {
    get: () => element.text + element.children.map((child) => child.markup).join(''),
    set: (value) => {
      element.markupWrites.push(String(value));
      element.text = String(value);
      element.children = [];
    },
  });
  Object.defineProperty(element, 'textContent', {
    get: () => element.text + element.children.map((child) => child.text).join(''),
    set: (value) => {
      element.text = String(value);
      element.children = [];
    },
  });
  element.appendChild = (child) => {
    element.children.push(child);
    return child;
  };
  return element;
}

function loadWorker(isMacOs) {
  const document = {
    createElement: (tag) => ({ tag, text: '', markup: `<${tag}>` }),
    createTextNode: (text) => ({ tag: '#text', text, markup: text }),
  };
  // The file reads AscCommon both as window.AscCommon and as a global.
  const context = { document, console: { log: () => {} }, setTimeout, clearTimeout };
  context.window = context;
  context.AscCommon = { AscBrowser: { isMacOs } };
  vm.createContext(context);
  vm.runInContext(source, context);
  const worker = context.AscCommon.SpeechWorker;
  worker.speechElement = makeElement();
  return worker;
}

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok });
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : ` (${detail})`}`);
}

for (const variant of ['_setValuePermanentlyDiffEqual', '_setValuePermanently', '_setValueWithCheckEqual']) {
  const worker = loadWorker(false);
  worker[variant](TAGGED);
  const element = worker.speechElement;
  check(`${variant}: no markup written`, element.markupWrites.length === 0, element.markupWrites.join(' | '));
  check(`${variant}: the tags arrive as text`, element.text === TAGGED && element.children.length === 0,
    `text ${JSON.stringify(element.text)}, ${element.children.length} children`);
}

const repeated = loadWorker(false);
repeated._setValue(TAGGED);
repeated._setValue(TAGGED);
let children = repeated.speechElement.children;
check('repeated value: a no-break space node follows the text',
  repeated.speechElement.text === TAGGED && children.length === 1
    && children[0].tag === '#text' && children[0].text === NO_BREAK_SPACE,
  JSON.stringify(children));
repeated._setValue(TAGGED);
check('repeated again: the addon goes away', repeated.speechElement.children.length === 0,
  JSON.stringify(repeated.speechElement.children));
check('repeated value: no markup written', repeated.speechElement.markupWrites.length === 0,
  repeated.speechElement.markupWrites.join(' | '));

const mac = loadWorker(true);
mac._setValue(TAGGED);
mac._setValue(TAGGED);
children = mac.speechElement.children;
check('repeated value on macOS: a line break element follows the text',
  children.length === 1 && children[0].tag === 'br' && mac.speechElement.markupWrites.length === 0,
  `${JSON.stringify(children)}, markup ${mac.speechElement.markupWrites.join(' | ')}`);

const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
if (failed > 0) {
  console.log('Are the sdkjs patches applied? node scripts/apply-patches.mjs apply --only sdkjs');
  process.exit(1);
}
