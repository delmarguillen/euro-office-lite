const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

var ClipboardHelper = (function() {
  async function readNativeClipboardImage() {
    try {
      var result = await window.__TAURI__.core.invoke('read_clipboard_image');
      return result || null;
    } catch(e) {
      window._eoLog('[EO] Clipboard image read failed: ' + (e.message || e));
      return null;
    }
  }
  function extractImageUriFromPaste(clipboardData) {
    if (!clipboardData) return null;
    var uri = clipboardData.getData('text/uri-list');
    if (!uri) return null;
    uri = uri.trim().split('\n')[0].trim();
    if (!uri) return null;
    var lower = uri.toLowerCase();
    var imageExts = ['.png', '.jpg', '.jpeg', '.gif', '.bmp', '.svg', '.webp', '.ico'];
    var isImage = false;
    for (var i = 0; i < imageExts.length; i++) {
      if (lower.endsWith(imageExts[i])) { isImage = true; break; }
    }
    if (!isImage) return null;
    if (uri.indexOf('file://') === 0) {
      uri = uri.substring(7);
      uri = decodeURIComponent(uri);
    }
    return uri;
  }
  function installUrlPipelineOverrides(AscCommon) {
    if (!AscCommon || !AscCommon.g_oDocumentUrls) return;
    var origGetUrl = AscCommon.g_oDocumentUrls.getUrl;
    AscCommon.g_oDocumentUrls.getUrl = function(strPath) {
      if (strPath && (strPath.indexOf('data:') === 0 || strPath.indexOf('blob:') === 0))
        return strPath;
      return origGetUrl.call(this, strPath);
    };
    var origGetImageUrl = AscCommon.g_oDocumentUrls.getImageUrl;
    AscCommon.g_oDocumentUrls.getImageUrl = function(strPath) {
      if (strPath && (strPath.indexOf('data:') === 0 || strPath.indexOf('blob:') === 0))
        return strPath;
      return origGetImageUrl.call(this, strPath);
    };
    if (AscCommon.sendImgUrls) {
      var origSendImgUrls = AscCommon.sendImgUrls;
      AscCommon.sendImgUrls = function(api, images, callback) {
        var hasSpecialUrls = false;
        for (var i = 0; i < images.length; i++) {
          if (images[i] && (images[i].indexOf('data:') === 0 || images[i].indexOf('blob:') === 0)) {
            hasSpecialUrls = true;
            break;
          }
        }
        if (!hasSpecialUrls) {
          return origSendImgUrls.call(this, api, images, callback);
        }
        var results = [];
        for (var i = 0; i < images.length; i++) {
          var img = images[i];
          if (img && (img.indexOf('data:') === 0 || img.indexOf('blob:') === 0)) {
            results.push({ url: img, path: img });
          } else {
            results.push({ url: AscCommon.g_oDocumentUrls.getUrl(img), path: img });
          }
        }
        callback(results);
      };
    }
  }
  return {
    readNativeClipboardImage: readNativeClipboardImage,
    extractImageUriFromPaste: extractImageUriFromPaste,
    installUrlPipelineOverrides: installUrlPipelineOverrides
  };
})();

window._eoLogBuffer = [];
window._eoLog = function() {
  var parts = [];
  for (var i = 0; i < arguments.length; i++) {
    try { parts.push(String(arguments[i])); } catch(e) { parts.push('[?]'); }
  }
  var msg = parts.join(' ');
  console.log(msg);
  window._eoLogBuffer.push(msg);
  try {
    invoke('js_log', { msg: msg });
  } catch(e) {
    try { window.top.__TAURI__.core.invoke('js_log', { msg: msg }); } catch(e2) {}
  }
};

var _isWindows = navigator.platform && navigator.platform.indexOf('Win') !== -1;
var _isMac = navigator.platform && navigator.platform.indexOf('Mac') === 0;
var ASC_PROTO_BASE = _isWindows ? 'http://ascdesktop.localhost/' : 'ascdesktop://';

// ── i18n: language detection, UI strings, translation ──

var _SUPPORTED_LANGS = [
  { code: 'ar', nativeName: 'العربية' },
  { code: 'az', nativeName: 'Azərbaycan' },
  { code: 'be', nativeName: 'Беларуская' },
  { code: 'bg', nativeName: 'Български' },
  { code: 'ca', nativeName: 'Català' },
  { code: 'cs', nativeName: 'Čeština' },
  { code: 'da', nativeName: 'Dansk' },
  { code: 'de', nativeName: 'Deutsch' },
  { code: 'el', nativeName: 'Ελληνικά' },
  { code: 'en', nativeName: 'English' },
  { code: 'es', nativeName: 'Español' },
  { code: 'eu', nativeName: 'Euskara' },
  { code: 'fi', nativeName: 'Suomi' },
  { code: 'fr', nativeName: 'Français' },
  { code: 'gl', nativeName: 'Galego' },
  { code: 'he', nativeName: 'עברית' },
  { code: 'hu', nativeName: 'Magyar' },
  { code: 'hy', nativeName: 'Հայերեն' },
  { code: 'id', nativeName: 'Bahasa Indonesia' },
  { code: 'it', nativeName: 'Italiano' },
  { code: 'ja', nativeName: '日本語' },
  { code: 'ko', nativeName: '한국어' },
  { code: 'lo', nativeName: 'ລາວ' },
  { code: 'lv', nativeName: 'Latviešu' },
  { code: 'ms', nativeName: 'Bahasa Melayu' },
  { code: 'nl', nativeName: 'Nederlands' },
  { code: 'no', nativeName: 'Norsk' },
  { code: 'pl', nativeName: 'Polski' },
  { code: 'pt', nativeName: 'Português (Brasil)' },
  { code: 'pt-pt', nativeName: 'Português (Portugal)' },
  { code: 'ro', nativeName: 'Română' },
  { code: 'ru', nativeName: 'Русский' },
  { code: 'si', nativeName: 'සිංහල' },
  { code: 'sk', nativeName: 'Slovenčina' },
  { code: 'sl', nativeName: 'Slovenščina' },
  { code: 'sq', nativeName: 'Shqip' },
  { code: 'sr', nativeName: 'Srpski' },
  { code: 'sr-cyrl', nativeName: 'Српски' },
  { code: 'sv', nativeName: 'Svenska' },
  { code: 'tr', nativeName: 'Türkçe' },
  { code: 'uk', nativeName: 'Українська' },
  { code: 'ur', nativeName: 'اردو' },
  { code: 'vi', nativeName: 'Tiếng Việt' },
  { code: 'zh', nativeName: '简体中文' },
  { code: 'zh-tw', nativeName: '繁體中文' }
];

