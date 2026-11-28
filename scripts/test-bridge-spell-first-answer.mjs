// Checks that the spellcheck bridge (bridge.js, SpellCheckBridge) logs how long
// each worker took to give its first answer, once per worker.
//
// Run: node scripts/test-bridge-spell-first-answer.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];
const FIRST_ANSWER = /^\[SPELL\] first answer of the worker (\d+) ms after it was created$/;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// The editor frame that takes the answers, and a Worker that keeps what it is
// sent until the check answers it.
const EDITOR = `
  window.__answers = [];
  window.AscDesktopEditor._editorWindow = {
    AscCommon: {
      spellcheckGetLanguages: function() { return { 1033: { name: 'en_US' }, 3082: { name: 'es_ES' } }; }
    },
    asc_nativeOnSpellCheck: function(answer) { __answers.push(answer); }
  };
  window.__workers = [];
  window.Worker = function() {
    this.posted = [];
    __workers.push(this);
  };
  Worker.prototype.postMessage = function(message) { this.posted.push(message); };
  Worker.prototype.terminate = function() {};
  window.__answerLast = function() {
    var worker = __workers[__workers.length - 1];
    var task = worker.posted[worker.posted.length - 1];
    worker.onmessage({ data: { __eoTask: task.__eoTask, usrCorrect: [true, false] } });
  };
`;
const TASK = JSON.stringify({ type: 'spell', usrWords: ['hola', 'mundoo'], usrLang: [3082, 3082], ParagraphId: 1, RecalcId: 1 });
const ASK = `SpellCheckBridge.check(${JSON.stringify(TASK)});`;

const firstAnswers = (bridge) => bridge.logs
  .map((line) => FIRST_ANSWER.exec(line))
  .filter(Boolean)
  .map((match) => Number(match[1]));

const bridge = await loadBridge({
  invoke: { list_user_dictionaries: () => ({ valid: [], skipped: [] }) },
  fetch: async () => ({ ok: true, json: async () => ['en_US', 'es_ES'] }),
});
await sleep(0);
bridge.run(EDITOR);

bridge.run(ASK);
check(results, 'the first task creates the worker', bridge.sandbox.__workers.length === 1,
  String(bridge.sandbox.__workers.length));
check(results, 'nothing is logged before the worker answers', firstAnswers(bridge).length === 0,
  firstAnswers(bridge).join());
await sleep(40);
bridge.run('__answerLast();');
const first = firstAnswers(bridge);
check(results, 'the first answer logs the time since the worker was created',
  first.length === 1 && first[0] >= 30 && first[0] < 5000, first.join());
check(results, 'the answer reaches the editor', bridge.sandbox.__answers.length === 1,
  String(bridge.sandbox.__answers.length));

bridge.run(ASK);
bridge.run('__answerLast();');
check(results, 'a later answer of the same worker logs nothing more', firstAnswers(bridge).length === 1,
  firstAnswers(bridge).join());

// The editor restarts the spellcheck: the next task builds a new worker.
bridge.run("SpellCheckBridge.check('clear');");
bridge.run(ASK);
bridge.run('__answerLast();');
check(results, 'a new worker logs its own first answer',
  bridge.sandbox.__workers.length === 2 && firstAnswers(bridge).length === 2,
  `${bridge.sandbox.__workers.length} workers, ${firstAnswers(bridge).join()}`);

bridge.stopTimers();
finish(results);
