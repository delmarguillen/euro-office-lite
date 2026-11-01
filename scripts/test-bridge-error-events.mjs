// Checks the text _eoDescribeErrorEvent gives the [JS-ERROR] and
// [IFRAME-ERROR] log lines: script errors keep their fields and source, a
// resource that failed to load names its tag and sanitized URL, and nothing
// writes a query, document text or an "[Object keys=" placeholder.
//
// Run: node scripts/test-bridge-error-events.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const bridge = await loadBridge();
const describe = (event) => bridge.sandbox._eoDescribeErrorEvent(event);

function noLeak(text) {
  return !text.includes('?') && !text.includes('token') && !text.includes('secret')
    && !text.includes('[Object keys=');
}

class HTMLImageElement {}
class Window {}

const scriptError = {
  type: 'error',
  message: 'Uncaught TypeError: x is undefined',
  filename: 'file:///app/web-apps/apps/api/documents/api.js?ver=1#top',
  lineno: 12,
  colno: 34,
  error: { name: 'TypeError', message: 'x is undefined', stack: 'TypeError: x is undefined\n    at f (api.js:12:34)' },
};
const scriptText = describe(scriptError);
check(results, 'script error keeps name, message, stack and source',
  scriptText === 'name=TypeError message=Uncaught TypeError: x is undefined '
    + 'stack=TypeError: x is undefined\n    at f (api.js:12:34) '
    + 'source=web-apps/apps/api/documents/api.js:12:34', JSON.stringify(scriptText));

const img = Object.assign(new HTMLImageElement(), {
  tagName: 'IMG', src: 'https://example.test/a/b/images/logo.png?token=secret#frag', alt: 'secret text',
});
const imgText = describe({ type: 'error', target: img });
check(results, 'img load failure names tag and URL without query or fragment',
  imgText === 'resource=img src=images/logo.png', imgText);

const script = { tagName: 'SCRIPT', src: 'file:///app/sdkjs/word/sdk-all.js?token=secret#x', textContent: 'secret' };
const scriptLoadText = describe({ type: 'error', target: script });
check(results, 'script load failure names tag and URL without query or fragment',
  scriptLoadText === 'resource=script src=sdkjs/word/sdk-all.js', scriptLoadText);

const link = { tagName: 'LINK', href: 'file:///app/web-apps/css/app.css?v=2' };
const linkText = describe({ type: 'error', target: link });
check(results, 'link load failure uses href', linkText === 'resource=link src=web-apps/css/app.css', linkText);

const dataText = describe({ type: 'error', target: { tagName: 'IMG', src: 'data:image/png;base64,secretAAAA' } });
check(results, 'data: URL is written as its scheme only', dataText === 'resource=img src=data:', dataText);

const blobText = describe({ type: 'error', target: { tagName: 'AUDIO', src: '', currentSrc: 'BLOB:file:///1234-secret' } });
check(results, 'blob: URL is written as its scheme only', blobText === 'resource=audio src=blob:', blobText);

let emptyText;
let threw = false;
try {
  emptyText = describe({ type: 'error', target: new Window() });
} catch (error) {
  threw = true;
}
check(results, 'event with nothing gives type and target constructor',
  !threw && emptyText === 'type=error target=Window', String(emptyText));

const bareText = describe({});
check(results, 'event with no type nor target does not throw', bareText === 'type=(none) target=(none)', bareText);

const all = [scriptText, imgText, scriptLoadText, linkText, dataText, blobText, emptyText, bareText];
check(results, 'no case writes a query, document text or [Object keys=', all.every(noLeak), all.join(' | '));

bridge.stopTimers();
finish(results);