var _UI_STRINGS = {
  en: {
    document: 'Document', spreadsheet: 'Spreadsheet', presentation: 'Presentation',
    openFile: 'Open file', newDocument: 'New document', newSpreadsheet: 'New spreadsheet',
    newPresentation: 'New presentation', unsavedChanges: 'Unsaved changes',
    unsavedDiscardOpen: 'The current document has unsaved changes. Do you want to discard them and open another file?',
    unsavedDiscardClose: 'The current document has unsaved changes. Do you want to discard them and close?',
    saveError: 'Save error', saveErrorMsg: 'Could not save the file.\n\nTarget format not compatible with this document type.',
    documents: 'Documents', all: 'All', plainText: 'Plain text', user: 'User', language: 'Language'
  },
  es: {
    document: 'Documento', spreadsheet: 'Hoja de cálculo', presentation: 'Presentación',
    openFile: 'Abrir archivo', newDocument: 'Nuevo documento', newSpreadsheet: 'Nueva hoja de cálculo',
    newPresentation: 'Nueva presentación', unsavedChanges: 'Cambios sin guardar',
    unsavedDiscardOpen: 'El documento actual tiene cambios sin guardar. ¿Desea descartarlos y abrir otro archivo?',
    unsavedDiscardClose: 'El documento actual tiene cambios sin guardar. ¿Desea descartarlos y cerrar?',
    saveError: 'Error al guardar', saveErrorMsg: 'No se pudo guardar el archivo.\n\nFormato de destino no compatible con este tipo de documento.',
    documents: 'Documentos', all: 'Todos', plainText: 'Texto plano', user: 'Usuario', language: 'Idioma'
  },
  fr: {
    document: 'Document', spreadsheet: 'Feuille de calcul', presentation: 'Présentation',
    openFile: 'Ouvrir un fichier', newDocument: 'Nouveau document', newSpreadsheet: 'Nouvelle feuille de calcul',
    newPresentation: 'Nouvelle présentation', unsavedChanges: 'Modifications non enregistrées',
    unsavedDiscardOpen: 'Le document actuel contient des modifications non enregistrées. Voulez-vous les abandonner et ouvrir un autre fichier ?',
    unsavedDiscardClose: 'Le document actuel contient des modifications non enregistrées. Voulez-vous les abandonner et fermer ?',
    saveError: 'Erreur de sauvegarde', saveErrorMsg: 'Impossible d\'enregistrer le fichier.\n\nFormat de destination incompatible avec ce type de document.',
    documents: 'Documents', all: 'Tous', plainText: 'Texte brut', user: 'Utilisateur', language: 'Langue'
  },
  de: {
    document: 'Dokument', spreadsheet: 'Tabelle', presentation: 'Präsentation',
    openFile: 'Datei öffnen', newDocument: 'Neues Dokument', newSpreadsheet: 'Neue Tabelle',
    newPresentation: 'Neue Präsentation', unsavedChanges: 'Ungespeicherte Änderungen',
    unsavedDiscardOpen: 'Das aktuelle Dokument enthält ungespeicherte Änderungen. Möchten Sie diese verwerfen und eine andere Datei öffnen?',
    unsavedDiscardClose: 'Das aktuelle Dokument enthält ungespeicherte Änderungen. Möchten Sie diese verwerfen und schließen?',
    saveError: 'Speicherfehler', saveErrorMsg: 'Die Datei konnte nicht gespeichert werden.\n\nZielformat nicht kompatibel mit diesem Dokumenttyp.',
    documents: 'Dokumente', all: 'Alle', plainText: 'Nur Text', user: 'Benutzer', language: 'Sprache'
  },
  it: {
    document: 'Documento', spreadsheet: 'Foglio di calcolo', presentation: 'Presentazione',
    openFile: 'Apri file', newDocument: 'Nuovo documento', newSpreadsheet: 'Nuovo foglio di calcolo',
    newPresentation: 'Nuova presentazione', unsavedChanges: 'Modifiche non salvate',
    unsavedDiscardOpen: 'Il documento attuale ha modifiche non salvate. Vuoi eliminarle e aprire un altro file?',
    unsavedDiscardClose: 'Il documento attuale ha modifiche non salvate. Vuoi eliminarle e chiudere?',
    saveError: 'Errore di salvataggio', saveErrorMsg: 'Impossibile salvare il file.\n\nFormato di destinazione non compatibile con questo tipo di documento.',
    documents: 'Documenti', all: 'Tutti', plainText: 'Testo normale', user: 'Utente', language: 'Lingua'
  },
  pt: {
    document: 'Documento', spreadsheet: 'Planilha', presentation: 'Apresentação',
    openFile: 'Abrir arquivo', newDocument: 'Novo documento', newSpreadsheet: 'Nova planilha',
    newPresentation: 'Nova apresentação', unsavedChanges: 'Alterações não salvas',
    unsavedDiscardOpen: 'O documento atual tem alterações não salvas. Deseja descartá-las e abrir outro arquivo?',
    unsavedDiscardClose: 'O documento atual tem alterações não salvas. Deseja descartá-las e fechar?',
    saveError: 'Erro ao salvar', saveErrorMsg: 'Não foi possível salvar o arquivo.\n\nFormato de destino não compatível com este tipo de documento.',
    documents: 'Documentos', all: 'Todos', plainText: 'Texto simples', user: 'Usuário', language: 'Idioma'
  },
  ru: {
    document: 'Документ', spreadsheet: 'Таблица', presentation: 'Презентация',
    openFile: 'Открыть файл', newDocument: 'Новый документ', newSpreadsheet: 'Новая таблица',
    newPresentation: 'Новая презентация', unsavedChanges: 'Несохранённые изменения',
    unsavedDiscardOpen: 'Текущий документ содержит несохранённые изменения. Отменить их и открыть другой файл?',
    unsavedDiscardClose: 'Текущий документ содержит несохранённые изменения. Отменить их и закрыть?',
    saveError: 'Ошибка сохранения', saveErrorMsg: 'Не удалось сохранить файл.\n\nФормат назначения несовместим с этим типом документа.',
    documents: 'Документы', all: 'Все', plainText: 'Обычный текст', user: 'Пользователь', language: 'Язык'
  },
  uk: {
    document: 'Документ', spreadsheet: 'Таблиця', presentation: 'Презентація',
    openFile: 'Відкрити файл', newDocument: 'Новий документ', newSpreadsheet: 'Нова таблиця',
    newPresentation: 'Нова презентація', unsavedChanges: 'Незбережені зміни',
    unsavedDiscardOpen: 'Поточний документ має незбережені зміни. Бажаєте скасувати їх і відкрити інший файл?',
    unsavedDiscardClose: 'Поточний документ має незбережені зміни. Бажаєте скасувати їх і закрити?',
    saveError: 'Помилка збереження', saveErrorMsg: 'Не вдалося зберегти файл.\n\nФормат призначення несумісний із цим типом документа.',
    documents: 'Документи', all: 'Усі', plainText: 'Звичайний текст', user: 'Користувач', language: 'Мова'
  },
  zh: {
    document: '文档', spreadsheet: '电子表格', presentation: '演示文稿',
    openFile: '打开文件', newDocument: '新建文档', newSpreadsheet: '新建电子表格',
    newPresentation: '新建演示文稿', unsavedChanges: '未保存的更改',
    unsavedDiscardOpen: '当前文档有未保存的更改。是否放弃更改并打开另一个文件？',
    unsavedDiscardClose: '当前文档有未保存的更改。是否放弃更改并关闭？',
    saveError: '保存错误', saveErrorMsg: '无法保存文件。\n\n目标格式与此文档类型不兼容。',
    documents: '文档', all: '所有文件', plainText: '纯文本', user: '用户', language: '语言'
  },
  ja: {
    document: 'ドキュメント', spreadsheet: 'スプレッドシート', presentation: 'プレゼンテーション',
    openFile: 'ファイルを開く', newDocument: '新規ドキュメント', newSpreadsheet: '新規スプレッドシート',
    newPresentation: '新規プレゼンテーション', unsavedChanges: '未保存の変更',
    unsavedDiscardOpen: '現在のドキュメントには未保存の変更があります。変更を破棄して別のファイルを開きますか？',
    unsavedDiscardClose: '現在のドキュメントには未保存の変更があります。変更を破棄して閉じますか？',
    saveError: '保存エラー', saveErrorMsg: 'ファイルを保存できませんでした。\n\n対象の形式はこのドキュメントタイプと互換性がありません。',
    documents: 'ドキュメント', all: 'すべて', plainText: 'プレーンテキスト', user: 'ユーザー', language: '言語'
  }
};

