// Checks that an image read_clipboard_image saved into media/ is used by name,
// without a copy-to-media request (which answered 500 for it).
//
// Run: node scripts/test-bridge-clipboard-image-url.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const saved = 'clipboard_1730000000000.png';
const bridge = await loadBridge({ invoke: { read_clipboard_image: () => saved } });
const requests = [];
bridge.sandbox.__xhr = (url) => {
  requests.push(url);
  return { status: 500, body: 'Copy failed' };
};

const name = await bridge.run('ClipboardHelper.readNativeClipboardImage()');
check(results, 'the native read returns the saved name', name === saved, name);

const answer = bridge.run(`AscDesktopEditor.LocalFileGetImageUrl(${JSON.stringify(saved)})`);
check(results, 'the saved clipboard image keeps its name', answer === saved, answer);
check(results, '... with no copy-to-media request', requests.length === 0, requests.join(' '));
const failures = bridge.logs.filter((line) => line.indexOf('copy-to-media failed') !== -1);
check(results, '... and no copy-to-media failure logged', failures.length === 0, failures.join(' '));

requests.length = 0;
bridge.run('AscDesktopEditor.LocalFileGetImageUrl("image1.png")');
check(results, 'any other bare name still goes to copy-to-media',
  requests.length === 1 && requests[0].indexOf('copy-to-media/') !== -1, requests.join(' '));

bridge.stopTimers();
finish(results);
