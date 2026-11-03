// Checks that the slideshow calls reach the backend fullscreen command and that
// a refusal is logged instead of swallowed.
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

finish(results);