function _detectLang() {
  var stored = localStorage.getItem('eo-ui-lang');
  if (stored) return stored;
  var nav = (navigator.language || navigator.userLanguage || 'en').toLowerCase();
  var found = null;
  for (var i = 0; i < _SUPPORTED_LANGS.length; i++) {
    if (_SUPPORTED_LANGS[i].code === nav) { found = _SUPPORTED_LANGS[i]; break; }
  }
  if (!found) {
    var prefix = nav.split('-')[0];
    for (var i = 0; i < _SUPPORTED_LANGS.length; i++) {
      if (_SUPPORTED_LANGS[i].code === prefix) { found = _SUPPORTED_LANGS[i]; break; }
    }
  }
  var detected = found ? found.code : 'en';
  window._eoLog('[LANG] Auto-detected: ' + detected + ' (navigator: ' + nav + ')');
  return detected;
}

function _t(key) {
  var lang = window._eoCurrentLang || 'en';
  var strings = _UI_STRINGS[lang] || _UI_STRINGS['en'] || {};
  return strings[key] || (_UI_STRINGS['en'] && _UI_STRINGS['en'][key]) || key;
}

window._eoCurrentLang = _detectLang();
window._t = _t;
window._SUPPORTED_LANGS = _SUPPORTED_LANGS;
window._eoSetLang = function(code) {
  window._eoCurrentLang = code;
  localStorage.setItem('eo-ui-lang', code);
};

// ── end i18n ──

window.addEventListener('error', function(e) {
  window._eoLog('[JS-ERROR] ' + e.message + ' at ' + (e.filename || '') + ':' + (e.lineno || ''));
});
window.addEventListener('unhandledrejection', function(e) {
  window._eoLog('[JS-REJECT] ' + (e.reason && (e.reason.message || e.reason) || 'unknown'));
});
var _origConsoleError = console.error;
console.error = function() {
  var parts = [];
  for (var i = 0; i < arguments.length; i++) {
    try { parts.push(String(arguments[i])); } catch(e) { parts.push('[?]'); }
  }
  window._eoLog('[CONSOLE-ERROR] ' + parts.join(' '));
  _origConsoleError.apply(console, arguments);
};
var _origConsoleWarn = console.warn;
console.warn = function() {
  var parts = [];
  for (var i = 0; i < arguments.length; i++) {
    try { parts.push(String(arguments[i])); } catch(e) { parts.push('[?]'); }
  }
  window._eoLog('[CONSOLE-WARN] ' + parts.join(' '));
  _origConsoleWarn.apply(console, arguments);
};

function _findEditorWindow(win) {
  try { if (win.AscCommon) return win; } catch(e) {}
  for (var i = 0; i < win.frames.length; i++) {
    var found = _findEditorWindow(win.frames[i]);
    if (found) return found;
  }
  return null;
}

