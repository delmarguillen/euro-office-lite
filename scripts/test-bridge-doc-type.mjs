// Checks which editor _eoDocTypeForPath picks for each format the Open dialog
// offers (src-tauri/src/file_dialogs.rs, OPEN_EXTENSIONS) and for those only
// opened another way (xltm, potm, ppsm).
//
// Run: node scripts/test-bridge-doc-type.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const bridge = await loadBridge();

const expected = {
  word: ['docx', 'doc', 'docm', 'dotx', 'dotm', 'odt', 'ott', 'fodt', 'rtf', 'txt'],
  cell: ['xlsx', 'xls', 'xlsm', 'xltx', 'xltm', 'xlsb', 'ods', 'ots', 'fods', 'csv'],
  slide: ['pptx', 'ppt', 'pptm', 'ppsx', 'ppsm', 'potx', 'potm', 'odp', 'otp', 'fodp'],
  pdf: ['pdf'],
};

for (const [docType, extensions] of Object.entries(expected)) {
  for (const ext of extensions) {
    const got = bridge.run(`_eoDocTypeForPath('/home/u/File.${ext.toUpperCase()}')`);
    check(results, `.${ext} opens in the ${docType} editor`, got === docType, got);
  }
}

bridge.stopTimers();
finish(results);
