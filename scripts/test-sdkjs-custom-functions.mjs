// Checks which custom-function library registerCustomFunctionsLibrary in
// src/sdkjs/cell/api_plugins.js loads, with the sdkjs patches applied:
//
//   - macros disabled: only the user's own library (safeJsonParse), never the
//     merge with the document's functions (mergeCustomFunctions);
//   - macros enabled: the merge, as upstream;
//   - no DocInfo (no editor settings): only the user's own library;
//   - the converter (NATIVE_EDITOR_ENJINE): only the user's own library.
//
// The function runs against stubs: nothing from a document is evaluated.
//
// Run: node scripts/apply-patches.mjs apply --only sdkjs
//      node scripts/test-sdkjs-custom-functions.mjs [path/to/api_plugins.js]
//      node scripts/apply-patches.mjs revert --only sdkjs
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const defaultPath = fileURLToPath(new URL('../src/sdkjs/cell/api_plugins.js', import.meta.url));
const source = readFileSync(process.argv[2] || defaultPath, 'utf8');

function extractFunction(text) {
  const start = text.indexOf('Api.prototype.registerCustomFunctionsLibrary = function');
  const end = text.indexOf('Api.prototype.addCustomFunctionsLibrary', start);
  if (start < 0 || end < 0) {
    throw new Error('registerCustomFunctionsLibrary not found in api_plugins.js');
  }
  return text.slice(start, end);
}

function makeContext(native) {
  const calls = { merge: 0, parse: 0, evaluated: [] };
  const userLibrary = { macrosArray: [{ value: 'user-library' }] };
  const context = {
    window: native ? { NATIVE_EDITOR_ENJINE: true } : {},
    Api: function () {},
    AscCommon: {
      getLocalStorageItem: () => 'stored',
      safePluginEval: (code) => calls.evaluated.push(code),
    },
    AscCommonExcel: {
      mergeCustomFunctions: () => {
        calls.merge++;
        return { macrosArray: [{ value: 'user-library' }, { value: 'document-functions' }] };
      },
      safeJsonParse: () => {
        calls.parse++;
        return userLibrary;
      },
    },
    customFunctionsStorageId: 'cell-custom-functions-library',
  };
  vm.createContext(context);
  return { context, calls };
}

function run(docInfo, native = false) {
  const { context, calls } = makeContext(native);
  vm.runInContext(extractFunction(source), context);
  const api = new context.Api();
  api.DocInfo = docInfo;
  api.clearCustomFunctions = () => {};
  api.recalculateCustomFunctions = () => {};
  context.Api.prototype.registerCustomFunctionsLibrary.call(api, undefined, true);
  return calls;
}

const docInfo = (enabled) => ({ asc_getIsEnabledMacroses: () => enabled });
const results = [];

function check(name, ok, detail) {
  results.push({ name, ok });
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : ` (${detail})`}`);
}

const off = run(docInfo(false));
check('macros disabled: no merge with the document', off.merge === 0, `merge called ${off.merge} times`);
check('macros disabled: the user library loads', off.parse === 1, `safeJsonParse called ${off.parse} times`);
check('macros disabled: only the user library is evaluated',
  off.evaluated.join(',') === 'user-library', off.evaluated.join(','));

const on = run(docInfo(true));
check('macros enabled: merge as upstream', on.merge === 1 && on.parse === 0, `merge ${on.merge}, parse ${on.parse}`);

const noSettings = run(null);
check('no DocInfo: no merge with the document', noSettings.merge === 0, `merge called ${noSettings.merge} times`);
check('no DocInfo: only the user library is evaluated',
  noSettings.evaluated.join(',') === 'user-library', noSettings.evaluated.join(','));

const native = run(null, true);
check('converter: no merge with the document', native.merge === 0, `merge called ${native.merge} times`);
check('converter: only the user library is evaluated',
  native.evaluated.join(',') === 'user-library', native.evaluated.join(','));

const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed} passed, ${failed} failed`);
if (failed > 0) {
  console.log('Are the sdkjs patches applied? node scripts/apply-patches.mjs apply --only sdkjs');
  process.exit(1);
}
