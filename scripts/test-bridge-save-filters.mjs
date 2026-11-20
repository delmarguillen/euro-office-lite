// Checks the filters the Save As dialog offers. On Windows and macOS the full
// list of each editor includes its template, and the dialog appends the
// extension of the filter picked. On Linux there is one filter, the format the
// document already is, a template's included. A format the editor asked for
// is the only filter everywhere.
//
// Run: node scripts/test-bridge-save-filters.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const bridge = await loadBridge();

function filters(docType, requestedExt, currentName, isLinux) {
  const args = [docType, requestedExt, currentName, isLinux].map((arg) => JSON.stringify(arg)).join(', ');
  return JSON.parse(bridge.run(`JSON.stringify(_eoSaveAsFilters(${args}))`));
}

function extensionsOf(list) {
  return list.map((filter) => filter.extensions.join('+')).join();
}

const families = [
  { docType: 'word', template: { name: 'Word template', extensions: ['dotx'] },
    formats: 'docx,odt,rtf,txt,dotx,pdf', current: 'Letter.dotx', document: 'docx' },
  { docType: 'cell', template: { name: 'Excel template', extensions: ['xltx'] },
    formats: 'xlsx,ods,csv,xltx,pdf', current: 'Budget.XLTX', document: 'xlsx' },
  { docType: 'slide', template: { name: 'PowerPoint template', extensions: ['potx'] },
    formats: 'pptx,odp,potx,pdf', current: 'Deck.potx', document: 'pptx' },
];

for (const family of families) {
  const all = filters(family.docType, null, 'Doc.' + family.document, false);
  check(results, `the ${family.docType} list on Windows and macOS is ${family.formats}`,
    extensionsOf(all) === family.formats, extensionsOf(all));
  const template = all.find((filter) => filter.extensions[0] === family.template.extensions[0]);
  check(results, `... and names its template "${family.template.name}"`,
    JSON.stringify(template) === JSON.stringify(family.template), JSON.stringify(template));

  const onLinux = filters(family.docType, null, family.current, true);
  check(results, `on Linux an open ${family.current} gets its own filter`,
    JSON.stringify(onLinux) === JSON.stringify([family.template]), JSON.stringify(onLinux));
  const documentOnLinux = filters(family.docType, null, 'Doc.' + family.document, true);
  check(results, `... and an open .${family.document} still gets its format`,
    extensionsOf(documentOnLinux) === family.document, JSON.stringify(documentOnLinux));

  const requested = family.template.extensions[0];
  for (const isLinux of [false, true]) {
    const asked = filters(family.docType, requested, 'Doc.' + family.document, isLinux);
    check(results, `a requested .${requested} is the only filter (${isLinux ? 'Linux' : 'Windows, macOS'})`,
      JSON.stringify(asked) === JSON.stringify([{ name: requested.toUpperCase(), extensions: [requested] }]),
      JSON.stringify(asked));
  }
}

const unsaved = filters('word', null, null, true);
check(results, 'on Linux a document never saved gets the first format',
  extensionsOf(unsaved) === 'docx', JSON.stringify(unsaved));

bridge.stopTimers();
finish(results);
