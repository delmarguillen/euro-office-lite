// Checks that the slideshow calls reach the backend fullscreen command, that
// a refusal is logged instead of swallowed, and that on macOS Esc in the
// slideshow keeps its default from AppKit while still reaching sdkjs.
//
// Run: node scripts/test-bridge-presentation-fullscreen.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];

async function callsWith(code, fail) {
  const calls = [];
  const bridge = await loadBridge({
    invoke: {
      set_presentation_fullscreen: (args) => {
        calls.push(args.on);
        if (fail) throw 'window refused';
      },
    },
  });
  bridge.run(code);
  await new Promise((resolve) => setTimeout(resolve, 0));
  bridge.stopTimers();
  return { calls, logs: bridge.logs };
}

const on = await callsWith('AscDesktopEditor.SetFullscreen(true)');
check(results, 'SetFullscreen(true) enters fullscreen', on.calls.join() === 'true', on.calls.join());

const off = await callsWith('AscDesktopEditor.SetFullscreen(false)');
check(results, 'SetFullscreen(false) leaves fullscreen', off.calls.join() === 'false', off.calls.join());

const end = await callsWith('AscDesktopEditor.endReporter()');
check(results, 'endReporter() leaves fullscreen', end.calls.join() === 'false', end.calls.join());

const failed = await callsWith('AscDesktopEditor.SetFullscreen(true)', true);
const logged = failed.logs.some((line) => line.includes('[presentation]') && line.includes('window refused'));
check(results, 'a refused call is logged', logged, failed.logs.join(' | '));

// An editor loaded as _loadEditorBin finds it, on `platform`. Its document
// keeps the listeners the bridge adds; `sdkjs` stands for sdkjs's own keydown
// handler, which runs after the capture listeners unless one stops the event.
async function editorOn(platform) {
  const bridge = await loadBridge({ platform });
  bridge.run(`
    function __makeEditorDoc() {
      return {
        listeners: [],
        addEventListener: function(type, fn, capture) {
          this.listeners.push({ type: type, fn: fn, capture: capture === true });
        },
      };
    }
    function __loadEditor(doc) {
      window.AscDesktopEditor._editorWindow = {
        Asc: { editor: { openDocument: function() {} } },
        document: doc,
        AscCommon: {
          g_oDocumentUrls: { documentUrl: '', getImageUrl: function() {}, getUrl: function() {} },
          OpenFileResult: function() {},
          History: {},
        },
      };
      _loadEditorBin('', 'test.pptx', { skipRecovery: true });
    }
    var __editorDoc = __makeEditorDoc();
    __loadEditor(__editorDoc);
  `);
  const sb = bridge.sandbox;
  const sdkjsCalls = [];
  const sdkjs = (e) => {
    sdkjsCalls.push(e.keyCode);
    // Esc ends the slideshow, as DemonstrationManager.End does.
    if (e.keyCode === 27) sb.AscDesktopEditor.SetFullscreen(false);
  };
  sb.__editorDoc.addEventListener('keydown', sdkjs, false);

  function press(keyCode) {
    const event = {
      keyCode,
      key: keyCode === 27 ? 'Escape' : 'a',
      defaultPrevented: false,
      stopped: false,
      preventDefault() { this.defaultPrevented = true; },
      stopPropagation() { this.stopped = true; },
    };
    const keydowns = sb.__editorDoc.listeners.filter((l) => l.type === 'keydown');
    const ordered = keydowns.filter((l) => l.capture).concat(keydowns.filter((l) => !l.capture));
    const before = sdkjsCalls.length;
    for (const listener of ordered) {
      if (event.stopped) break;
      listener.fn(event);
    }
    return { prevented: event.defaultPrevented, reachedSdkjs: sdkjsCalls.length > before };
  }

  const captureKeydowns = (doc) =>
    doc.listeners.filter((l) => l.type === 'keydown' && l.capture).length;

  return { bridge, sb, press, captureKeydowns };
}

const mac = await editorOn('MacIntel');

mac.sb.AscDesktopEditor.SetFullscreen(true);
const esc = mac.press(27);
check(results, 'macOS slideshow: Esc keeps its default from AppKit and still reaches sdkjs',
  esc.prevented && esc.reachedSdkjs, JSON.stringify(esc));

// Each check below also needs the guard to act once, so none of them passes
// on a bridge without it.
mac.sb.AscDesktopEditor.SetFullscreen(true);
const inShow = mac.press(27);
mac.sb.AscDesktopEditor.SetFullscreen(false);
const afterOff = mac.press(27);
check(results, 'macOS: SetFullscreen(false) turns the Esc guard off',
  inShow.prevented && !afterOff.prevented && afterOff.reachedSdkjs,
  JSON.stringify({ inShow, afterOff }));

mac.sb.AscDesktopEditor.SetFullscreen(true);
mac.sb.AscDesktopEditor.endReporter();
const afterEnd = mac.press(27);
mac.sb.AscDesktopEditor.SetFullscreen(true);
const backOn = mac.press(27);
check(results, 'macOS: endReporter() turns the Esc guard off',
  !afterEnd.prevented && afterEnd.reachedSdkjs && backOn.prevented,
  JSON.stringify({ afterEnd, backOn }));

mac.sb.AscDesktopEditor.SetFullscreen(true);
const otherKey = mac.press(65);
const escStill = mac.press(27);
check(results, 'macOS slideshow: other keys are left alone',
  !otherKey.prevented && otherKey.reachedSdkjs && escStill.prevented,
  JSON.stringify({ otherKey, escStill }));

mac.sb.AscDesktopEditor.SetFullscreen(false);
const outside = mac.press(27);
mac.sb.AscDesktopEditor.SetFullscreen(true);
const inside = mac.press(27);
check(results, 'macOS: Esc outside the slideshow is left alone',
  !outside.prevented && outside.reachedSdkjs && inside.prevented, JSON.stringify({ outside, inside }));

// The editor loads again in the same document, then in a new one.
mac.bridge.run('__loadEditor(__editorDoc)');
const sameDoc = mac.captureKeydowns(mac.sb.__editorDoc);
mac.bridge.run('var __newDoc = __makeEditorDoc(); __loadEditor(__newDoc)');
const newDoc = mac.captureKeydowns(mac.sb.__newDoc);
check(results, 'macOS: one Esc guard per editor document, also after a reload',
  sameDoc === 1 && newDoc === 1, `same=${sameDoc} new=${newDoc}`);
mac.bridge.stopTimers();

// Windows and Linux keep the old behaviour. The macOS half proves the guard
// exists, so the check fails without it.
const macAgain = await editorOn('MacIntel');
macAgain.sb.AscDesktopEditor.SetFullscreen(true);
const macResult = macAgain.press(27);
macAgain.bridge.stopTimers();
const untouched = [];
for (const platform of ['Win32', 'Linux x86_64']) {
  const other = await editorOn(platform);
  other.sb.AscDesktopEditor.SetFullscreen(true);
  const result = other.press(27);
  if (!result.prevented && result.reachedSdkjs) untouched.push(platform);
  other.bridge.stopTimers();
}
check(results, 'Esc is guarded on macOS only',
  macResult.prevented && untouched.length === 2,
  `mac=${JSON.stringify(macResult)} untouched=${untouched.join()}`);

finish(results);