function _forceReload() {
  window.onbeforeunload = null;
  var iframes = document.querySelectorAll('iframe');
  for (var i = 0; i < iframes.length; i++) iframes[i].remove();
  invoke('set_document_modified', { modified: false })
    .catch(function(){})
    .finally(function() { window.location.reload(); });
}

function _getEditor() {
  var ew = window.AscDesktopEditor._editorWindow || _findEditorWindow(window);
  if (ew) window.AscDesktopEditor._editorWindow = ew;
  var editor = ew && ew.Asc && ew.Asc.editor;
  return { ew: ew, editor: editor };
}

function _loadEditorBin(b64data, fileName) {
  var ref = _getEditor();
  if (!ref.editor) return;

  try {
    var binaryStr = atob(b64data);
    var bytes = new Uint8Array(binaryStr.length);
    for (var i = 0; i < binaryStr.length; i++) {
      bytes[i] = binaryStr.charCodeAt(i);
    }

    if (ref.ew.AscCommon && ref.ew.AscCommon.g_oDocumentUrls) {
      ref.ew.AscCommon.g_oDocumentUrls.documentUrl = ASC_PROTO_BASE + 'docmedia';
      window._eoLog('[EO] documentUrl set to ' + ASC_PROTO_BASE + 'docmedia');
      var origGetImageUrl = ref.ew.AscCommon.g_oDocumentUrls.getImageUrl;
      ref.ew.AscCommon.g_oDocumentUrls.getImageUrl = function(strPath) {
        if (strPath && strPath.indexOf(ASC_PROTO_BASE) === 0)
          return strPath;
        return origGetImageUrl.call(this, strPath);
      };
    } else {
      window._eoLog('[EO] WARN: g_oDocumentUrls not available at load time');
    }

    ClipboardHelper.installUrlPipelineOverrides(ref.ew.AscCommon);

    var editorDoc = ref.ew.document;
    var isLinux = navigator.platform && navigator.platform.indexOf('Linux') !== -1;

    var cellClipboard = ref.ew.AscCommonExcel && ref.ew.AscCommonExcel.g_clipboardExcel;
    if (cellClipboard && cellClipboard.drawSelectedArea && !cellClipboard.__eoTaintGuardInstalled) {
      var origDrawSelectedArea = cellClipboard.drawSelectedArea;
      cellClipboard.drawSelectedArea = function() {
        try {
          return origDrawSelectedArea.apply(this, arguments);
        } catch(e) {
          if (e.name === 'SecurityError') {
            window._eoLog('[EO] Copy: skipped image flavor (tainted canvas)');
            return null;
          }
          throw e;
        }
      };
      cellClipboard.__eoTaintGuardInstalled = true;
    } else if ((!cellClipboard || !cellClipboard.drawSelectedArea) && window.AscDesktopEditor._currentDocType === 'cell') {
      window._eoLog('[EO] WARN: drawSelectedArea taint guard NOT installed');
    }

    if (editorDoc) {
      if (isLinux || _isMac) {
        editorDoc.addEventListener('keydown', function(e) {
          if ((e.ctrlKey || e.metaKey) && e.key === 'v' && !e.shiftKey) {
            ClipboardHelper.readNativeClipboardImage().then(function(imageFile) {
              if (imageFile) {
                window._eoLog('[EO] Clipboard image pasted: ' + imageFile);
                var ref = _getEditor();
                if (ref.editor) {
                  ref.editor.AddImageUrl([imageFile]);
                }
              }
            });
          }
        }, true);
      }

      editorDoc.addEventListener('paste', function(e) {
        var cd = e.clipboardData;
        if (!cd) return;
        var imagePath = ClipboardHelper.extractImageUriFromPaste(cd);
        if (!imagePath) return;
        e.preventDefault();
        e.stopPropagation();
        var ref = _getEditor();
        if (ref.editor) {
          ref.editor.AddImageUrl([imagePath]);
        }
      }, true);
    }

    invoke('list_media_dir').then(function(r) {
      window._eoLog('[EO] Media dir contents: ' + r);
    }).catch(function(){});

    var file = new ref.ew.AscCommon.OpenFileResult();
    file.data = bytes;
    file.bSerFormat = true;
    ref.editor.openDocument(file);
    ref.ew.AscCommon.History.UserSaveMode = true;
    _ensureCoreProps(ref);

    if (fileName) {
      var name = fileName.replace(/\\/g, '/').split('/').pop();
      invoke('set_window_title', { name: name }).catch(function(){});
    }
  } catch(e) {
    window._eoLog('[EO] Error loading document:', e.message);
  }
}

function _ensureCoreProps(ref) {
  try {
    var logicDoc = ref.editor.WordControl && ref.editor.WordControl.m_oLogicDocument;
    if (!logicDoc && ref.editor.wbModel) {
      logicDoc = ref.editor.wbModel;
    }
    if (logicDoc && !logicDoc.Core && ref.ew.AscCommon.CCore) {
      logicDoc.Core = new ref.ew.AscCommon.CCore();
      window._eoLog('[EO] Initialized empty Core properties for new document');
    }
  } catch(e) {
    window._eoLog('[EO] _ensureCoreProps error: ' + (e.message || e));
  }
}

