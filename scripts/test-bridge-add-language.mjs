// Checks "Add spell check language…": an installed language is announced and
// the user dictionary list is asked for again, the "Replace?" words go to the
// backend (which asks), and a misnamed pair shows its message.
//
// Run: node scripts/test-bridge-add-language.mjs [path/to/bridge.js]
import { check, finish, loadBridge } from './bridge-test-harness.mjs';

const results = [];

// The start of sdkjs's spell.js, where the language table is defined.
const SPELL_JS = "window['AscCommon']['spellcheckGetLanguages']=function(){return "
  + '{"1033":{"hyphen":true,"name":"en_US"},"1036":{"hyphen":true,"name":"fr_FR"},'
  + '"2070":{"hyphen":true,"name":"pt_PT"},"1046":{"hyphen":true,"name":"pt_BR"},'
  + '"2049":{"hyphen":false,"name":"ar"},"3073":{"hyphen":false,"name":"ar"}}};\n'
  + "window['AscCommon'].spellcheckGetLanguages = window['AscCommon']['spellcheckGetLanguages'];\n";

// answers: what import_dictionary does on each call, in order.
async function setup(answers) {
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
  const fetched = [];
  bridge.sandbox.fetch = (url) => {
    fetched.push(url);
    return Promise.resolve({ text: () => Promise.resolve(SPELL_JS) });
  };
  const messages = [];
  const confirms = [];
  bridge.sandbox.__TAURI__.dialog = {
    message: (text, options) => { messages.push({ text, kind: options.kind }); return Promise.resolve(); },
    confirm: (text) => { confirms.push(text); return Promise.resolve(true); },
  };
  return {
    bridge,
    imports,
    fetched,
    messages,
    confirms,
    listCalls: () => listCalls,
    t: (key) => bridge.run(`_t(${JSON.stringify(key)})`),
  };
}

async function addLanguage(env) {
  const lang = await env.bridge.run('_eoAddLanguage()');
  env.bridge.stopTimers();
  return lang;
}

{
  const env = await setup(['fr_FR']);
  await addLanguage(env);
  const known = env.imports[0] && env.imports[0].known;
  check(results, 'the import sends the checker codes read from spell.js',
    JSON.stringify(known) === JSON.stringify(['ar', 'en_US', 'fr_FR', 'pt_BR', 'pt_PT']),
    JSON.stringify(known));
  check(results, 'the codes come from the sdkjs file',
    env.fetched.length === 1 && env.fetched[0] === 'sdkjs/common/spell/spell.js', JSON.stringify(env.fetched));
  check(results, 'the success message names the folder the backend chose',
    env.messages[0].text === env.t('languageAdded').replace('{lang}', 'fr_FR'));
}

{
  const env = await setup(['de_DE']);
  env.bridge.sandbox.AscDesktopEditor._editorWindow = { AscCommon: {
    spellcheckGetLanguages: () => ({ 1031: { name: 'de_DE' }, 3079: { name: 'de_AT' } }),
  } };
  await addLanguage(env);
  check(results, 'with an editor open the codes come from its table',
    JSON.stringify(env.imports[0].known) === JSON.stringify(['de_AT', 'de_DE']) && env.fetched.length === 0,
    JSON.stringify(env.imports[0].known));
}

{
  const env = await setup([new Error('dictionary-unknown-language: en (en_GB, en_US)')]);
  await addLanguage(env);
  check(results, 'an unknown language shows its message with the options',
    env.messages.length === 1 && env.messages[0].kind === 'error'
    && env.messages[0].text === env.t('languageUnknown').replace('{detail}', 'en (en_GB, en_US)'));
}

{
  const env = await setup(['es']);
  const before = env.listCalls();
  const lang = await addLanguage(env);
  await new Promise((resolve) => setTimeout(resolve, 10));
  check(results, 'an import returns the language', lang === 'es');
  check(results, 'the import is one call, with no replace flag',
    env.imports.length === 1 && !('replace' in env.imports[0]));
  check(results, 'the success message names the language',
    env.messages.length === 1 && env.messages[0].kind === 'info'
    && env.messages[0].text.includes('es') && env.messages[0].text === env.t('languageAdded').replace('{lang}', 'es'));
  check(results, 'the user dictionary list is asked for again', env.listCalls() === before + 1,
    `before ${before}, after ${env.listCalls()}`);
}

{
  const env = await setup(['es_ES']);
  await addLanguage(env);
  const question = env.imports[0] && env.imports[0].question;
  check(results, 'the import sends the translated question words',
    question && question.title === env.t('addLanguageTitle')
    && question.text === env.t('languageExists')
    && question.replace === env.t('languageReplace')
    && question.cancel === env.t('languageKeep'), JSON.stringify(question));
  check(results, 'the question keeps {lang} for the backend to fill',
    question && question.text.includes('{lang}'));
  check(results, 'the bridge never asks "Replace?" itself', env.confirms.length === 0);
}

{
  const env = await setup(['es_ES']);
  env.bridge.run("window._eoCurrentLang = 'es'");
  await addLanguage(env);
  const question = env.imports[0].question;
  check(results, 'the question is sent in Spanish',
    question.title === 'Añadir idioma' && question.replace === 'Reemplazar'
    && question.cancel === 'Cancelar' && question.text.startsWith('Ya hay un diccionario {lang}'),
    JSON.stringify(question));
}

{
  // The backend returns null when the user keeps the installed language.
  const env = await setup([null]);
  const lang = await addLanguage(env);
  check(results, 'a kept language shows nothing and installs nothing',
    lang === null && env.messages.length === 0 && env.imports.length === 1);
}

{
  const env = await setup([new Error('dictionary-busy: es_ES')]);
  await addLanguage(env);
  check(results, 'a language another window is installing shows its message',
    env.messages.length === 1 && env.messages[0].kind === 'error'
    && env.messages[0].text === env.t('languageBusy').replace('{detail}', 'es_ES'));
}

{
  const env = await setup([new Error('dictionary-io: es_ES')]);
  await addLanguage(env);
  check(results, 'a write failure shows the generic message with the language only',
    env.messages.length === 1
    && env.messages[0].text === env.t('languageFailed').replace('{detail}', 'es_ES'));
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
