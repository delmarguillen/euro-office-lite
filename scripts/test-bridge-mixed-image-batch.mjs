// Checks that every image of a pasted batch reaches LocalFileGetImageUrl (which
// copies another document's image in, or downloads a web one), also when the
// batch mixes them with data: or blob: images that the bridge resolves itself.
//
// Run: node scripts/test-bridge-mixed-image-batch.mjs [path/to/bridge.js]
import { check, finish, loadBridge, loadSdkjsSnippet } from './bridge-test-harness.mjs';

const results = [];
const bridge = await loadBridge();
const imported = [];
bridge.sandbox.AscDesktopEditor.LocalFileGetImageUrl = (url) => {
  if (url.indexOf('ascdesktop://docmedia/') === 0) {
    imported.push(url);
    return 'image1_1.png';
  }
  return url;
};

// sdkjs's desktop DocumentUrls and sendImgUrls (common/Local/common.js), the
// pair the bridge wraps in the editor.
bridge.run(`
  var AscCommon = {
    g_oDocumentUrls: {
      documentUrl: 'ascdesktop://docmedia/1000-2/2',
      getUrl: function(name) { return this.documentUrl + '/media/' + name; },
      getImageUrl: function(name) { return this.documentUrl + '/media/' + name; },
    },
  };
`);
await loadSdkjsSnippet(
  bridge,
  'src/sdkjs/common/Local/common.js',
  'AscCommon.sendImgUrls = function(api, images, callback)',
  '\tcallback(_data);\n};',
);
bridge.run('ClipboardHelper.installUrlPipelineOverrides(AscCommon);');

function send(images) {
  let answer = null;
  bridge.sandbox.__images = images;
  bridge.sandbox.__answer = (data) => { answer = data; };
  bridge.run('AscCommon.sendImgUrls({}, __images, __answer);');
  return answer;
}

const otherDocImage = 'ascdesktop://docmedia/1000-1/2/media/image1.png';
const dataImage = 'data:image/png;base64,iVBORw0KGgo=';

imported.length = 0;
let answer = send([otherDocImage]);
check(results, "a batch with only another document's image imports it", imported.length === 1,
  `imports=${imported.length}`);
check(results, '... and references the imported copy', answer && answer[0].path === 'image1_1.png',
  answer && answer[0].path);

imported.length = 0;
answer = send([dataImage, otherDocImage]);
check(results, 'the same image next to a data: image is still imported', imported.length === 1,
  `imports=${imported.length}`);
check(results, '... in batch order: data: first, untouched',
  answer && answer[0].path === dataImage && answer[0].url === dataImage);
check(results, '... then the imported copy', answer && answer[1].path === 'image1_1.png',
  answer && answer[1].path);

bridge.stopTimers();
finish(results);