window.AscDesktopEditor = {
  IsLocalFile: () => true,
  GetEditorId: () => 'euro-office-lite',
  CheckNeedWheel: function() { return true; },

  getFontsSprite: function(suffix) {
    suffix = suffix || '';
    return '../../../../sdkjs/common/Images/fonts_thumbnail' + suffix + '.png';
  },
  isSupportBinaryFontsSprite: false,

  _editorWindow: null,
  _currentDocType: null,
  _isModified: false,
  _isPrinting: false,

  CreateEditorApi: function(api) {
    try {
      var frames = document.querySelectorAll('iframe');
      for (var i = 0; i < frames.length; i++) {
        try {
          if (frames[i].contentWindow && frames[i].contentWindow.Asc) {
            window.AscDesktopEditor._editorWindow = frames[i].contentWindow;
            break;
          }
        } catch(e) {}
      }
    } catch(e) {}

    if (!window.AscDesktopEditor._editorWindow) {
      window.AscDesktopEditor._editorWindow = _findEditorWindow(window);
    }
  },

  LocalStartOpen: function() {
    var ref = _getEditor();
    if (!ref.editor) return;

    var doOpen = function() {
      try {
        if (window._pendingFileData) {
          var pending = window._pendingFileData;
          window._pendingFileData = null;
          _loadEditorBin(pending.data, pending.path);
        } else {
          var emptyData = ref.ew.AscCommon.getEmpty();
          var file = new ref.ew.AscCommon.OpenFileResult();
          file.data = emptyData;
          file.bSerFormat = true;
          ref.editor.openDocument(file);
          ref.ew.AscCommon.History.UserSaveMode = true;
          _ensureCoreProps(ref);
        }
      } catch(e) {
        window._eoLog('[EO] LocalStartOpen error:', e.message);
      }
    };

    setTimeout(doOpen, 100);
  },

  CheckUserId: () => 'local-user',

  OpenFilenameDialog: function(filterType, allowMultiple, callback) {
    var filterMap = {
      'images':  { name: 'Images',       extensions: ['png','jpg','jpeg','gif','bmp','svg','ico','tif','tiff','webp'] },
      'word':    { name: 'Documents',     extensions: ['docx','doc','odt','rtf','txt'] },
      'cell':    { name: 'Spreadsheets',  extensions: ['xlsx','xls','ods','csv'] },
      'video':   { name: 'Video',         extensions: ['mp4','avi','mov','wmv','mkv','webm'] },
      'audio':   { name: 'Audio',         extensions: ['mp3','wav','ogg','flac','aac','wma'] },
      'csv/txt': { name: 'CSV / Text',    extensions: ['csv','txt'] },
      '(*.xml)': { name: 'XML',           extensions: ['xml'] },
      'any':     { name: 'All files',     extensions: ['*'] },
    };
    var filter = filterMap[filterType] || filterMap['any'];
    var dialog = window.__TAURI__.dialog;
    dialog.open({
      multiple: !!allowMultiple,
      filters: [filter, { name: 'All files', extensions: ['*'] }]
    }).then(function(result) {
      if (result === null) return;
      if (callback) callback(result);
    }).catch(function(e) {
      window._eoLog('[EO] OpenFilenameDialog error: ' + (e.message || e));
    });
  },

  LocalFileOpen: async function(path) {
    if (!path) {
      var dialog = window.__TAURI__.dialog;
      path = await dialog.open({
        filters: [
          { name: _t('documents'), extensions: ['docx', 'xlsx', 'pptx', 'odt', 'ods', 'odp', 'rtf', 'txt', 'csv', 'pdf'] },
          { name: _t('all'), extensions: ['*'] }
        ]
      });
    }
    if (!path) return;

    if (window.AscDesktopEditor._currentDocType) {
      if (window.AscDesktopEditor._isModified) {
        var discard = await window.__TAURI__.dialog.confirm(
          _t('unsavedDiscardOpen'),
          { title: _t('unsavedChanges'), kind: 'warning' }
        );
        if (!discard) return;
      }
      window._eoLog('[EO] LocalFileOpen: editor active, reloading with pending path: ' + path);
      localStorage.setItem('eo-pending-open-path', path);
      _forceReload();
      return;
    }

    try {
      var b64data = await invoke('open_file', { path: path });
      _loadEditorBin(b64data, path);
    } catch(e) {
      window._eoLog('[EO] Error opening file: ' + e);
    }
  },

  LocalFileSave: async function(param, password, docinfo, fileType, jsonOptions) {
    var isSaveAs = param && param.indexOf('saveas=true') !== -1;

    if (isSaveAs && fileType === 513 && !window.AscDesktopEditor._isPrinting) {
      window.AscDesktopEditor.Print();
      return;
    }
    if (window.AscDesktopEditor._isPrinting) return;

    var ref = _getEditor();
    if (!ref.editor) return;

    try {
      var binData = ref.editor.asc_nativeGetFile();
      if (!binData) return;

      var b64;
      if (typeof binData === 'string') {
        b64 = btoa(binData);
      } else {
        var binary = '';
        var bytes = new Uint8Array(binData);
        for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
        b64 = btoa(binary);
      }

      await invoke('write_editor_bin', { data: b64 });

      var currentPath = await invoke('get_current_path');
      if (!isSaveAs && !currentPath) isSaveAs = true;

      if (isSaveAs) {
        var docType = window.AscDesktopEditor._currentDocType || 'word';

        var filters;
        if (docType === 'cell') {
          filters = [
            { name: 'Excel', extensions: ['xlsx'] },
            { name: 'OpenDocument Spreadsheet', extensions: ['ods'] },
            { name: 'CSV', extensions: ['csv'] },
            { name: 'PDF', extensions: ['pdf'] },
          ];
        } else if (docType === 'slide') {
          filters = [
            { name: 'PowerPoint', extensions: ['pptx'] },
            { name: 'OpenDocument Presentation', extensions: ['odp'] },
            { name: 'PDF', extensions: ['pdf'] },
          ];
        } else {
          filters = [
            { name: 'Word', extensions: ['docx'] },
            { name: 'OpenDocument Text', extensions: ['odt'] },
            { name: 'Rich Text', extensions: ['rtf'] },
            { name: _t('plainText'), extensions: ['txt'] },
            { name: 'PDF', extensions: ['pdf'] },
          ];
        }
        var dialog = window.__TAURI__.dialog;
        var savePath = await dialog.save({ filters: filters });

        if (savePath) {
          var knownExts = ['docx','doc','odt','rtf','txt','xlsx','xls','ods','csv','pptx','ppt','odp','pdf'];
          var pathExt = savePath.split('.').pop().toLowerCase();
          if (savePath.indexOf('.') === -1 || knownExts.indexOf(pathExt) === -1) {
            savePath += '.' + filters[0].extensions[0];
          }
          try {
            await invoke('save_file_as', { path: savePath });
            var savedName = savePath.replace(/\\/g, '/').split('/').pop();
            invoke('set_window_title', { name: savedName }).catch(function(){});
            try {
              var frames = document.querySelectorAll('iframe');
              for (var fi = 0; fi < frames.length; fi++) {
                try {
                  var titleInput = frames[fi].contentDocument.querySelector('#title-doc-name');
                  if (titleInput) titleInput.value = savedName;
                  var ribInput = frames[fi].contentDocument.querySelector('#rib-doc-name');
                  if (ribInput) ribInput.value = savedName;
                } catch(te) {}
              }
            } catch(te) {}
          } catch(saveErr) {
            window._eoLog('[EO] SaveAs failed: ' + saveErr);
            await window.__TAURI__.dialog.message(
              _t('saveErrorMsg'),
              { title: _t('saveError'), kind: 'error' }
            );
            if (ref.ew && ref.ew.DesktopOfflineAppDocumentEndSave) {
              ref.ew.DesktopOfflineAppDocumentEndSave(1);
            }
            return;
          }
        }
      } else {
        await invoke('save_file', { data: '' });
      }

      if (ref.ew.DesktopOfflineAppDocumentEndSave) {
        ref.ew.DesktopOfflineAppDocumentEndSave(0);
      }
    } catch(e) {
      window._eoLog('[EO] Error saving file:', e);
      if (ref.ew && ref.ew.DesktopOfflineAppDocumentEndSave) {
        ref.ew.DesktopOfflineAppDocumentEndSave(1);
      }
    }
  },

  LocalFileCreate: async (type) => {
    return await invoke('create_new', { docType: type });
  },

  DownloadFiles: function(urls, otherParams, callback) {
    if (!urls || !urls.length) {
      if (callback) callback({});
      return;
    }
    var fileMap = {};
    var pending = urls.length;
    urls.forEach(function(url) {
      try {
        var xhr = new XMLHttpRequest();
        xhr.open('GET', ASC_PROTO_BASE + 'download-to-media/' + encodeURIComponent(url), false);
        xhr.send(null);
        if (xhr.status === 200 && xhr.responseText) {
          fileMap[url] = xhr.responseText;
        } else {
          window._eoLog('[EO] DownloadFiles failed for ' + url + ': status ' + xhr.status);
        }
      } catch(e) {
        window._eoLog('[EO] DownloadFiles error for ' + url + ': ' + (e.message || e));
      }
      pending--;
      if (pending === 0 && callback) callback(fileMap);
    });
  },

  convertFile: function(filePath, targetFormat, callback) {
    invoke('convert_for_insert', { path: filePath }).then(function(result) {
      var binB64 = result.data;
      var raw = atob(binB64);
      var bytes = new Uint8Array(raw.length);
      for (var i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);

      var fileObj = {
        _data: bytes,
        _images: result.images || {},
        get: function() { return this._data; },
        getImages: function() { return this._images; },
        close: function() { this._data = null; this._images = null; }
      };
      if (callback) callback(fileObj);
    }).catch(function(e) {
      window._eoLog('[EO] convertFile error: ' + (e.message || e));
      if (callback) callback(null);
    });
  },

  CompareDocumentFile: function(file, oOptions) {
    invoke('convert_for_insert', { path: file }).then(function(result) {
      var ref = _getEditor();
      if (ref.ew && ref.ew.onDocumentCompare) {
        ref.ew.onDocumentCompare('', result.data, result.data.length, result.images || {}, oOptions);
      }
    }).catch(function(e) {
      window._eoLog('[EO] CompareDocumentFile error: ' + (e.message || e));
    });
  },

  CompareDocumentUrl: function(file, oOptions) {
    invoke('convert_for_insert', { path: file }).then(function(result) {
      var ref = _getEditor();
      if (ref.ew && ref.ew.onDocumentCompare) {
        ref.ew.onDocumentCompare('', result.data, result.data.length, result.images || {}, oOptions);
      }
    }).catch(function(e) {
      window._eoLog('[EO] CompareDocumentUrl error: ' + (e.message || e));
    });
  },

  MergeDocumentFile: function(file, oOptions) {
    invoke('convert_for_insert', { path: file }).then(function(result) {
      var ref = _getEditor();
      if (ref.ew && ref.ew.onDocumentMerge) {
        ref.ew.onDocumentMerge('', result.data, result.data.length, result.images || {}, oOptions);
      }
    }).catch(function(e) {
      window._eoLog('[EO] MergeDocumentFile error: ' + (e.message || e));
    });
  },

  MergeDocumentUrl: function(file, oOptions) {
    invoke('convert_for_insert', { path: file }).then(function(result) {
      var ref = _getEditor();
      if (ref.ew && ref.ew.onDocumentMerge) {
        ref.ew.onDocumentMerge('', result.data, result.data.length, result.images || {}, oOptions);
      }
    }).catch(function(e) {
      window._eoLog('[EO] MergeDocumentUrl error: ' + (e.message || e));
    });
  },

  LocalFileGetSourcePath: () => '',
  LocalFileGetSaved: () => false,
  LocalFileGetImageUrl: function(url) {
    if (!url) return url;
    if (url.indexOf('data:') === 0 || url.indexOf('blob:') === 0) return url;
    if (url.indexOf(ASC_PROTO_BASE) === 0) return url;
    var protocol = (url.indexOf('http://') === 0 || url.indexOf('https://') === 0)
      ? 'download-to-media' : 'copy-to-media';
    try {
      var xhr = new XMLHttpRequest();
      xhr.open('GET', ASC_PROTO_BASE + protocol + '/' + encodeURIComponent(url), false);
      xhr.send(null);
      if (xhr.status === 200 && xhr.responseText) {
          var result = xhr.responseText;
          if (protocol === 'download-to-media') {
            result = result.replace(/\\/g, '/').split('/').pop();
          }
          return result;
      }
    } catch(e) {
      window._eoLog('[EO] LocalFileGetImageUrl error: ' + (e.message || e));
    }
    return url;
  },
  LocalFileGetModified: function() { return window.AscDesktopEditor._isModified; },
  LocalFileSetModified: function(modified) {
    window.AscDesktopEditor._isModified = modified;
    invoke('set_document_modified', { modified }).catch(function(){});
  },

  GetOpenedFile: function(data) { return null; },

  Copy: function() {
    if (!_isWindows) return;
    var ref = _getEditor();
    if (!ref.ew) { window._eoLog('[EO] WARN: Copy - editor context not available'); return; }
    var cb = ref.ew.AscCommon && ref.ew.AscCommon.g_clipboardBase;
    if (cb) {
      if (cb.inputContext && cb.inputContext.HtmlArea) cb.inputContext.HtmlArea.focus();
      if (cb.CommonDiv_Execute_CopyCut) cb.CommonDiv_Execute_CopyCut();
    }
    try {
      var ok = ref.ew.document.execCommand('copy');
      window._eoLog('[EO] Copy: execCommand=' + ok);
    } catch(e) {
      window._eoLog('[EO] Copy: execCommand=error ' + (e.message || e));
    }
  },
  Paste: async () => {
    var platform = _isWindows ? 'win' : _isMac ? 'mac' : 'linux';
    var order = _isMac ? 'image-first' : 'text-first';
    window._eoLog('[EO] Paste: start (platform=' + platform + ', order=' + order + ')');
    var imageFile = null;
    var text = null;
    var probeImage = async function() {
      try {
        imageFile = await ClipboardHelper.readNativeClipboardImage();
        window._eoLog('[EO] Paste: image probe -> ' + (imageFile ? 'found (' + imageFile + ')' : 'none'));
      } catch(e) {
        window._eoLog('[EO] Paste: image probe -> error ' + (e.message || e));
      }
    };
    var probeText = async function() {
      try {
        text = await invoke('read_clipboard_text');
        window._eoLog('[EO] Paste: text probe -> ' + (text ? 'found (len=' + text.length + ')' : 'none'));
      } catch(e) {
        window._eoLog('[EO] Paste: text probe -> error ' + (e.message || e));
      }
    };
    if (_isMac) {
      await probeImage();
      if (!imageFile) await probeText();
    } else {
      await probeText();
      if (!text) await probeImage();
    }
    if (imageFile && (!text || _isMac)) {
      try {
        var ref = _getEditor();
        if (ref.editor) {
          ref.editor.AddImageUrl([imageFile]);
          window._eoLog('[EO] Paste: branch=image -> AddImageUrl ok');
        }
      } catch(e) {
        window._eoLog('[EO] Paste: branch=image -> error ' + (e.message || e));
      }
      return;
    }
    if (text) {
      try {
        var ref = _getEditor();
        if (ref.editor && ref.ew && ref.ew.AscCommon) {
          ref.editor.asc_PasteData(ref.ew.AscCommon.c_oAscClipboardDataFormat.Text, text);
          window._eoLog('[EO] Paste: branch=text -> asc_PasteData ok');
        }
      } catch(e) {
        window._eoLog('[EO] Paste: branch=text -> error ' + (e.message || e));
      }
      return;
    }
    window._eoLog('[EO] Paste: branch=none -> empty clipboard');
  },
  Cut: function() {
    if (!_isWindows) return;
    var ref = _getEditor();
    if (!ref.ew) { window._eoLog('[EO] WARN: Cut - editor context not available'); return; }
    var cb = ref.ew.AscCommon && ref.ew.AscCommon.g_clipboardBase;
    if (cb) {
      if (cb.inputContext && cb.inputContext.HtmlArea) cb.inputContext.HtmlArea.focus();
      if (cb.CommonDiv_Execute_CopyCut) cb.CommonDiv_Execute_CopyCut();
    }
    try {
      var ok = ref.ew.document.execCommand('cut');
      window._eoLog('[EO] Cut: execCommand=' + ok);
    } catch(e) {
      window._eoLog('[EO] Cut: execCommand=error ' + (e.message || e));
    }
  },

  _pendingPrinter: null,

  Print: async function(optionsJson) {
    var printerName = window.AscDesktopEditor._pendingPrinter;
    if (optionsJson) {
      try {
        var parsed = JSON.parse(optionsJson);
        if (parsed.nativeOptions && parsed.nativeOptions.printer) {
          printerName = parsed.nativeOptions.printer;
        }
      } catch(e) {}
    }

    var ref = _getEditor();
    if (!ref.editor) return;

    try {
      window.AscDesktopEditor._isPrinting = true;
      var binData = ref.editor.asc_nativeGetFile();
      if (!binData) return;

      var b64;
      if (typeof binData === 'string') {
        b64 = btoa(binData);
      } else {
        var binary = '';
        var bytes = new Uint8Array(binData);
        for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
        b64 = btoa(binary);
      }
      await invoke('write_editor_bin', { data: b64 });
      var pdfPath = await invoke('print_document');

      if (printerName) {
        var printResult = await invoke('plugin:printer|print_pdf', {
          id: printerName,
          path: pdfPath,
          printer: printerName,
          print_settings: '{}',
          remove_after_print: true
        });
      } else {
        await invoke('open_pdf_viewer', { path: pdfPath });
      }

      if (ref.ew && ref.ew.DesktopOfflineAppDocumentEndSave) {
        ref.ew.DesktopOfflineAppDocumentEndSave(0);
      }
    } catch(e) {
      window._eoLog('[EO] Print: ERROR: ' + (e.message || e));
      if (ref.ew && ref.ew.DesktopOfflineAppDocumentEndSave) {
        ref.ew.DesktopOfflineAppDocumentEndSave(1);
      }
    } finally {
      window.AscDesktopEditor._isPrinting = false;
    }
  },
  IsSupportNativePrint: () => true,

  onDocumentModifiedChanged: function(modified) {
    window.AscDesktopEditor._isModified = modified;
    invoke('set_document_modified', { modified }).catch(function(){});
  },
  getViewportSettings: function() {
    return { widgetType: 'window' };
  },

  SetDocumentName: (name) => {
    invoke('set_window_title', { name }).catch(function(){});
  },

  execCommand: function(cmd, param) {
    if (cmd === 'saveas') {
      window.AscDesktopEditor.LocalFileSave('saveas=true;', '', undefined, 0, '{}');
    } else if (cmd === 'title:button') {
      try {
        var btn = JSON.parse(param);
        if (btn.click === 'home') {
          (async function() {
            if (window.AscDesktopEditor._isModified) {
              var discard = await window.__TAURI__.dialog.confirm(
                _t('unsavedDiscardClose'),
                { title: _t('unsavedChanges'), kind: 'warning' }
              );
              if (!discard) return;
            }
            _forceReload();
          })();
        }
      } catch(e) {}
    } else if (cmd === 'go:folder') {
      (async function() {
        if (window.AscDesktopEditor._isModified) {
          var discard = await window.__TAURI__.dialog.confirm(
            _t('unsavedDiscardClose'),
            { title: _t('unsavedChanges'), kind: 'warning' }
          );
          if (!discard) return;
        }
        _forceReload();
      })();
    } else if (cmd === 'editor:event') {
      try {
        var evt = JSON.parse(param);
        if (evt.action === 'file:open') {
          window.AscDesktopEditor.LocalFileOpen();
        } else if (evt.action === 'file:close') {
          (async function() {
            if (window.AscDesktopEditor._isModified) {
              var discard = await window.__TAURI__.dialog.confirm(
                _t('unsavedDiscardClose'),
                { title: _t('unsavedChanges'), kind: 'warning' }
              );
              if (!discard) return;
            }
            _forceReload();
          })();
        }
      } catch(e) {
        window._eoLog('[EO] execCommand parse error: ' + e.message);
      }
    }
    return '';
  },

  LoadFontBase64: function(fontId) {
    try {
      var xhr = new XMLHttpRequest();
      xhr.open('GET', '/fonts/' + fontId, false);
      xhr.responseType = 'arraybuffer';
      xhr.send(null);
      if (xhr.status === 200) {
        var bytes = new Uint8Array(xhr.response);
        var binary = '';
        for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
        window[fontId] = bytes.length + ';' + btoa(binary);
      }
    } catch(e) {}
  },

  LocalFileSaveChanges: function(changes, deleteIndex, count) {
    invoke('save_changes', {
      changes: typeof changes === 'string' ? changes : JSON.stringify(changes),
      deleteIndex: deleteIndex,
      count: count
    }).catch(function(){});
  },

  OnSave: function() {},

  GetInstallPlugins: () => JSON.stringify([
    { url: '', pluginsData: [] },
    { url: '', pluginsData: [] }
  ]),

  IsSignaturesSupport: () => false,
  IsProtectionSupport: () => false,
  isBlockchainSupport: () => false,
  SpellCheck: function() {},
  SetFullscreen: function(fullscreen) {
    var win = window.__TAURI__.window.getCurrentWindow();
    var fs = !!fullscreen;
    win.setFullscreen(fs).catch(function(){});
    win.setAlwaysOnTop(fs).catch(function(){});
  },
  endReporter: function() {
    var win = window.__TAURI__.window.getCurrentWindow();
    win.setFullscreen(false).catch(function(){});
    win.setAlwaysOnTop(false).catch(function(){});
  },
  ConsoleLog: (msg) => console.log('[EO]', msg),

  NativeViewerOpen: function() {},
  SetAdvancedOptions: function() {},
  LocalFileRecoverFolder: () => '',
  LocalFileRemoveRecoverFolder: function() {},
  InitRecoverFolder: function() {},
  GetRecoverFolder: () => '',
  LocalFileRecents: function() {},
  LocalFileRecover: function() {},
  LocalFileGetOpenChangesCount: function() { return 0; },
  LocalFileGetOpenChanges: function() { return ''; },
  LocalFileSetOpenChangesCount: function() {},
  CanShare: function() { return false; },
  IsViewer: function() { return false; },
};

