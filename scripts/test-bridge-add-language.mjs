// Checks "Add spell check language…": an installed language is announced and
// the user dictionary list is asked for again, an existing one is replaced
// only after a confirm, and a misnamed pair shows its message.
//
// Run: node scripts/test-bridge-add-language.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];

// answers: what import_dictionary does on each call, in order.
async function setup(answers, confirmAnswer = true) {
  const imports = [];
  let listCalls = 0;
  const bridge = await loadBridge({
    invoke: {
      list_user_dictionaries: () => { listCalls += 1; return { valid: [], skipped: [] }; },
      import_dictionary: (args) => {
        imports.push(args);
        const answer = answers[imports.length - 1];
        if (answer instanceof Error) throw answer;
        return answer;
      },
    },
  });
  const messages = [];
  const confirms = [];
  bridge.sandbox.__TAURI__.dialog = {
    message: (text, options) => { messages.push({ text, kind: options.kind }); return Promise.resolve(); },
    confirm: (text) => { confirms.push(text); return Promise.resolve(confirmAnswer); },
  };
  return {
    bridge,
    imports,
    messages,
    confirms,
    listCalls: () => listCalls,
    t: (key) => bridge.run(`_t(${JSON.stringify(key)})`),
  };
}

async function addLanguage(env) {
  const lang = await env.bridge.run('_eoAddLanguage(false)');
  env.bridge.stopTimers();
  return lang;
}

{
  const env = await setup(['es']);
  const before = env.listCalls();
  const lang = await addLanguage(env);
  await new Promise((resolve) => setTimeout(resolve, 10));
  check(results, 'an import returns the language', lang === 'es');
  check(results, 'the import opens the dialog without replace',
    env.imports.length === 1 && env.imports[0].replace === false);
  check(results, 'the success message names the language',
    env.messages.length === 1 && env.messages[0].kind === 'info'
    && env.messages[0].text.includes('es') && env.messages[0].text === env.t('languageAdded').replace('{lang}', 'es'));
  check(results, 'the user dictionary list is asked for again', env.listCalls() === before + 1,
    `before ${before}, after ${env.listCalls()}`);
}

{
  const env = await setup([new Error('dictionary-exists: es'), 'es']);
  const lang = await addLanguage(env);
  check(results, 'an existing language asks before replacing', env.confirms.length === 1
    && env.confirms[0] === env.t('languageExists').replace('{lang}', 'es'));
  check(results, 'the confirmed replace is sent with replace: true',
    env.imports.length === 2 && env.imports[1].replace === true && lang === 'es');
}

{
  const env = await setup([new Error('dictionary-exists: es')], false);
  const lang = await addLanguage(env);
  check(results, 'a declined replace installs nothing',
    lang === null && env.imports.length === 1 && env.messages.length === 0);
}

{
  const env = await setup([new Error('dictionary-bad-name: es v2')]);
  const lang = await addLanguage(env);
  check(results, 'a misnamed pair shows the name error', lang === null
    && env.messages.length === 1 && env.messages[0].kind === 'error'
    && env.messages[0].text === env.t('languageBadName').replace('{detail}', 'es v2'));
}

{
  const env = await setup([new Error('could not write /x/es.aff: disk full')]);
  await addLanguage(env);
  check(results, 'any other failure shows the generic message with its text',
    env.messages.length === 1
    && env.messages[0].text === env.t('languageFailed').replace('{detail}', 'could not write /x/es.aff: disk full'));
}

{
  const env = await setup([null]);
  const lang = await addLanguage(env);
  check(results, 'a cancelled dialog shows nothing', lang === null && env.messages.length === 0);
}

finish(results);