window.RendererProcessVariable = {
  theme: { current: 'light', system: 'disabled' },
  localthemes: [],
};

window.DesktopAfterOpen = window.DesktopAfterOpen || function(editor) {};

window.UpdateInstallPlugins = window.UpdateInstallPlugins || function() {};

listen('file-opened', (event) => {
  if (event.payload && event.payload.data) {
    _loadEditorBin(event.payload.data, event.payload.path);
  }
});

var _closeDialogOpen = false;
listen('confirm-close', async () => {
  if (_closeDialogOpen) return;
  _closeDialogOpen = true;
  try {
    if (window.AscDesktopEditor._isModified) {
      var discard = await window.__TAURI__.dialog.confirm(
        _t('unsavedDiscardClose'),
        { title: _t('unsavedChanges'), kind: 'warning' }
      );
      if (!discard) return;
    }
    await invoke('force_close');
  } finally {
    _closeDialogOpen = false;
  }
});

listen('open-file', async (event) => {
  if (!event.payload) return;
  var filePath = event.payload;
  window._eoLog('[EO] open-file event received: ' + filePath);

  var ext = filePath.split('.').pop().toLowerCase();
  var docType = 'word';
  if (['xlsx', 'xls', 'ods', 'csv'].indexOf(ext) !== -1) docType = 'cell';
  else if (['pptx', 'ppt', 'odp'].indexOf(ext) !== -1) docType = 'slide';

  try {
    var b64data = await invoke('open_file', { path: filePath });
    var fileName = filePath.replace(/\\/g, '/').split('/').pop();
    window._pendingFileData = { data: b64data, path: filePath, name: fileName };
    window._eoLog('[EO] open-file: converted OK, opening editor as ' + docType);

    if (window._openEditor) {
      window._openEditor(docType);
    } else {
      window._eoLog('[EO] open-file: _openEditor not available, trying LocalFileOpen');
      window.AscDesktopEditor.LocalFileOpen(filePath);
    }
  } catch(e) {
    window._eoLog('[EO] open-file: conversion failed: ' + (e.message || e));
  }
});
