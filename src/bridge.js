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
  // Detects clipboard text that is really a file-manager copy of one image
  // file (Nautilus and friends publish the path as the plain-text flavor, so
  // a text-first probe order would paste the path as text). Multi-line text
  // is rejected: multi-file copies deliberately keep pasting as text.
  // Extension list matches clipboard.rs, without tif/tiff: the WebView has no
  // TIFF decoder and main.rs's content-type map does not cover them either.
  function looksLikeImageFilePath(str) {
    if (!str || typeof str !== 'string') return false;
    if (str.indexOf('\n') !== -1 || str.indexOf('\r') !== -1) return false;
    var s = str.trim();
    if (!s) return false;
    if (s.indexOf('file://') === 0) {
      try { s = decodeURIComponent(s.substring(7)); } catch(e) { return false; }
    }
    var isAbsolute = s.charAt(0) === '/' || /^[A-Za-z]:[\\/]/.test(s);
    if (!isAbsolute) return false;
    var lower = s.toLowerCase();
    var imageExts = ['.png', '.jpg', '.jpeg', '.gif', '.bmp', '.svg', '.webp', '.ico'];
    for (var i = 0; i < imageExts.length; i++) {
      if (lower.endsWith(imageExts[i])) return true;
    }
    return false;
  }
  return {
    readNativeClipboardImage: readNativeClipboardImage,
    extractImageUriFromPaste: extractImageUriFromPaste,
    looksLikeImageFilePath: looksLikeImageFilePath,
    installUrlPipelineOverrides: installUrlPipelineOverrides
  };
})();

// True when an HTML clipboard flavor is nothing but an image reference (the
// shape a browser's "Copy Image" produces: optional meta/html scaffolding
// around a single <img>). Used to tell image pastes apart from rich text.
function _eoHtmlIsJustImage(html) {
  if (!html || !/<img[\s>]/i.test(html)) return false;
  var stripped = html
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<\/?(?:html|head|body|meta|img)[^>]*>/gi, '');
  return stripped.trim() === '';
}

// Replays the SDK's own internal copy buffer, mirroring what sdkjs's
// Button_Paste does when the system clipboard is unreachable. Used when a
// clipboard probe times out because our webview owns the X11 selection and
// arboard cannot read it: in that state the last in-app copy
// IS the clipboard content. Returns true if something was pasted.
function _eoPasteFromLastCopyBinary(ref) {
  if (!ref.editor || !ref.ew || !ref.ew.AscCommon) return false;
  var fmt = ref.ew.AscCommon.c_oAscClipboardDataFormat;
  var cb = ref.ew.AscCommon.g_clipboardBase;
  var last = cb && cb.LastCopyBinary;
  if (!last || !last.length) return false;
  var internalData = null, textData = null;
  for (var i = 0; i < last.length; i++) {
    if (fmt.Internal === last[i].type) internalData = last[i].data;
    else if (fmt.Text === last[i].type) textData = last[i].data;
  }
  if (internalData !== null) ref.editor.asc_PasteData(fmt.Internal, internalData, null, textData);
  else ref.editor.asc_PasteData(last[0].type, last[0].data, null, textData);
  return true;
}

// Issue #23: with a non-Latin keyboard layout active (e.g. Cyrillic), WebKitGTK
// reports the layout character in KeyboardEvent.key/keyCode ("м" instead of "v",
// keyCode from the keysym), so sdkjs/web-apps shortcut handlers stop matching.
// CEF (upstream desktop) normalizes to the US layout automatically; WebKitGTK
// does not. With Ctrl/Alt/Meta held, map the physical key (event.code) back to
// its US-layout equivalent before the event reaches any other handler.
var KeyboardLayoutShim = (function() {
  var CODE_TO_US = {
    'Semicolon': [186, ';'], 'Equal': [187, '='], 'Comma': [188, ','],
    'Minus': [189, '-'], 'Period': [190, '.'], 'Slash': [191, '/'],
    'Backquote': [192, '`'], 'BracketLeft': [219, '['], 'Backslash': [220, '\\'],
    'BracketRight': [221, ']'], 'Quote': [222, "'"]
  };
  for (var i = 0; i < 26; i++) {
    CODE_TO_US['Key' + String.fromCharCode(65 + i)] = [65 + i, String.fromCharCode(97 + i)];
  }
  for (var d = 0; d <= 9; d++) {
    CODE_TO_US['Digit' + d] = [48 + d, String(d)];
  }

  function _normalize(e) {
    if (!(e.ctrlKey || e.metaKey || e.altKey)) return;
    // AltGr combos produce characters, not shortcuts. sdkjs (getAltGr) treats
    // Ctrl+Alt as AltGr, so both forms must pass through untouched.
    if ((e.ctrlKey || e.metaKey) && e.altKey) return;
    if (e.getModifierState && e.getModifierState('AltGraph')) return;
    var mapped = CODE_TO_US[e.code];
    if (!mapped) return;
    // Only rewrite when the layout produced a non-ASCII character; on a Latin
    // layout the event already matches and must not be remapped.
    if (typeof e.key !== 'string' || e.key.length !== 1 || e.key.charCodeAt(0) < 128) return;
    var keyCode = mapped[0];
    var key = mapped[1];
    if (e.shiftKey && keyCode >= 65 && keyCode <= 90) key = key.toUpperCase();
    try {
      Object.defineProperty(e, 'key', { configurable: true, get: function() { return key; } });
      Object.defineProperty(e, 'keyCode', { configurable: true, get: function() { return keyCode; } });
      Object.defineProperty(e, 'which', { configurable: true, get: function() { return keyCode; } });
      e.__eoLayoutNormalized = true;
    } catch (err) {
      window._eoLog('[EO] KeyShim: defineProperty failed: ' + (err.message || err));
    }
  }

  function install(doc) {
    if (!doc || doc.__eoKeyLayoutShimInstalled) return;
    doc.addEventListener('keydown', _normalize, true);
    doc.__eoKeyLayoutShimInstalled = true;
  }

  return { install: install };
})();

function _eoLimitLogText(value, limit) {
  var text = String(value === undefined || value === null ? '' : value);
  return text.length > limit ? text.substring(0, limit) + '...[truncated]' : text;
}

function _eoSafeSource(source) {
  var value = String(source || '').split('#')[0].split('?')[0].replace(/\\/g, '/');
  for (var i = 0; i < 2; i++) {
    var marker = i === 0 ? '/web-apps/' : '/sdkjs/';
    var markerIndex = value.indexOf(marker);
    if (markerIndex !== -1) return value.substring(markerIndex + 1);
  }
  var parts = value.split('/');
  return parts.slice(Math.max(0, parts.length - 2)).join('/');
}

// Keep diagnostics useful without serializing arbitrary SDK objects, which may
// contain document data. Only known error fields and object key names are logged.
function _eoFormatLogValue(value) {
  try {
    if (value === undefined) return 'undefined';
    if (value === null) return 'null';
    if (typeof value !== 'object') return _eoLimitLogText(value, 6000);

    if (Array.isArray(value)) return '[Array length=' + value.length + ']';

    var fields = [];
    var allowed = ['name', 'message', 'errorCode', 'errorDescription', 'level', 'stack'];
    for (var i = 0; i < allowed.length; i++) {
      var key = allowed[i];
      if (value[key] === undefined || value[key] === null || value[key] === '') continue;
      fields.push(key + '=' + _eoLimitLogText(value[key], key === 'stack' ? 6000 : 1000));
    }
    if (fields.length) return fields.join(' ');

    var keys = Object.keys(value).slice(0, 20);
    return '[Object keys=' + keys.join(',') + (Object.keys(value).length > keys.length ? ',...' : '') + ']';
  } catch(e) {
    return '[unprintable diagnostic value]';
  }
}

window._eoSafeSource = _eoSafeSource;
window._eoFormatLogValue = _eoFormatLogValue;
window._eoLogBuffer = [];
window._eoLog = function() {
  var parts = [];
  for (var i = 0; i < arguments.length; i++) {
    parts.push(_eoFormatLogValue(arguments[i]));
  }
  var msg = '[' + new Date().toISOString() + '] ' + parts.join(' ');
  msg = _eoLimitLogText(msg, 12000);
  console.log(msg);
  window._eoLogBuffer.push(msg);
  if (window._eoLogBuffer.length > 500) window._eoLogBuffer.shift();
  try {
    invoke('js_log', { msg: msg });
  } catch(e) {
    try { window.top.__TAURI__.core.invoke('js_log', { msg: msg }); } catch(e2) {}
  }
};

var _isWindows = navigator.platform && navigator.platform.indexOf('Win') !== -1;
var _isMac = navigator.platform && navigator.platform.indexOf('Mac') === 0;
var _isLinux = navigator.platform && navigator.platform.indexOf('Linux') !== -1;
var ASC_PROTO_BASE = _isWindows ? 'http://ascdesktop.localhost/' : 'ascdesktop://';
window._eoAscProtoBase = ASC_PROTO_BASE;

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
    export: 'Export',
    openFile: 'Open file', newDocument: 'New document', newSpreadsheet: 'New spreadsheet',
    newPresentation: 'New presentation', unsavedChanges: 'Unsaved changes',
    unsavedDiscardOpen: 'The current document has unsaved changes. Do you want to discard them and open another file?',
    unsavedDiscardClose: 'The current document has unsaved changes. Do you want to discard them and close?',
    saveError: 'Save error', saveErrorMsg: 'Could not save the file.\n\nTarget format not compatible with this document type.',
    documents: 'Documents', all: 'All', plainText: 'Plain text', user: 'User', language: 'Language',
    recoveredDocuments: 'Recovered documents', recover: 'Recover', discard: 'Discard', discardAll: 'Discard all',
    recentFiles: 'Recent files', rememberRecent: 'Remember recent files', clearRecent: 'Clear',
    noRecentFiles: 'No recent files yet', openFailed: 'Could not open file',
    openFailedMsg: 'The file could not be opened. It may have been moved, renamed or deleted.',
    openErrorMsg: 'The file could not be opened. It is not a valid document or it is damaged.',
    openingFile: 'Opening {name}…', creatingDocument: 'Creating document…',
    removeNoteSeparator: 'Note lines',
    noteSeparator: 'Note lines',
    noteSeparatorConfirm: 'Remove the note separator lines from this document? The lines above footnotes and endnotes are removed from the saved file.',
    noteSeparatorConfirmRestore: 'The separator lines have been removed from this document. Restore them?',
    noteSeparatorSaveFirst: 'Save the document before changing the note separator lines. The change is made in the file on disk.',
    noteSeparatorNoFile: 'Save the document as a .docx file before changing the note separator lines.',
    noteSeparatorNotDocx: 'The note separator lines can only be changed in documents in .docx format.',
    noteSeparatorNoNotes: 'The document does not have footnotes or endnotes yet. Insert a note first: until then the separator is not kept in the file.',
    noteSeparatorAlready: 'The note separator has already been removed from this document.',
    noteSeparatorAlreadyPresent: 'This document already has its note separator lines.',
    noteSeparatorFailed: 'The note separator lines could not be changed.'
  },
  es: {
    document: 'Documento', spreadsheet: 'Hoja de cálculo', presentation: 'Presentación',
    export: 'Exportar',
    openFile: 'Abrir archivo', newDocument: 'Nuevo documento', newSpreadsheet: 'Nueva hoja de cálculo',
    newPresentation: 'Nueva presentación', unsavedChanges: 'Cambios sin guardar',
    unsavedDiscardOpen: 'El documento actual tiene cambios sin guardar. ¿Desea descartarlos y abrir otro archivo?',
    unsavedDiscardClose: 'El documento actual tiene cambios sin guardar. ¿Desea descartarlos y cerrar?',
    saveError: 'Error al guardar', saveErrorMsg: 'No se pudo guardar el archivo.\n\nFormato de destino no compatible con este tipo de documento.',
    documents: 'Documentos', all: 'Todos', plainText: 'Texto plano', user: 'Usuario', language: 'Idioma',
    recoveredDocuments: 'Documentos recuperados', recover: 'Recuperar', discard: 'Descartar', discardAll: 'Descartar todo',
    recentFiles: 'Archivos recientes', rememberRecent: 'Recordar archivos recientes', clearRecent: 'Limpiar',
    noRecentFiles: 'Todavía no hay archivos recientes', openFailed: 'No se pudo abrir el archivo',
    openFailedMsg: 'No se pudo abrir el archivo. Puede que se haya movido, renombrado o eliminado.',
    openErrorMsg: 'No se pudo abrir el archivo. No es un documento válido o está dañado.',
    openingFile: 'Abriendo {name}…', creatingDocument: 'Creando documento…',
    removeNoteSeparator: 'Líneas de notas',
    noteSeparator: 'Líneas de notas',
    noteSeparatorConfirm: '¿Quitar las líneas separadoras de notas de este documento? Las líneas que aparecen encima de las notas al pie y de las notas al final se quitan del archivo guardado.',
    noteSeparatorConfirmRestore: 'Las líneas separadoras de notas se quitaron de este documento. ¿Restaurarlas?',
    noteSeparatorSaveFirst: 'Guarde el documento antes de cambiar las líneas separadoras de notas. El cambio se hace en el archivo en disco.',
    noteSeparatorNoFile: 'Guarde el documento en formato .docx antes de cambiar las líneas separadoras de notas.',
    noteSeparatorNotDocx: 'Las líneas separadoras de notas solo se pueden cambiar en documentos con formato .docx.',
    noteSeparatorNoNotes: 'El documento todavía no tiene notas al pie ni notas al final. Inserte una nota primero: hasta entonces el separador no se conserva en el archivo.',
    noteSeparatorAlready: 'El separador de notas ya se había quitado de este documento.',
    noteSeparatorAlreadyPresent: 'Este documento ya tiene sus líneas separadoras de notas.',
    noteSeparatorFailed: 'No se pudieron cambiar las líneas separadoras de notas.'
  },
  fr: {
    document: 'Document', spreadsheet: 'Feuille de calcul', presentation: 'Présentation',
    export: 'Exporter',
    openFile: 'Ouvrir un fichier', newDocument: 'Nouveau document', newSpreadsheet: 'Nouvelle feuille de calcul',
    newPresentation: 'Nouvelle présentation', unsavedChanges: 'Modifications non enregistrées',
    unsavedDiscardOpen: 'Le document actuel contient des modifications non enregistrées. Voulez-vous les abandonner et ouvrir un autre fichier ?',
    unsavedDiscardClose: 'Le document actuel contient des modifications non enregistrées. Voulez-vous les abandonner et fermer ?',
    saveError: 'Erreur de sauvegarde', saveErrorMsg: 'Impossible d\'enregistrer le fichier.\n\nFormat de destination incompatible avec ce type de document.',
    documents: 'Documents', all: 'Tous', plainText: 'Texte brut', user: 'Utilisateur', language: 'Langue',
    recoveredDocuments: 'Documents récupérés', recover: 'Récupérer', discard: 'Abandonner', discardAll: 'Tout abandonner',
    recentFiles: 'Fichiers récents', rememberRecent: 'Mémoriser les fichiers récents', clearRecent: 'Effacer',
    noRecentFiles: 'Aucun fichier récent pour le moment', openFailed: 'Impossible d\'ouvrir le fichier',
    openFailedMsg: 'Impossible d\'ouvrir le fichier. Il a peut-être été déplacé, renommé ou supprimé.',
    openErrorMsg: 'Impossible d\'ouvrir le fichier. Ce n\'est pas un document valide ou il est endommagé.',
    openingFile: 'Ouverture de {name}…', creatingDocument: 'Création du document…',
    removeNoteSeparator: 'Lignes de notes',
    noteSeparator: 'Lignes de notes',
    noteSeparatorConfirm: 'Supprimer les lignes de séparation des notes de ce document ? Les lignes situées au-dessus des notes de bas de page et des notes de fin sont supprimées du fichier enregistré.',
    noteSeparatorConfirmRestore: 'Les lignes de séparation des notes ont été supprimées de ce document. Les rétablir ?',
    noteSeparatorSaveFirst: 'Enregistrez le document avant de modifier les lignes de séparation des notes. La modification est effectuée dans le fichier sur le disque.',
    noteSeparatorNoFile: 'Enregistrez le document au format .docx avant de modifier les lignes de séparation des notes.',
    noteSeparatorNotDocx: 'Les lignes de séparation des notes ne peuvent être modifiées que dans les documents au format .docx.',
    noteSeparatorNoNotes: 'Le document ne contient pas encore de notes de bas de page ni de notes de fin. Insérez d\'abord une note : jusque-là, le séparateur n\'est pas conservé dans le fichier.',
    noteSeparatorAlready: 'Le séparateur de notes a déjà été supprimé de ce document.',
    noteSeparatorAlreadyPresent: 'Ce document possède déjà ses lignes de séparation des notes.',
    noteSeparatorFailed: 'Impossible de modifier les lignes de séparation des notes.'
  },
  de: {
    document: 'Dokument', spreadsheet: 'Tabelle', presentation: 'Präsentation',
    export: 'Exportieren',
    openFile: 'Datei öffnen', newDocument: 'Neues Dokument', newSpreadsheet: 'Neue Tabelle',
    newPresentation: 'Neue Präsentation', unsavedChanges: 'Ungespeicherte Änderungen',
    unsavedDiscardOpen: 'Das aktuelle Dokument enthält ungespeicherte Änderungen. Möchten Sie diese verwerfen und eine andere Datei öffnen?',
    unsavedDiscardClose: 'Das aktuelle Dokument enthält ungespeicherte Änderungen. Möchten Sie diese verwerfen und schließen?',
    saveError: 'Speicherfehler', saveErrorMsg: 'Die Datei konnte nicht gespeichert werden.\n\nZielformat nicht kompatibel mit diesem Dokumenttyp.',
    documents: 'Dokumente', all: 'Alle', plainText: 'Nur Text', user: 'Benutzer', language: 'Sprache',
    recoveredDocuments: 'Wiederhergestellte Dokumente', recover: 'Wiederherstellen', discard: 'Verwerfen', discardAll: 'Alle verwerfen',
    recentFiles: 'Zuletzt verwendete Dateien', rememberRecent: 'Zuletzt verwendete Dateien merken', clearRecent: 'Leeren',
    noRecentFiles: 'Noch keine zuletzt verwendeten Dateien', openFailed: 'Datei konnte nicht geöffnet werden',
    openFailedMsg: 'Die Datei konnte nicht geöffnet werden. Möglicherweise wurde sie verschoben, umbenannt oder gelöscht.',
    openErrorMsg: 'Die Datei konnte nicht geöffnet werden. Sie ist kein gültiges Dokument oder sie ist beschädigt.',
    openingFile: '{name} wird geöffnet…', creatingDocument: 'Dokument wird erstellt…',
    removeNoteSeparator: 'Trennlinien',
    noteSeparator: 'Trennlinien',
    noteSeparatorConfirm: 'Die Trennlinien der Fuß- und Endnoten aus diesem Dokument entfernen? Die Linien über den Fuß- und Endnoten werden aus der gespeicherten Datei entfernt.',
    noteSeparatorConfirmRestore: 'Die Trennlinien der Fuß- und Endnoten wurden aus diesem Dokument entfernt. Sollen sie wiederhergestellt werden?',
    noteSeparatorSaveFirst: 'Speichern Sie das Dokument, bevor Sie die Trennlinien der Fuß- und Endnoten ändern. Die Änderung erfolgt in der Datei auf dem Datenträger.',
    noteSeparatorNoFile: 'Speichern Sie das Dokument im Format .docx, bevor Sie die Trennlinien der Fuß- und Endnoten ändern.',
    noteSeparatorNotDocx: 'Die Trennlinien der Fuß- und Endnoten können nur in Dokumenten im Format .docx geändert werden.',
    noteSeparatorNoNotes: 'Das Dokument enthält noch keine Fuß- oder Endnoten. Fügen Sie zuerst eine Fußnote oder Endnote ein: bis dahin wird die Trennlinie nicht in der Datei gespeichert.',
    noteSeparatorAlready: 'Die Fuß-/Endnotentrennlinie wurde aus diesem Dokument bereits entfernt.',
    noteSeparatorAlreadyPresent: 'Dieses Dokument enthält die Trennlinien der Fuß- und Endnoten bereits.',
    noteSeparatorFailed: 'Die Trennlinien der Fuß- und Endnoten konnten nicht geändert werden.'
  },
  it: {
    document: 'Documento', spreadsheet: 'Foglio di calcolo', presentation: 'Presentazione',
    export: 'Esporta',
    openFile: 'Apri file', newDocument: 'Nuovo documento', newSpreadsheet: 'Nuovo foglio di calcolo',
    newPresentation: 'Nuova presentazione', unsavedChanges: 'Modifiche non salvate',
    unsavedDiscardOpen: 'Il documento attuale ha modifiche non salvate. Vuoi eliminarle e aprire un altro file?',
    unsavedDiscardClose: 'Il documento attuale ha modifiche non salvate. Vuoi eliminarle e chiudere?',
    saveError: 'Errore di salvataggio', saveErrorMsg: 'Impossibile salvare il file.\n\nFormato di destinazione non compatibile con questo tipo di documento.',
    documents: 'Documenti', all: 'Tutti', plainText: 'Testo normale', user: 'Utente', language: 'Lingua',
    recoveredDocuments: 'Documenti recuperati', recover: 'Recupera', discard: 'Scarta', discardAll: 'Scarta tutto',
    recentFiles: 'File recenti', rememberRecent: 'Ricorda i file recenti', clearRecent: 'Cancella',
    noRecentFiles: 'Nessun file recente', openFailed: 'Impossibile aprire il file',
    openFailedMsg: 'Impossibile aprire il file. Potrebbe essere stato spostato, rinominato o eliminato.',
    openErrorMsg: 'Impossibile aprire il file. Non è un documento valido oppure è danneggiato.',
    openingFile: 'Apertura di {name}…', creatingDocument: 'Creazione del documento…',
    removeNoteSeparator: 'Linee delle note',
    noteSeparator: 'Linee delle note',
    noteSeparatorConfirm: 'Rimuovere le linee di separazione delle note da questo documento? Le linee sopra le note a piè di pagina e le note di chiusura vengono rimosse dal file salvato.',
    noteSeparatorConfirmRestore: 'Le linee di separazione delle note sono state rimosse da questo documento. Ripristinarle?',
    noteSeparatorSaveFirst: 'Salvare il documento prima di modificare le linee di separazione delle note. La modifica viene effettuata nel file su disco.',
    noteSeparatorNoFile: 'Salvare il documento in formato .docx prima di modificare le linee di separazione delle note.',
    noteSeparatorNotDocx: 'Le linee di separazione delle note possono essere modificate solo nei documenti in formato .docx.',
    noteSeparatorNoNotes: 'Il documento non ha ancora note a piè di pagina né note di chiusura. Inserire prima una nota: fino ad allora il separatore non viene conservato nel file.',
    noteSeparatorAlready: 'Il separatore delle note è già stato rimosso da questo documento.',
    noteSeparatorAlreadyPresent: 'Questo documento ha già le linee di separazione delle note.',
    noteSeparatorFailed: 'Impossibile modificare le linee di separazione delle note.'
  },
  pt: {
    document: 'Documento', spreadsheet: 'Planilha', presentation: 'Apresentação',
    export: 'Exportar',
    openFile: 'Abrir arquivo', newDocument: 'Novo documento', newSpreadsheet: 'Nova planilha',
    newPresentation: 'Nova apresentação', unsavedChanges: 'Alterações não salvas',
    unsavedDiscardOpen: 'O documento atual tem alterações não salvas. Deseja descartá-las e abrir outro arquivo?',
    unsavedDiscardClose: 'O documento atual tem alterações não salvas. Deseja descartá-las e fechar?',
    saveError: 'Erro ao salvar', saveErrorMsg: 'Não foi possível salvar o arquivo.\n\nFormato de destino não compatível com este tipo de documento.',
    documents: 'Documentos', all: 'Todos', plainText: 'Texto simples', user: 'Usuário', language: 'Idioma',
    recoveredDocuments: 'Documentos recuperados', recover: 'Recuperar', discard: 'Descartar', discardAll: 'Descartar tudo',
    recentFiles: 'Arquivos recentes', rememberRecent: 'Lembrar arquivos recentes', clearRecent: 'Limpar',
    noRecentFiles: 'Ainda não há arquivos recentes', openFailed: 'Não foi possível abrir o arquivo',
    openFailedMsg: 'Não foi possível abrir o arquivo. Ele pode ter sido movido, renomeado ou excluído.',
    openErrorMsg: 'Não foi possível abrir o arquivo. Não é um documento válido ou está danificado.',
    openingFile: 'Abrindo {name}…', creatingDocument: 'Criando documento…',
    removeNoteSeparator: 'Linhas de notas',
    noteSeparator: 'Linhas de notas',
    noteSeparatorConfirm: 'Remover as linhas separadoras de notas deste documento? As linhas acima das notas de rodapé e das notas de fim são removidas do arquivo salvo.',
    noteSeparatorConfirmRestore: 'As linhas separadoras de notas foram removidas deste documento. Restaurá-las?',
    noteSeparatorSaveFirst: 'Salve o documento antes de alterar as linhas separadoras de notas. A alteração é feita no arquivo em disco.',
    noteSeparatorNoFile: 'Salve o documento no formato .docx antes de alterar as linhas separadoras de notas.',
    noteSeparatorNotDocx: 'As linhas separadoras de notas só podem ser alteradas em documentos no formato .docx.',
    noteSeparatorNoNotes: 'O documento ainda não tem notas de rodapé nem notas de fim. Insira uma nota primeiro: até lá o separador não é mantido no arquivo.',
    noteSeparatorAlready: 'O separador de notas já foi removido deste documento.',
    noteSeparatorAlreadyPresent: 'Este documento já tem as linhas separadoras de notas.',
    noteSeparatorFailed: 'Não foi possível alterar as linhas separadoras de notas.'
  },
  ru: {
    document: 'Документ', spreadsheet: 'Таблица', presentation: 'Презентация',
    export: 'Экспорт',
    openFile: 'Открыть файл', newDocument: 'Новый документ', newSpreadsheet: 'Новая таблица',
    newPresentation: 'Новая презентация', unsavedChanges: 'Несохранённые изменения',
    unsavedDiscardOpen: 'Текущий документ содержит несохранённые изменения. Отменить их и открыть другой файл?',
    unsavedDiscardClose: 'Текущий документ содержит несохранённые изменения. Отменить их и закрыть?',
    saveError: 'Ошибка сохранения', saveErrorMsg: 'Не удалось сохранить файл.\n\nФормат назначения несовместим с этим типом документа.',
    documents: 'Документы', all: 'Все', plainText: 'Обычный текст', user: 'Пользователь', language: 'Язык',
    recoveredDocuments: 'Восстановленные документы', recover: 'Восстановить', discard: 'Отклонить', discardAll: 'Отклонить всё',
    recentFiles: 'Недавние файлы', rememberRecent: 'Запоминать недавние файлы', clearRecent: 'Очистить',
    noRecentFiles: 'Недавних файлов пока нет', openFailed: 'Не удалось открыть файл',
    openFailedMsg: 'Не удалось открыть файл. Возможно, он был перемещён, переименован или удалён.',
    openErrorMsg: 'Не удалось открыть файл. Это не действительный документ или он повреждён.',
    openingFile: 'Открытие {name}…', creatingDocument: 'Создание документа…',
    removeNoteSeparator: 'Линии сносок',
    noteSeparator: 'Линии сносок',
    noteSeparatorConfirm: 'Убрать из этого документа линии, отделяющие сноски? Линии над обычными и концевыми сносками будут удалены из сохранённого файла.',
    noteSeparatorConfirmRestore: 'Линии, отделяющие сноски, были убраны из этого документа. Восстановить их?',
    noteSeparatorSaveFirst: 'Сохраните документ, прежде чем изменять линии сносок. Изменение вносится в файл на диске.',
    noteSeparatorNoFile: 'Сохраните документ в формате .docx, прежде чем изменять линии сносок.',
    noteSeparatorNotDocx: 'Линии сносок можно изменить только в документах формата .docx.',
    noteSeparatorNoNotes: 'В документе пока нет ни обычных, ни концевых сносок. Сначала вставьте сноску: до этого разделитель не сохраняется в файле.',
    noteSeparatorAlready: 'Разделитель сносок уже убран из этого документа.',
    noteSeparatorAlreadyPresent: 'В этом документе линии, отделяющие сноски, уже есть.',
    noteSeparatorFailed: 'Не удалось изменить линии сносок.'
  },
  uk: {
    document: 'Документ', spreadsheet: 'Таблиця', presentation: 'Презентація',
    export: 'Експорт',
    openFile: 'Відкрити файл', newDocument: 'Новий документ', newSpreadsheet: 'Нова таблиця',
    newPresentation: 'Нова презентація', unsavedChanges: 'Незбережені зміни',
    unsavedDiscardOpen: 'Поточний документ має незбережені зміни. Бажаєте скасувати їх і відкрити інший файл?',
    unsavedDiscardClose: 'Поточний документ має незбережені зміни. Бажаєте скасувати їх і закрити?',
    saveError: 'Помилка збереження', saveErrorMsg: 'Не вдалося зберегти файл.\n\nФормат призначення несумісний із цим типом документа.',
    documents: 'Документи', all: 'Усі', plainText: 'Звичайний текст', user: 'Користувач', language: 'Мова',
    recoveredDocuments: 'Відновлені документи', recover: 'Відновити', discard: 'Відхилити', discardAll: 'Відхилити все',
    recentFiles: 'Нещодавні файли', rememberRecent: 'Запам\'ятовувати нещодавні файли', clearRecent: 'Очистити',
    noRecentFiles: 'Нещодавніх файлів поки немає', openFailed: 'Не вдалося відкрити файл',
    openFailedMsg: 'Не вдалося відкрити файл. Можливо, його переміщено, перейменовано або видалено.',
    openErrorMsg: 'Не вдалося відкрити файл. Це не дійсний документ або його пошкоджено.',
    openingFile: 'Відкриття {name}…', creatingDocument: 'Створення документа…',
    removeNoteSeparator: 'Лінії виносок',
    noteSeparator: 'Лінії виносок',
    noteSeparatorConfirm: 'Прибрати з цього документа лінії, що відокремлюють виноски? Лінії над звичайними та кінцевими виносками буде вилучено зі збереженого файлу.',
    noteSeparatorConfirmRestore: 'Лінії, що відокремлюють виноски, було прибрано з цього документа. Відновити їх?',
    noteSeparatorSaveFirst: 'Збережіть документ, перш ніж змінювати лінії виносок. Зміну буде внесено до файлу на диску.',
    noteSeparatorNoFile: 'Збережіть документ у форматі .docx, перш ніж змінювати лінії виносок.',
    noteSeparatorNotDocx: 'Лінії виносок можна змінити лише в документах формату .docx.',
    noteSeparatorNoNotes: 'У документі поки немає ні звичайних, ні кінцевих виносок. Спочатку вставте виноску: до цього роздільник не зберігається у файлі.',
    noteSeparatorAlready: 'Роздільник виносок уже прибрано з цього документа.',
    noteSeparatorAlreadyPresent: 'У цьому документі лінії, що відокремлюють виноски, уже є.',
    noteSeparatorFailed: 'Не вдалося змінити лінії виносок.'
  },
  zh: {
    document: '文档', spreadsheet: '电子表格', presentation: '演示文稿',
    export: '导出',
    openFile: '打开文件', newDocument: '新建文档', newSpreadsheet: '新建电子表格',
    newPresentation: '新建演示文稿', unsavedChanges: '未保存的更改',
    unsavedDiscardOpen: '当前文档有未保存的更改。是否放弃更改并打开另一个文件？',
    unsavedDiscardClose: '当前文档有未保存的更改。是否放弃更改并关闭？',
    saveError: '保存错误', saveErrorMsg: '无法保存文件。\n\n目标格式与此文档类型不兼容。',
    documents: '文档', all: '所有文件', plainText: '纯文本', user: '用户', language: '语言',
    recoveredDocuments: '已恢复的文档', recover: '恢复', discard: '丢弃', discardAll: '全部丢弃',
    recentFiles: '最近的文件', rememberRecent: '记住最近的文件', clearRecent: '清除',
    noRecentFiles: '暂无最近的文件', openFailed: '无法打开文件',
    openFailedMsg: '无法打开文件。它可能已被移动、重命名或删除。',
    openErrorMsg: '无法打开文件。它不是有效的文档，或者已损坏。',
    openingFile: '正在打开 {name}…', creatingDocument: '正在创建文档…',
    removeNoteSeparator: '脚注/尾注线',
    noteSeparator: '脚注/尾注线',
    noteSeparatorConfirm: '要移除此文档中的脚注/尾注分隔线吗？脚注和尾注上方的横线将从保存的文件中移除。',
    noteSeparatorConfirmRestore: '此文档的脚注/尾注分隔线已被移除。要恢复它们吗？',
    noteSeparatorSaveFirst: '请先保存文档，然后再更改脚注/尾注分隔线。更改是在磁盘上的文件中进行的。',
    noteSeparatorNoFile: '请先将文档保存为 .docx 格式，然后再更改脚注/尾注分隔线。',
    noteSeparatorNotDocx: '只能在 .docx 格式的文档中更改脚注/尾注分隔线。',
    noteSeparatorNoNotes: '该文档尚无脚注或尾注。请先插入一条脚注或尾注：在此之前，分隔线不会保存到文件中。',
    noteSeparatorAlready: '该文档的脚注/尾注分隔线已经移除。',
    noteSeparatorAlreadyPresent: '此文档已有脚注/尾注分隔线。',
    noteSeparatorFailed: '无法更改脚注/尾注分隔线。'
  },
  ja: {
    document: 'ドキュメント', spreadsheet: 'スプレッドシート', presentation: 'プレゼンテーション',
    export: 'エクスポート',
    openFile: 'ファイルを開く', newDocument: '新規ドキュメント', newSpreadsheet: '新規スプレッドシート',
    newPresentation: '新規プレゼンテーション', unsavedChanges: '未保存の変更',
    unsavedDiscardOpen: '現在のドキュメントには未保存の変更があります。変更を破棄して別のファイルを開きますか？',
    unsavedDiscardClose: '現在のドキュメントには未保存の変更があります。変更を破棄して閉じますか？',
    saveError: '保存エラー', saveErrorMsg: 'ファイルを保存できませんでした。\n\n対象の形式はこのドキュメントタイプと互換性がありません。',
    documents: 'ドキュメント', all: 'すべて', plainText: 'プレーンテキスト', user: 'ユーザー', language: '言語',
    recoveredDocuments: '復元されたドキュメント', recover: '復元', discard: '破棄', discardAll: 'すべて破棄',
    recentFiles: '最近使用したファイル', rememberRecent: '最近使用したファイルを記憶する', clearRecent: 'クリア',
    noRecentFiles: '最近使用したファイルはありません', openFailed: 'ファイルを開けませんでした',
    openFailedMsg: 'ファイルを開けませんでした。移動、名前の変更、または削除された可能性があります。',
    openErrorMsg: 'ファイルを開けませんでした。有効なドキュメントではないか、破損しています。',
    openingFile: '{name} を開いています…', creatingDocument: 'ドキュメントを作成しています…',
    removeNoteSeparator: '注の区切り線',
    noteSeparator: '注の区切り線',
    noteSeparatorConfirm: 'この文書から注の区切り線を削除しますか？脚注と文末脚注の上にある線が、保存されるファイルから削除されます。',
    noteSeparatorConfirmRestore: 'この文書からは注の区切り線が削除されています。元に戻しますか？',
    noteSeparatorSaveFirst: '注の区切り線を変更する前にドキュメントを保存してください。変更はディスク上のファイルに対して行われます。',
    noteSeparatorNoFile: '注の区切り線を変更する前に、ドキュメントを .docx 形式で保存してください。',
    noteSeparatorNotDocx: '注の区切り線を変更できるのは .docx 形式のドキュメントだけです。',
    noteSeparatorNoNotes: 'このドキュメントにはまだ脚注も文末脚注もありません。先に注を挿入してください。それまで区切り線はファイルに保存されません。',
    noteSeparatorAlready: 'このドキュメントの注の区切り線はすでに削除されています。',
    noteSeparatorAlreadyPresent: 'この文書にはすでに注の区切り線があります。',
    noteSeparatorFailed: '注の区切り線を変更できませんでした。'
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

// One dialog for every failed open. The Rust side already refuses a file the
// converter cannot read, but the reason only reached js-debug.log: the user saw
// the start screen come back and nothing else (Issue #38). The raw error stays
// in the log, the user gets a translated, generic message.
function _eoShowOpenError() {
  return window.__TAURI__.dialog.message(
    _t('openErrorMsg'),
    { title: _t('openFailed'), kind: 'error' }
  );
}
window._eoShowOpenError = _eoShowOpenError;

// Converting a file happens before the editor exists and reports no progress,
// so the shell shows an indeterminate indicator until the editor takes over
// with its own loading mask. Calling it again while visible only updates the text.
function _eoShowOpening(text) {
  var overlay = document.getElementById('opening');
  if (!overlay) return;
  document.getElementById('opening-text').textContent = text || '';
  if (overlay.classList.contains('active')) return;
  overlay.classList.add('active');
  // The overlay already catches clicks; inert also stops a focused button
  // from starting a second open on Enter or Space.
  var startScreen = document.getElementById('start-screen');
  if (startScreen) startScreen.inert = true;
  if (document.activeElement && document.activeElement.blur) document.activeElement.blur();
}

function _eoHideOpening() {
  var overlay = document.getElementById('opening');
  if (!overlay) return;
  overlay.classList.remove('active');
  var startScreen = document.getElementById('start-screen');
  if (startScreen) startScreen.inert = false;
}

// A function replacement, so a file name containing "$&" is inserted as is.
function _eoOpeningFileText(path) {
  var name = String(path || '').replace(/\\/g, '/').split('/').pop();
  return _t('openingFile').replace('{name}', function() { return name; });
}

window._eoShowOpening = _eoShowOpening;
window._eoHideOpening = _eoHideOpening;
window._eoOpeningFileText = _eoOpeningFileText;

// Which editor a path belongs to. Shared with index.html so the start screen,
// the reopen-after-reload path and the recent files list all agree.
function _eoDocTypeForPath(path) {
  var ext = String(path || '').split('.').pop().toLowerCase();
  if (['xlsx', 'xls', 'ods', 'csv'].indexOf(ext) !== -1) return 'cell';
  if (['pptx', 'ppt', 'odp'].indexOf(ext) !== -1) return 'slide';
  return 'word';
}
window._eoDocTypeForPath = _eoDocTypeForPath;

window.addEventListener('error', function(e) {
  window._eoLog('[JS-ERROR]', {
    name: e.error && e.error.name,
    message: e.message || (e.error && e.error.message),
    stack: e.error && e.error.stack
  }, 'source=' + _eoSafeSource(e.filename) + ':' + (e.lineno || 0) + ':' + (e.colno || 0));
});
window.addEventListener('unhandledrejection', function(e) {
  window._eoLog('[JS-REJECT]', e.reason || 'unknown');
});
var _origConsoleError = console.error;
console.error = function() {
  var parts = [];
  for (var i = 0; i < arguments.length; i++) {
    parts.push(_eoFormatLogValue(arguments[i]));
  }
  window._eoLog('[CONSOLE-ERROR] ' + parts.join(' '));
  _origConsoleError.apply(console, arguments);
};

function _findEditorWindow(win) {
  try { if (win.AscCommon) return win; } catch(e) {}
  for (var i = 0; i < win.frames.length; i++) {
    var found = _findEditorWindow(win.frames[i]);
    if (found) return found;
  }
  return null;
}

function _eoBaseName(path) {
  if (!path) return null;
  return path.replace(/\\/g, '/').split('/').pop();
}

// The same table as detect_format in file_ops.rs. Recorded in doc.info so a
// recovered document is written back in the format its extension asks for.
function _eoFormatForPath(path) {
  var map = {
    docx: 65, doc: 66, odt: 67, rtf: 68, txt: 69,
    xlsx: 257, xls: 258, ods: 259, csv: 260,
    pptx: 129, ppt: 130, odp: 131, pdf: 513
  };
  if (!path || path.indexOf('.') === -1) return null;
  return map[path.split('.').pop().toLowerCase()] || null;
}

function _eoBytesToB64(data) {
  if (typeof data === 'string') return btoa(data);
  var bytes = new Uint8Array(data);
  var binary = '';
  for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

// Opens the recovery session for the document that just went into the editor.
// The bytes handed over are the ones openDocument received, because that is
// what a recovery replays the changes on top of: everything the editor did
// afterwards is already inside those changes.
function _recoveryBegin(b64data, name, path) {
  invoke('recovery_begin', {
    name: name || _eoBaseName(path) || 'Untitled',
    path: path || null,
    format: path ? _eoFormatForPath(path) : null,
    docType: window.AscDesktopEditor._currentDocType || 'word',
    data: b64data
  }).catch(function(e) {
    window._eoLog('[RECOVER] begin failed: ' + ((e && e.message) || e));
  });
}

// How many changes the current document was recovered with; 0 when it was
// opened normally. It lives in the host frame's module scope on purpose: the
// editor iframe gets its own AscDesktopEditor, but that object is a shallow
// copy of this one (editor-patches.js), so both hold the very same functions
// and both close over this variable. Setting it answers the editor's question
// whichever of the two objects it asks.
var _eoOpenChangesCount = 0;

// Recovery replays a document's unsaved changes through the editor's OWN open
// pipeline, the way the desktop shell does: the changes are ENQUEUED into
// CollaborativeEditing.m_aChanges before openDocument runs, and openDocument
// applies them inside onDocumentContentReady (silent mode, recalculate,
// repaint, release the load lock, close the block action). Applying them by
// hand after the open instead lands them on the model but never redraws the
// canvas and leaves the load lock open.
//
// Enqueue is safe before the document is loaded: it only pushes onto an array
// (Add_Changes). The count has to be answerable before the open too, so the
// editor's first modified check sees a non-zero value.
function _recoveryEnqueue(ref, recovery) {
  var changes = (recovery && recovery.changes) || [];
  try {
    _eoOpenChangesCount = changes.length;
    // The iframe holds its own copy of AscDesktopEditor; point its count
    // getter at the live one before the editor asks.
    try {
      if (ref.ew && ref.ew.AscDesktopEditor) {
        ref.ew.AscDesktopEditor.LocalFileGetOpenChangesCount =
          window.AscDesktopEditor.LocalFileGetOpenChangesCount;
      }
    } catch(e) {}
    // True is the editor's default; set it here so a stale instance still lets
    // openDocument apply the queued changes in its pipeline.
    if (ref.editor) ref.editor.isApplyChangesOnOpenEnabled = true;
    if (changes.length && ref.ew.DesktopOfflineAppDocumentApplyChanges) {
      // Enqueues only (Add_Changes -> m_aChanges.push); openDocument applies.
      ref.ew.DesktopOfflineAppDocumentApplyChanges(changes);
    }
  } catch(e) {
    window._eoLog('[RECOVER] enqueue failed: ' + ((e && e.message) || e));
  }
  window._eoLog('[RECOVER] replay n=' + changes.length);
}

// Once the open pipeline has applied the changes, the document holds work that
// is in no file, so it must read as modified: GetOpenChangesCount answers the
// editor's own check, set_document_modified sets the Rust flag that gates the
// close confirmation.
function _recoveryMarkModified(name) {
  try {
    window.AscDesktopEditor._isModified = true;
    invoke('set_document_modified', { modified: true }).catch(function(){});
    if (name) invoke('set_window_title', { name: name }).catch(function(){});
  } catch(e) {}
}

function _forceReload() {
  window.onbeforeunload = null;
  var iframes = document.querySelectorAll('iframe');
  for (var i = 0; i < iframes.length; i++) iframes[i].remove();
  // The document is being dropped on purpose here: it was unmodified, or the
  // user answered the discard dialog. Ending the session keeps a deliberate
  // close from coming back as a crash offer on the next start.
  invoke('recovery_end', { discard: true })
    .catch(function(){})
    .finally(function() {
      invoke('set_document_modified', { modified: false })
        .catch(function(){})
        .finally(function() { window.location.reload(); });
    });
}

// Note lines (#42), a toggle. The horizontal rules above the notes come from
// special notes in the file itself and no editor setting reaches them, so the
// Rust side edits word/footnotes.xml and word/endnotes.xml in the .docx on disk
// (both the separator and the continuationSeparator of each) and the document
// is reopened from there.
//
// Because the surgery happens on disk, anything still only in the editor would
// be written over it by the next save. The order is therefore: refuse while the
// document is modified, operate, reopen.
//
// Refusing rather than saving first is deliberate. LocalFileSave is awaitable
// here, but it also reports the end of a save to sdkjs with
// DesktopOfflineAppDocumentEndSave, which sdkjs expects to answer a save IT
// started; calling it from a save the editor never began puts the editor's own
// state at odds with ours, and it returns the same undefined whether the write
// succeeded or failed, so the surgery could not tell one from the other. A
// message the user answers with Ctrl+S costs one click and no such guessing.
//
// One entry, both directions. Taking the lines away and never giving them back
// would be a trap, so the entry is a toggle: it asks the file which state it is
// in first (an inspection that writes nothing) and then offers the move that
// state allows, removing the lines or putting them back. The menu label stays
// the same noun either way, which is why the confirmation has to say which of
// the two is about to happen.
//
// The full order is: refuse while modified, inspect, confirm, operate, reopen.
// The confirmation carries the documentation of the action, for the reason given
// where it is raised.
async function _eoRemoveNoteSeparator() {
  var dialog = window.__TAURI__.dialog;
  function _tell(key) {
    return dialog.message(_t(key), { title: _t('noteSeparator'), kind: 'info' });
  }

  if (window.AscDesktopEditor._isModified) {
    await _tell('noteSeparatorSaveFirst');
    return;
  }

  async function _act(action) {
    try {
      var code = await invoke('remove_note_separator', { action: action });
      window._eoLog('[EO] note separator ' + action + ': ' + code);
      return code;
    } catch(e) {
      window._eoLog('[EO] note separator ' + action + ' failed: ' + (e.message || e));
      return null;
    }
  }

  // Messages for everything that is not a state the toggle can act on, shared
  // by the inspection and by the two actions: a document can stop being
  // operable between the question and the answer.
  var messages = {
    no_notes: 'noteSeparatorNoNotes',
    not_docx: 'noteSeparatorNotDocx',
    no_file: 'noteSeparatorNoFile',
    already_removed: 'noteSeparatorAlready',
    already_present: 'noteSeparatorAlreadyPresent'
  };

  var state = await _act('inspect');
  if (state !== 'present' && state !== 'removed') {
    await _tell(messages[state] || 'noteSeparatorFailed');
    return;
  }

  // The File menu entry is a short noun ("Note lines") so that it lines up
  // with its neighbours instead of running over them, and a noun does not say
  // what clicking it does. This dialog does: it spells out which way the lines
  // are about to go, and (for the removal) that the change lands in the file on
  // disk rather than in the editor, which is worth a confirmation on its own.
  var removing = state === 'present';
  var confirmed = await dialog.confirm(
    _t(removing ? 'noteSeparatorConfirm' : 'noteSeparatorConfirmRestore'),
    { title: _t('noteSeparator'), kind: 'warning' });
  if (!confirmed) return;

  var code = await _act(removing ? 'remove' : 'restore');

  if (code === 'removed' || code === 'restored') {
    // Same reopen the document switch uses: the editor holds the document it
    // parsed at open time, and only a fresh open shows the edited file.
    var path = await invoke('get_current_path').catch(function() { return null; });
    if (path) localStorage.setItem('eo-pending-open-path', path);
    _forceReload();
    return;
  }

  await _tell(messages[code] || 'noteSeparatorFailed');
}
window._eoRemoveNoteSeparator = _eoRemoveNoteSeparator;

// Which filters the Save As dialog offers. Pulled out of LocalFileSave so the
// Linux branch below can be reasoned about (and exercised) on its own.
//
// On Linux the GTK dialog never reports back which filter the user picked:
// rfd reads only gtk_file_chooser_get_filename and drops the filter, and
// tauri-plugin-dialog's save returns the bare path (verified against rfd 0.16
// and tauri-plugin-dialog 2.7.1). A dropdown listing every format therefore
// offers a choice we cannot honour, and picking PDF while typing a name with
// no extension wrote a .docx instead (#34). So on Linux the generic Save As
// offers exactly one filter, the format the document already is, and the
// dialog stops promising anything it cannot deliver. Choosing a different
// format there goes through the editor's own Save As submenu, which passes
// fileType and takes the requestedExt branch.
//
// Windows and macOS keep the full list: their dialogs append the extension of
// the selected type themselves, so the choice does survive.
function _eoSaveAsFilters(docType, requestedExt, currentPath, isLinux) {
  var byDocType = {
    cell: [
      { name: 'Excel', extensions: ['xlsx'] },
      { name: 'OpenDocument Spreadsheet', extensions: ['ods'] },
      { name: 'CSV', extensions: ['csv'] },
      { name: 'PDF', extensions: ['pdf'] },
    ],
    slide: [
      { name: 'PowerPoint', extensions: ['pptx'] },
      { name: 'OpenDocument Presentation', extensions: ['odp'] },
      { name: 'PDF', extensions: ['pdf'] },
    ],
    word: [
      { name: 'Word', extensions: ['docx'] },
      { name: 'OpenDocument Text', extensions: ['odt'] },
      { name: 'Rich Text', extensions: ['rtf'] },
      { name: _t('plainText'), extensions: ['txt'] },
      { name: 'PDF', extensions: ['pdf'] },
    ]
  };
  var all = byDocType[docType] || byDocType.word;

  // The editor already said which format it wants: one filter, and the
  // auto-append below has nothing to guess.
  if (requestedExt) {
    return [{
      name: requestedExt === 'pdf' ? 'PDF' : requestedExt.toUpperCase(),
      extensions: [requestedExt]
    }];
  }

  if (!isLinux) return all;

  // The format the document already is, falling back to the doc type's default
  // when the file was never saved or is of a legacy type Save As does not
  // offer to write back (a .doc or an .xls, say).
  var currentExt = (currentPath || '').replace(/\\/g, '/').split('/').pop().split('.').pop().toLowerCase();
  var match = null;
  for (var i = 0; i < all.length; i++) {
    if (all[i].extensions[0] === currentExt) { match = all[i]; break; }
  }
  return [match || all[0]];
}

function _getEditor() {
  var ew = window.AscDesktopEditor._editorWindow || _findEditorWindow(window);
  if (ew) window.AscDesktopEditor._editorWindow = ew;
  var editor = ew && ew.Asc && ew.Asc.editor;
  return { ew: ew, editor: editor };
}

// `options` carries what the file name alone cannot say: the display name of a
// document with no path yet, and the recovery payload for a document being
// replayed.
function _loadEditorBin(b64data, fileName, options) {
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
      if (isLinux) {
        // Must register before the Ctrl+V interceptor below: same document and
        // phase, so listener order is registration order, and the interceptor's
        // e.key === 'v' check needs the already-normalized key.
        KeyboardLayoutShim.install(editorDoc);

        editorDoc.addEventListener('keydown', function(e) {
          // WebKitGTK matches its native copy/cut/paste editing commands by
          // keyval, so with a non-Latin layout they never run (the clipboard
          // side of Issue #23). For normalized events, drive them from here:
          // execCommand for copy/cut (fires the same copy/cut DOM event the
          // native path uses), the bridge's Paste() for paste (execCommand
          // 'paste' is refused).
          if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.__eoLayoutNormalized) {
            if (e.key === 'c' || e.key === 'x') {
              var execOk = false;
              try {
                execOk = editorDoc.execCommand(e.key === 'c' ? 'copy' : 'cut');
              } catch(err) {
                window._eoLog('[EO] KeyShim ' + e.key + ': execCommand=error ' + (err.message || err));
              }
              // WebKitGTK refuses the cut command on sdkjs's collapsed DOM
              // selection; mirror sdkjs's own Button_Cut fallback: copy
              // natively, then delete the selection.
              if (e.key === 'x' && !execOk) {
                try { editorDoc.execCommand('copy'); } catch(err) {}
                var refCut = _getEditor();
                if (refCut.editor && refCut.editor.asc_SelectionCut) refCut.editor.asc_SelectionCut();
              }
              return;
            }
            if (e.key === 'v') {
              // Issue #24: mark the native read as in flight so the paste
              // listener below can block the SDK's duplicate insertion. With
              // non-Latin layouts WebKitGTK normally fires no paste event at
              // all; the timeout covers that case.
              window._eoLinuxPasteInFlight = true;
              setTimeout(function() { window._eoLinuxPasteInFlight = false; }, 400);
              // read_clipboard_text hangs ~30s when our own webview owns the
              // clipboard (it cannot answer the selection request while
              // waiting). In that case the last in-app copy IS
              // the clipboard content, so fall back to sdkjs's internal copy
              // buffer like Button_Paste does. A fast empty read means an
              // external owner without text: probe for an image as usual.
              var TIMED_OUT = { timedOut: true };
              var timeoutP = new Promise(function(resolve) {
                setTimeout(function() { resolve(TIMED_OUT); }, 1200);
              });
              Promise.race([invoke('read_clipboard_text'), timeoutP]).then(function(text) {
                var refV = _getEditor();
                if (!refV.editor || !refV.ew || !refV.ew.AscCommon) return;
                var fmt = refV.ew.AscCommon.c_oAscClipboardDataFormat;
                if (text && text !== TIMED_OUT) {
                  refV.editor.asc_PasteData(fmt.Text, text);
                  window._eoLog('[EO] KeyShim v: result=text');
                  return;
                }
                if (text === TIMED_OUT) {
                  window._eoLog('[EO] KeyShim v: result=' +
                    (_eoPasteFromLastCopyBinary(refV) ? 'internal' : 'timeout-no-internal'));
                  return;
                }
                ClipboardHelper.readNativeClipboardImage().then(function(imageFile) {
                  if (imageFile) {
                    refV.editor.AddImageUrl([imageFile]);
                    window._eoLog('[EO] KeyShim v: result=image');
                  } else {
                    window._eoLog('[EO] KeyShim v: result=empty');
                  }
                });
              }).catch(function(err) {
                window._eoLog('[EO] KeyShim v: error ' + (err && err.message || err));
              });
              return;
            }
          }
          if ((e.ctrlKey || e.metaKey) && e.key === 'v' && !e.shiftKey) {
            // Issue #24: mark the native read as in flight so the paste
            // listener below can block the SDK's duplicate insertion of the
            // same keystroke (it fires ~50ms later on the same document).
            window._eoLinuxPasteInFlight = true;
            setTimeout(function() { window._eoLinuxPasteInFlight = false; }, 400);
            ClipboardHelper.readNativeClipboardImage().then(function(imageFile) {
              if (imageFile) {
                var ref = _getEditor();
                if (ref.editor) {
                  ref.editor.AddImageUrl([imageFile]);
                  window._eoLog('[EO] Ctrl+V: clipboard image inserted ' + imageFile);
                }
              }
            });
          }
        }, true);
      }

      editorDoc.addEventListener('paste', function(e) {
        var cd = e.clipboardData;
        // Issue #24: on Linux, one Ctrl+V fires both the keydown interceptor
        // above (native clipboard read -> AddImageUrl) and the SDK's own
        // paste handler. A browser image copy carries no plain text, only a
        // text/html "<img src=...>" that the SDK would download and insert
        // as a second copy (through its hidden asc_pasteFrame iframe). While
        // the interceptor's read is in flight and the payload is image-shaped,
        // block the SDK handler: this capture listener runs before sdkjs's
        // bubble-phase ones, so stopPropagation is enough. Anything carrying
        // plain text (web text, in-app copies) must pass through untouched.
        if (isLinux && window._eoLinuxPasteInFlight) {
          window._eoLinuxPasteInFlight = false;
          var suppressPlain = '';
          var suppressHtml = '';
          try {
            suppressPlain = cd ? cd.getData('text/plain') : '';
            suppressHtml = cd ? cd.getData('text/html') : '';
          } catch(suppressErr) {}
          if (!suppressPlain && (!suppressHtml || _eoHtmlIsJustImage(suppressHtml))) {
            window._eoLog('[EO] paste: SDK paste suppressed (image payload, native read in flight)');
            e.preventDefault();
            e.stopPropagation();
            return;
          }
        }
        if (!cd) return;
        var imagePath = ClipboardHelper.extractImageUriFromPaste(cd);
        if (imagePath) {
          e.preventDefault();
          e.stopPropagation();
          var ref = _getEditor();
          if (ref.editor) {
            ref.editor.AddImageUrl([imagePath]);
          }
          return;
        }
        // On macOS, WKWebView exposes a Finder-copied file as a File object (not text/uri-list).
        // preventDefault stops the SDK from also inserting the icon as a duplicate data: URI.
        if (_isMac && cd.files && cd.files.length > 0) {
          var hasImageFile = false;
          for (var i = 0; i < cd.files.length; i++) {
            if (cd.files[i].type.indexOf('image/') === 0) {
              hasImageFile = true;
              break;
            }
          }
          if (hasImageFile) {
            e.preventDefault();
            e.stopPropagation();
            ClipboardHelper.readNativeClipboardImage().then(function(imageFile) {
              if (imageFile) {
                var ref = _getEditor();
                if (ref.editor) {
                  ref.editor.AddImageUrl([imageFile]);
                }
              }
            });
          }
        }
      }, true);
    }

    var opts = options || {};
    // Recovery queues its changes BEFORE the open so the editor applies them in
    // its pipeline. recovery_load already adopted the folder, so no session is
    // opened here.
    if (opts.recovery) {
      _recoveryEnqueue(ref, opts.recovery);
    }

    var file = new ref.ew.AscCommon.OpenFileResult();
    file.data = bytes;
    file.bSerFormat = true;
    ref.editor.openDocument(file);
    ref.ew.AscCommon.History.UserSaveMode = true;
    _ensureCoreProps(ref);

    if (opts.recovery) {
      _recoveryMarkModified(opts.name || _eoBaseName(fileName));
    } else if (!opts.skipRecovery) {
      _recoveryBegin(b64data, opts.name || _eoBaseName(fileName), opts.path || fileName || null);
    }

    if (fileName) {
      var name = fileName.replace(/\\/g, '/').split('/').pop();
      invoke('set_window_title', { name: name }).catch(function(){});
    }
  } catch(e) {
    window._eoLog('[EO] Error loading document:', e.message);
  }
}

// ── Spellcheck (issue #6) ───────────────────────────────────────────────────
//
// sdkjs ships a Hunspell engine compiled to WebAssembly
// (sdkjs/common/spell/spell/spell.js, loaded as a Web Worker) and our bundle has
// always carried it, unused: no dictionaries were packaged and no shell answered
// the editor. This wires both ends.
//
// The editor asks through AscDesktopEditor.SpellCheck(json) and expects the
// answer on asc_nativeOnSpellCheck inside its own frame. Both halves of that
// contract are installed by apiBase._coSpellCheckInit, which upstream only ever
// reaches from the co-authoring socket's auth reply (apiBase wires it to
// CoAuthoringApi.onSpellCheckInit). Offline there is no socket, so we call it
// ourselves from CreateEditorApi; it takes the AscDesktopEditor branch, installs
// asc_nativeOnSpellCheck and routes spellCheck/restart to us. It deliberately
// does NOT emit asc_onSpellCheckInit for a local file, which suits us: the list
// of languages upstream would send there is every language sdkjs knows, and we
// have two.
//
// The worker runs in the top window, not in the editor frame: it is created once
// and survives the iframe being torn down and rebuilt for the next document, so
// a dictionary is downloaded and parsed once per session.
//
// One rule the whole design hangs on, measured before it was written: asked
// about a language it has no dictionary for, the worker answers FALSE, not true
// (Dictionary.load marks a missing language "ready" with no data and Hunspell
// then fails every lookup). Letting it see an unpackaged language would
// underline every word of it. Unpackaged languages are therefore answered here,
// as correct, and never reach the worker.
var SpellCheckBridge = (function() {
  var WORKER_URL = 'sdkjs/common/spell/spell/spell.js';
  var MANIFEST_URL = 'dictionaries/manifest.json';
  var USER_DICT_KEY = 'eo-spell-userdict-v1';
  var USER_FOLDERS_COMMAND = 'list_user_dictionaries';
  // 0x0A, the primary language id shared by every Spanish locale.
  var SPANISH_PRIMARY_LANGUAGE = 10;

  var _worker = null;
  var _manifest = null;
  var _manifestPromise = null;
  var _packed = null;
  var _userWords = null;
  // Language folders found in the user's dictionaries directory. Null means the
  // answer has not arrived yet, which is not the same as an empty list.
  var _userFolders = null;
  var _userFoldersPromise = null;
  var _waitLogged = false;
  var _tasks = {};
  var _nextTaskId = 1;

  function _log(message) {
    window._eoLog('[SPELL] ' + message);
  }

  // The worker resolves dictionary URLs against this prefix, so it has to be
  // absolute. It points at the ascdesktop protocol rather than at the frontend
  // origin because that handler is the only place where the bundled
  // dictionaries and the user's folder can hide behind one base: the worker
  // asks for "<base>/<folder>/<folder>.aff" and never learns which of the two
  // answered. One base for every language also means one code path to keep
  // right, which is why the bundled ones moved over here too.
  //
  // ASC_PROTO_BASE already carries the per-platform spelling of that protocol
  // (a real custom scheme on mac and Linux, http://ascdesktop.localhost on
  // Windows, where WebView2 has no custom schemes), so it is reused rather than
  // spelled out again. Reaching it from inside the worker was verified on
  // WebKitGTK before this was written.
  function _dictionariesPath() {
    return ASC_PROTO_BASE + 'dictionaries';
  }

  // The Rust side answers with the folders that hold a usable pair of hunspell
  // files, plus the ones it refused. Both lists are logged: a folder that was
  // ignored has to say so somewhere, or a user who misnamed one is left with a
  // language that silently never appears.
  //
  // A failure here is not allowed to be fatal. An older binary without the
  // command, or any error at all, leaves the bridge with the bundled
  // dictionaries and a line in the log, which is exactly how v0.17.18 behaved.
  function _userFoldersReady() {
    if (_userFoldersPromise) return _userFoldersPromise;
    _userFoldersPromise = Promise.resolve().then(function() {
      return invoke(USER_FOLDERS_COMMAND);
    }).then(function(result) {
      var valid = (result && result.valid) || [];
      var skipped = (result && result.skipped) || [];
      _userFolders = valid.slice();
      if (valid.length) {
        _log('user dictionaries: ' + valid.join(', '));
      } else {
        _log('no user dictionaries installed');
      }
      if (skipped.length) {
        _log('user dictionary folders ignored, each needs <folder>.aff and ' +
             '<folder>.dic named after the folder: ' + skipped.join(', '));
      }
    }).catch(function(e) {
      _userFolders = [];
      _log('the user dictionary folder could not be read, using the bundled ' +
           'dictionaries only: ' + (e.message || e));
    });
    return _userFoldersPromise;
  }

  // Both lists have to be in before the language map is worth caching.
  function _dictionariesReady() {
    return Promise.all([_manifestReady(), _userFoldersReady()]);
  }

  function _manifestReady() {
    if (_manifestPromise) return _manifestPromise;
    _manifestPromise = fetch(MANIFEST_URL).then(function(response) {
      if (!response.ok) throw new Error('HTTP ' + response.status);
      return response.json();
    }).then(function(list) {
      _manifest = Array.isArray(list) ? list : [];
      _log('manifest loaded: ' + _manifest.join(', '));
    }).catch(function(e) {
      _manifest = [];
      _log('manifest unavailable, spellcheck disabled: ' + (e.message || e));
    });
    return _manifestPromise;
  }

  // Every Spanish LCID web-apps knows about. The low 10 bits of an LCID are its
  // primary language, and 0x0A is Spanish, so the whole es-* family answers this
  // test without a table of our own to keep in step.
  //
  // The list comes from web-apps' own LanguageInfo rather than from the sdkjs
  // spellcheck table, which names only es-ES. It has to be enumerated, not
  // matched at call time: these LCIDs travel as keys in the worker's languages
  // map (the worker resolves languages[lcid] by exact key) and as the array
  // behind asc_onSpellCheckInit, which is what paints the dictionary icon on
  // each entry of the language picker.
  //
  // Entries without a third field are kept too, even though the picker hides
  // them: what the picker offers and what a document is tagged with are
  // different things, and es-ES_tradnl (1034) in particular is the Spanish LCID
  // that older Word documents carry. An LCID the picker does not know is simply
  // never looked up there, so the extra entries cost nothing.
  function _spanishVariants(editorWindow) {
    var found = [];
    try {
      var common = editorWindow && editorWindow.Common;
      var info = common && common.util && common.util.LanguageInfo;
      var table = (info && typeof info.getLanguages === 'function') ? info.getLanguages() : null;
      if (!table) return found;
      for (var code in table) {
        if (!table.hasOwnProperty(code)) continue;
        var lcid = parseInt(code, 10);
        if (!lcid || (lcid & 0x3FF) !== SPANISH_PRIMARY_LANGUAGE) continue;
        found.push(String(lcid));
      }
    } catch(e) {
      _log('the language table was unreadable, Spanish variants not aliased: ' + (e.message || e));
    }
    return found;
  }

  // LCID -> dictionary folder, for the packaged languages only. The LCID table
  // is sdkjs's own (AscCommon.spellcheckGetLanguages), never a copy of it: two
  // LCIDs may share one folder and that mapping is upstream's to maintain.
  function _packedLanguages(editorWindow) {
    if (_packed) return _packed;
    if (!_manifest) return null;
    // Null means the backend has not answered yet, which is not an empty list.
    // Computing the map now would cache a bundled-only answer for the whole
    // session, so the user's languages would never appear however long the
    // wait was. A failed command lands here as an empty array, not as null, so
    // that case still resolves to bundled-only instead of blocking.
    if (_userFolders === null) return null;
    var table = null;
    try {
      if (editorWindow && editorWindow.AscCommon &&
          typeof editorWindow.AscCommon.spellcheckGetLanguages === 'function') {
        table = editorWindow.AscCommon.spellcheckGetLanguages();
      }
    } catch(e) {}
    if (!table) return null;

    var map = {};
    var found = [];
    // The user's folders are matched to LCIDs through the same sdkjs table the
    // bundled ones use, by exact folder name. That is why the README asks for
    // the name the dictionaries repo gives the language: a folder this table
    // does not name has no LCID to be offered under, and the editor only ever
    // asks about languages by LCID.
    var userFolders = _userFolders;
    var matched = {};
    var shared = [];
    for (var lcid in table) {
      if (!table.hasOwnProperty(lcid)) continue;
      var folder = table[lcid] && table[lcid].name;
      if (!folder) continue;
      var bundled = _manifest.indexOf(folder) !== -1;
      var fromUser = userFolders.indexOf(folder) !== -1;
      if (fromUser) {
        if (!matched[folder]) {
          matched[folder] = true;
          if (bundled) shared.push(folder);
        }
      }
      if (!bundled && !fromUser) continue;
      map[String(lcid)] = folder;
      found.push(lcid + '=' + folder + (bundled ? '' : ' (user)'));
    }
    // Spanish is a single orthography. The RAE norm is shared across the
    // Spanish-speaking world and the rla-es dictionary bundled here is
    // pan-Hispanic, so every Spanish LCID is answered from the es_ES folder
    // instead of only es-ES itself: a document written in es-419 or es-MX was
    // getting no spellcheck at all, because the sdkjs table only names 3082.
    //
    // English deliberately does NOT get the same treatment. en_GB and en_AU
    // disagree with en_US on the spelling of ordinary words (colour/color,
    // realise/realize), so aliasing them to the en_US dictionary would underline
    // correct text, which is worse than checking nothing.
    //
    // Accepted cost: the worker keys its dictionaries by LCID, so a document
    // that really mixes several Spanish variants loads the same two files once
    // per variant in use. Rare, bounded, and not worth the complexity of a
    // shared-file cache inside a worker we do not own.
    if (_manifest.indexOf('es_ES') !== -1) {
      var variants = _spanishVariants(editorWindow);
      var aliased = 0;
      for (var i = 0; i < variants.length; i++) {
        if (map[variants[i]]) continue;
        map[variants[i]] = 'es_ES';
        aliased++;
      }
      if (aliased > 0) found.push(aliased + ' more Spanish variants=es_ES');
    }

    _packed = map;
    _log('packaged languages: ' + (found.length ? found.join(', ') : 'none'));
    if (shared.length) {
      _log('these languages ship with the app and were also found in the user ' +
           'folder, whose files are read first: ' + shared.join(', '));
    }
    var unmatched = [];
    for (var u = 0; u < userFolders.length; u++) {
      if (!matched[userFolders[u]]) unmatched.push(userFolders[u]);
    }
    if (unmatched.length) {
      _log('user dictionary folders this build has no language id for, the ' +
           'folder needs the name the dictionaries repo uses: ' + unmatched.join(', '));
    }
    return _packed;
  }

  function _loadUserDict() {
    if (_userWords) return _userWords;
    _userWords = {};
    try {
      var raw = localStorage.getItem(USER_DICT_KEY);
      if (raw) {
        var list = JSON.parse(raw);
        if (list && list.length) {
          for (var i = 0; i < list.length; i++) {
            if (typeof list[i] === 'string' && list[i]) _userWords[list[i]] = true;
          }
        }
      }
    } catch(e) {
      _log('user dictionary unreadable, starting empty: ' + (e.message || e));
    }
    return _userWords;
  }

  function _saveUserDict() {
    try {
      var list = [];
      for (var word in _userWords) {
        if (_userWords.hasOwnProperty(word)) list.push(word);
      }
      localStorage.setItem(USER_DICT_KEY, JSON.stringify(list));
    } catch(e) {
      _log('user dictionary not saved: ' + (e.message || e));
    }
  }

  function _isUserWord(userWords, word) {
    if (!userWords || !word) return false;
    var text = String(word);
    return !!(userWords[text] || userWords[text.toLowerCase()]);
  }

  // Pure. Splits one editor task into the part the worker may answer and the
  // part answered here, and keeps the index map that puts them back together.
  function _splitTask(data, packed, userWords) {
    var words = data.usrWords || [];
    var langs = data.usrLang || [];
    var length = Math.min(words.length, langs.length);
    var isSpell = data.type === 'spell';
    var hasPositions = !!(data.usrPos && data.usrPosEnd);

    var correct = new Array(length);
    var suggest = new Array(length);
    var map = [];
    var subWords = [], subLangs = [], subPos = [], subPosEnd = [];

    for (var i = 0; i < length; i++) {
      var packaged = !!(packed && packed[String(langs[i])]);
      if (!packaged || (isSpell && _isUserWord(userWords, words[i]))) {
        correct[i] = true;
        suggest[i] = [];
        continue;
      }
      map.push(i);
      subWords.push(words[i]);
      subLangs.push(langs[i]);
      if (hasPositions) {
        subPos.push(data.usrPos[i]);
        subPosEnd.push(data.usrPosEnd[i]);
      }
    }

    var workerData = null;
    if (map.length > 0) {
      workerData = {
        type: data.type,
        usrWords: subWords,
        usrLang: subLangs,
        ParagraphId: data.ParagraphId,
        RecalcId: data.RecalcId
      };
      if (hasPositions) {
        workerData.usrPos = subPos;
        workerData.usrPosEnd = subPosEnd;
      }
    }

    return { workerData: workerData, map: map, correct: correct, suggest: suggest };
  }

  // Pure. The answer is the ORIGINAL task object with only the result field
  // replaced, so every echoed field the editor put there survives untouched
  // whether or not this code knows what it is for.
  function _mergeAnswer(task, response) {
    var answer = {};
    for (var key in task.original) {
      if (task.original.hasOwnProperty(key)) answer[key] = task.original[key];
    }
    var i;
    if (task.original.type === 'suggest') {
      var suggest = task.suggest.slice();
      for (i = 0; i < task.map.length; i++) {
        suggest[task.map[i]] = (response && response.usrSuggest && response.usrSuggest[i]) || [];
      }
      answer.usrSuggest = suggest;
      delete answer.usrCorrect;
    } else {
      var correct = task.correct.slice();
      for (i = 0; i < task.map.length; i++) {
        correct[task.map[i]] = !!(response && response.usrCorrect && response.usrCorrect[i]);
      }
      answer.usrCorrect = correct;
    }
    return answer;
  }

  function _answer(payload) {
    var ref = _getEditor();
    if (!ref.ew || typeof ref.ew.asc_nativeOnSpellCheck !== 'function') {
      _log('answer dropped: editor frame not listening');
      return;
    }
    // Worker replies are asynchronous and can arrive after the document has
    // moved on (a reopen or replay rebuilt the paragraphs), where the editor's
    // callback dereferences a paragraph state that no longer exists. A stale
    // reply must never throw upward: drop it.
    try {
      ref.ew.asc_nativeOnSpellCheck(payload);
    } catch(e) {
      _log('[SPELL] stale spellcheck response dropped: ' + ((e && e.message) || e));
    }
  }

  function _onWorkerMessage(data) {
    if (!data) return;
    var id = data.__eoTask;
    var task = (id !== undefined && id !== null) ? _tasks[id] : null;
    if (task) delete _tasks[id];
    if (!task) {
      _log('worker answer without a pending task, dropped');
      return;
    }
    _answer(_mergeAnswer(task, data));
  }

  function _ensureWorker(editorWindow) {
    if (_worker) return _worker;
    // The worker takes its language map once and ignores every later init, so
    // starting it before the user folders are known would lock those languages
    // out for the whole session. Waiting costs at most one request, which the
    // caller answers as correct rather than underlining anything, and the
    // editor asks again.
    if (!_userFolders) {
      if (!_waitLogged) {
        _waitLogged = true;
        _log('waiting for the user dictionary list before starting the worker');
      }
      return null;
    }
    var packed = _packedLanguages(editorWindow);
    if (!packed) return null;
    try {
      _worker = new Worker(WORKER_URL);
    } catch(e) {
      _log('worker could not be created: ' + (e.message || e));
      return null;
    }
    _worker.onerror = function(e) {
      _log('worker error: ' + ((e && e.message) || 'unknown') +
           ' at ' + ((e && e.filename) || '?') + ':' + ((e && e.lineno) || 0));
    };
    _worker.onmessage = function(event) {
      _onWorkerMessage(event.data);
    };
    _worker.postMessage({
      type: 'init',
      dictionaries_path: _dictionariesPath(),
      languages: packed
    });
    return _worker;
  }

  // apiBase.SpellCheckApi.restart sets isRestart and expects the "clear" echo to
  // clear it; every answer in flight until then is discarded by the editor. A
  // fresh worker is only built when the next task arrives.
  function _restart() {
    if (_worker) {
      try { _worker.terminate(); } catch(e) {}
      _worker = null;
    }
    _tasks = {};
    _answer('clear');
  }

  function _addWords(words) {
    if (!words || !words.length) return;
    var userWords = _loadUserDict();
    var changed = false;
    for (var i = 0; i < words.length; i++) {
      var word = String(words[i] || '');
      if (!word || userWords[word]) continue;
      userWords[word] = true;
      changed = true;
    }
    // The re-check is the editor's own move: asc_spellCheckAddToDictionary
    // calls _spellCheckRestart(word) right after this, so nothing to trigger.
    if (changed) _saveUserDict();
  }

  function _handleTask(data) {
    var ref = _getEditor();
    var packed = _packedLanguages(ref.ew) || {};
    var split = _splitTask(data, packed, _loadUserDict());
    var task = {
      original: data,
      map: split.map,
      correct: split.correct,
      suggest: split.suggest
    };

    if (!split.workerData) {
      _answer(_mergeAnswer(task, null));
      return;
    }

    var worker = _ensureWorker(ref.ew);
    if (!worker) {
      // Nothing to check against: answer everything as correct rather than
      // leaving the paragraph waiting forever for a reply.
      task.map = [];
      _answer(_mergeAnswer(task, null));
      return;
    }

    var id = _nextTaskId++;
    _tasks[id] = task;
    split.workerData.__eoTask = id;
    worker.postMessage(split.workerData);
  }

  function check(json) {
    try {
      if (json === 'clear') {
        _restart();
        return;
      }
      var data = (typeof json === 'string') ? JSON.parse(json) : json;
      if (!data) return;
      if (data.type === 'add') {
        _addWords(data.usrWords);
        return;
      }
      if (data.type !== 'spell' && data.type !== 'suggest') return;
      _handleTask(data);
    } catch(e) {
      _log('request failed: ' + (e.message || e));
    }
  }

  // A handler of web-apps is on the other side of this event, and what it does
  // with the list is not ours to guarantee. It must not be able to break the
  // chain it was called from: the spreadsheet's handler throws when the list
  // arrives before its side panel exists, and unguarded that surfaced as an
  // unhandled rejection and swallowed everything after the send.
  function _sendInit(api, lcids) {
    try {
      api.sendEvent('asc_onSpellCheckInit', lcids);
      _log('languages offered to the editor: ' + (lcids.length ? lcids.join(', ') : 'none'));
    } catch(e) {
      _log('the editor UI threw while taking the language list: ' + (e.message || e));
    }
  }

  // The spreadsheet builds its spellcheck side panel on the postload pass, well
  // after asc_onDocumentContentReady, and its own controller announces the new
  // panel with script:loaded:spellcheck immediately after creating it. That is
  // the first moment the list can be stored AND reach the panel's language
  // combo, so it is sent again there. Re-sending is idempotent (loadLanguages
  // rebuilds the whole list from the array), and the notification exists only in
  // the spreadsheet, so this is inert in the other two editors.
  function _watchSpellcheckPanel(api, editorWindow, lcids) {
    try {
      if (api.__eoSpellPanelWatched) return;
      var centre = editorWindow && editorWindow.Common && editorWindow.Common.NotificationCenter;
      if (!centre || typeof centre.on !== 'function') return;
      api.__eoSpellPanelWatched = true;
      centre.on('script:loaded:spellcheck', function() {
        if (api.__eoSpellPanelInit) return;
        api.__eoSpellPanelInit = true;
        _log('spellcheck panel built, sending the language list again');
        _sendInit(api, lcids);
      });
    } catch(e) {
      _log('could not watch for the spellcheck panel: ' + (e.message || e));
    }
  }

  function _emitInit(api, editorWindow) {
    _dictionariesReady().then(function() {
      var packed = _packedLanguages(editorWindow) || {};
      var lcids = [];
      for (var lcid in packed) {
        // Strings, not numbers: web-apps compares these against the string keys
        // of its own language table (Main.js loadLanguages, _.indexOf).
        if (packed.hasOwnProperty(lcid)) lcids.push(String(lcid));
      }
      _watchSpellcheckPanel(api, editorWindow, lcids);
      _sendInit(api, lcids);
    });
  }

  function attach(api) {
    if (!api || api.__eoSpellWired) return;
    try {
      if (typeof api._coSpellCheckInit !== 'function') {
        _log('this build has no _coSpellCheckInit, spellcheck not wired');
        return;
      }
      api.__eoSpellWired = true;
      // A new API means a new editor frame: anything still in flight belongs to
      // the frame that is gone and would answer into a dead window.
      _tasks = {};
      var editorWindow = window.AscDesktopEditor._editorWindow;
      api._coSpellCheckInit();

      // Without this every language reports a dictionary (CSpellCheckApi
      // answers a flat true offline), so the editor would queue work for all of
      // them. The pre-marking in _splitTask stays as the belt to this braces.
      if (api.SpellCheckApi) {
        api.SpellCheckApi.checkDictionary = function(lang) {
          var packed = _packedLanguages(window.AscDesktopEditor._editorWindow || editorWindow);
          return !!(packed && packed[String(lang)]);
        };
      }

      // asc_onSpellCheckInit has no replay: sent before the UI registers its
      // handler it is simply lost, and sent after the document is ready it
      // arrives in time for the language pickers either way.
      api.asc_registerCallback('asc_onDocumentContentReady', function() {
        _emitInit(api, window.AscDesktopEditor._editorWindow || editorWindow);
      });
      _log('bridge attached to the editor API');
    } catch(e) {
      _log('attach failed: ' + (e.message || e));
    }
  }

  // Both lists are asked for as soon as the bridge loads, so they are in well
  // before the first document is ready and the worker never has to wait.
  _dictionariesReady();

  return {
    check: check,
    attach: attach,
    // Exposed for the pure-function tests; not part of the editor contract.
    _splitTask: _splitTask,
    _mergeAnswer: _mergeAnswer
  };
})();

function _ensureCoreProps(ref) {
  try {
    var logicDoc = ref.editor.WordControl && ref.editor.WordControl.m_oLogicDocument;
    if (!logicDoc && ref.editor.wbModel) {
      logicDoc = ref.editor.wbModel;
    }
    if (logicDoc && !logicDoc.Core && ref.ew.AscCommon.CCore) {
      logicDoc.Core = new ref.ew.AscCommon.CCore();
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

    SpellCheckBridge.attach(api);
  },

  LocalStartOpen: function() {
    var ref = _getEditor();
    if (!ref.editor) return;

    var doOpen = function() {
      try {
        if (window._pendingFileData) {
          var pending = window._pendingFileData;
          window._pendingFileData = null;
          _loadEditorBin(pending.data, pending.path, {
            name: pending.name,
            path: pending.path,
            skipRecovery: !!pending.recovery,
            recovery: pending.recovery || null
          });
        } else {
          var emptyData = ref.ew.AscCommon.getEmpty();
          var file = new ref.ew.AscCommon.OpenFileResult();
          file.data = emptyData;
          file.bSerFormat = true;
          ref.editor.openDocument(file);
          ref.ew.AscCommon.History.UserSaveMode = true;
          _ensureCoreProps(ref);
          // A document that never existed on disk still holds work worth
          // recovering, and these are the bytes its changes apply to.
          _recoveryBegin(_eoBytesToB64(emptyData), _t('newDocument'), null);
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
      localStorage.setItem('eo-pending-open-path', path);
      _forceReload();
      return;
    }

    try {
      var b64data = await invoke('open_file', { path: path });
      _loadEditorBin(b64data, path, { path: path });
    } catch(e) {
      window._eoLog('[EO] Error opening file: ' + e);
      await _eoShowOpenError();
    }
  },

  LocalFileSave: async function(param, password, docinfo, fileType, jsonOptions) {
    var isSaveAs = param && param.indexOf('saveas=true') !== -1;
    if (window.AscDesktopEditor._isPrinting) return;

    var docType = window.AscDesktopEditor._currentDocType || 'word';
    window._eoLog('[SAVE] docType=' + docType + ' saveAs=' + !!isSaveAs + ' fileType=' + fileType);

    var ref = _getEditor();
    if (!ref.editor) {
      window._eoLog('[EO] LocalFileSave aborted: editor API not found');
      return;
    }

    try {
      var binData = ref.editor.asc_nativeGetFile();
      if (!binData) {
        window._eoLog('[EO] LocalFileSave aborted: asc_nativeGetFile returned no data');
        return;
      }

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
        var formatExtensions = {
          65: 'docx', 66: 'doc', 67: 'odt', 68: 'rtf', 69: 'txt',
          129: 'pptx', 130: 'ppt', 131: 'odp',
          257: 'xlsx', 258: 'xls', 259: 'ods', 260: 'csv',
          513: 'pdf'
        };
        var requestedExt = formatExtensions[fileType] || null;

        var filters = _eoSaveAsFilters(docType, requestedExt, currentPath, _isLinux);
        var dialog = window.__TAURI__.dialog;
        var savePath = await dialog.save({ filters: filters });
        if (savePath) {
          var knownExts = ['docx','doc','odt','rtf','txt','xlsx','xls','ods','csv','pptx','ppt','odp','pdf'];
          var pathExt = savePath.split('.').pop().toLowerCase();
          if (savePath.indexOf('.') === -1 || knownExts.indexOf(pathExt) === -1) {
            savePath += '.' + (requestedExt || filters[0].extensions[0]);
            pathExt = requestedExt || filters[0].extensions[0];
          }
          try {
            await invoke('save_file_as', { path: savePath });
            var savedName = savePath.replace(/\\/g, '/').split('/').pop();
            if (pathExt !== 'pdf') {
              // A Save As moves the document and the session follows it, so a
              // later recovery writes where it lives now. A PDF export only
              // writes a copy, so it is not a save for recovery purposes.
              invoke('recovery_mark_saved', {
                path: savePath,
                format: _eoFormatForPath(savePath),
                name: savedName
              }).catch(function(){});
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
            }
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
        try {
          await invoke('save_file', { data: '' });
          // Everything recorded so far is now inside the user's file, and the
          // count is what stops that folder from being offered as a crash. The
          // changes themselves stay: sdkjs keeps numbering deleteIndex from the
          // start of its session, not from the last save, so emptying the file
          // would strand it.
          invoke('recovery_mark_saved', {}).catch(function(){});
        } catch(saveErr) {
          // Same dialog the Save As path shows. A save can fail for reasons the
          // user is the only one who can act on, a read-only location or a full
          // disk, and the editor's own "saved" indicator says nothing about it:
          // without this the document looks written and is not.
          window._eoLog('[EO] Save failed: ' + saveErr);
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
      if (xhr.status !== 200) {
        window._eoLog('[EO] LocalFileGetImageUrl ' + protocol + ' failed: status ' + xhr.status +
          ' url=' + _eoLimitLogText(url, 160));
      }
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
    var ref = _getEditor();
    if (!ref.ew) { window._eoLog('[EO] WARN: Copy - editor context not available'); return; }
    var cb = ref.ew.AscCommon && ref.ew.AscCommon.g_clipboardBase;
    if (cb) {
      if (cb.inputContext && cb.inputContext.HtmlArea) cb.inputContext.HtmlArea.focus();
      if (cb.CommonDiv_Execute_CopyCut) cb.CommonDiv_Execute_CopyCut();
    }
    try {
      ref.ew.document.execCommand('copy');
    } catch(e) {
      window._eoLog('[EO] Copy: execCommand=error ' + (e.message || e));
    }
  },
  Paste: async () => {
    var platform = _isWindows ? 'win' : _isMac ? 'mac' : 'linux';
    var order = _isMac ? 'image-first' : 'text-first';
    window._eoLog('[CLIPBOARD] paste platform=' + platform + ' order=' + order);
    var imageFile = null;
    var text = null;
    var probeImage = async function() {
      try {
        imageFile = await ClipboardHelper.readNativeClipboardImage();
      } catch(e) {
        window._eoLog('[EO] Paste: image probe -> error ' + (e.message || e));
      }
    };
    var textTimedOut = false;
    var probeText = async function() {
      try {
        // read_clipboard_text hangs ~30s when our own webview owns the X11
        // selection - exactly the copy-then-paste-in-app flow.
        // Race it and fall back to the SDK's internal copy buffer on timeout;
        // the image probe is skipped too, it hangs the same way.
        var TIMED_OUT = { timedOut: true };
        var result = await Promise.race([
          invoke('read_clipboard_text'),
          new Promise(function(resolve) { setTimeout(function() { resolve(TIMED_OUT); }, 1200); })
        ]);
        if (result === TIMED_OUT) textTimedOut = true;
        else text = result;
      } catch(e) {
        window._eoLog('[EO] Paste: text probe -> error ' + (e.message || e));
      }
    };
    if (_isMac) {
      await probeImage();
      if (!imageFile) await probeText();
    } else {
      await probeText();
      // A single-image-file copy from a file manager arrives with the file
      // PATH as the text flavor; prefer the image probe there (Issue #24
      // arc, L2: context menu pasted the path as text).
      if (!textTimedOut && (!text || ClipboardHelper.looksLikeImageFilePath(text))) await probeImage();
    }
    if (textTimedOut) {
      try {
        if (_eoPasteFromLastCopyBinary(_getEditor())) {
          window._eoLog('[CLIPBOARD] paste result=internal');
        } else {
          window._eoLog('[CLIPBOARD] paste result=timeout-no-internal');
        }
      } catch(e) {
        window._eoLog('[EO] Paste: branch=internal -> error ' + (e.message || e));
      }
      return;
    }
    if (imageFile && (!text || _isMac || ClipboardHelper.looksLikeImageFilePath(text))) {
      try {
        var ref = _getEditor();
        if (ref.editor) {
          ref.editor.AddImageUrl([imageFile]);
          window._eoLog('[CLIPBOARD] paste result=image');
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
          window._eoLog('[CLIPBOARD] paste result=text');
        }
      } catch(e) {
        window._eoLog('[EO] Paste: branch=text -> error ' + (e.message || e));
      }
      return;
    }
    window._eoLog('[CLIPBOARD] paste result=empty');
  },
  Cut: function() {
    var ref = _getEditor();
    if (!ref.ew) { window._eoLog('[EO] WARN: Cut - editor context not available'); return; }
    var cb = ref.ew.AscCommon && ref.ew.AscCommon.g_clipboardBase;
    if (cb) {
      if (cb.inputContext && cb.inputContext.HtmlArea) cb.inputContext.HtmlArea.focus();
      if (cb.CommonDiv_Execute_CopyCut) cb.CommonDiv_Execute_CopyCut();
    }
    var execOk = false;
    try {
      execOk = ref.ew.document.execCommand('cut');
    } catch(e) {
      window._eoLog('[EO] Cut: execCommand=error ' + (e.message || e));
    }
    // WebKit can refuse the cut command on sdkjs's collapsed DOM selection
    // mirror sdkjs's own Button_Cut fallback: copy natively,
    // then delete the selection.
    if (!execOk) {
      try { ref.ew.document.execCommand('copy'); } catch(e) {}
      if (ref.editor && ref.editor.asc_SelectionCut) ref.editor.asc_SelectionCut();
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
      if (!binData) {
        window._eoLog('[EO] Print aborted: asc_nativeGetFile returned no data');
        return;
      }

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
    } else if (cmd === 'open:recent') {
      try {
        var recent = JSON.parse(param);
        // LocalFileOpen already handles the unsaved-changes confirmation and the
        // reload needed to switch documents.
        if (recent && recent.path) window.AscDesktopEditor.LocalFileOpen(recent.path);
      } catch(e) {
        window._eoLog('[EO] open:recent parse error: ' + (e.message || e));
      }
    } else if (cmd === 'recent:forget') {
      invoke('clear_recent_files').then(function() {
        window.AscDesktopEditor.LocalFileRecents();
      }).catch(function(e) {
        window._eoLog('[EO] recent:forget error: ' + (e.message || e));
      });
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
      // x2t serializes Windows extended paths as /?/C:/... in AllFonts.js.
      // Normalize them before routing through the native protocol handler.
      if (fontId && /^\/\?\/[A-Za-z]:\//.test(fontId)) {
        fontId = fontId.substring(3);
      }
      var xhr = new XMLHttpRequest();
      var isAbs = fontId && fontId.length > 2 &&
        ((fontId[0] === '/' && fontId[1] !== '/') || (fontId[1] === ':'));
      var url = isAbs ? ASC_PROTO_BASE + 'abs/' + fontId : '/fonts/' + fontId;
      xhr.open('GET', url, false);
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
    // sdkjs joins the batch with the literal sequence "," and only hands over
    // the raw array past 100000 changes (common/Local/common.js:405-409).
    // Splitting here is what makes both branches reach Rust as the same array
    // instead of two different encodings of the same thing.
    var list;
    if (typeof changes === 'string') {
      list = changes.length ? changes.split('","') : [];
    } else {
      list = changes || [];
    }
    // An empty batch is not noise: a save and an undo both arrive as one, and
    // the index they carry is the whole message.
    invoke('save_changes', {
      changes: list,
      deleteIndex: (deleteIndex === undefined ? null : deleteIndex),
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
  SpellCheck: function(json) { SpellCheckBridge.check(json); },
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
  // The slide editor relays pen strokes and presenter commands to a separate
  // presenter window through this call. There is no such window here, so
  // dropping them is correct; without the stub the call throws a TypeError
  // inside onMouseDown and the editor stops accepting edits.
  sendToReporter: function() {},
  ConsoleLog: (msg) => console.log('[EO]', msg),

  NativeViewerOpen: function() {},
  SetAdvancedOptions: function() {},
  LocalFileRecoverFolder: () => '',
  LocalFileRemoveRecoverFolder: function() {},
  InitRecoverFolder: function() {},
  GetRecoverFolder: () => '',
  // Desktop.js asks for the list once the editor is ready and expects the shell
  // to answer through window.onupdaterecents in the editor frame. Each editor
  // then keeps only the formats it can open (utils.matchFileFormat), so the
  // same payload serves all three; its format ids are x2t's, the ones
  // detect_format already returns on the Rust side.
  LocalFileRecents: function() {
    invoke('recent_files_state').then(function(state) {
      var files = (state && state.enabled && state.files) ? state.files : [];
      var payload = files.map(function(file, index) {
        return { id: index, path: file.path, type: file.fileType };
      });
      var ref = _getEditor();
      var target = ref.ew && typeof ref.ew.onupdaterecents === 'function' ? ref.ew : null;
      if (!target) {
        window._eoLog('[RECENT] editor list skipped: onupdaterecents unavailable');
        return;
      }
      target.onupdaterecents(payload);
      window._eoLog('[RECENT] editor list sent: entries=' + payload.length);
    }).catch(function(e) {
      window._eoLog('[EO] LocalFileRecents error: ' + (e.message || e));
    });
  },
  LocalFileRecover: function() {},
  // Not a stub any more: a non-zero answer here is the whole reason a recovered
  // document reports itself as modified. Have_Changes(undefined, true), the
  // query that feeds onDocumentModifiedChanged (word/Local/api.js), looks at
  // nothing else.
  LocalFileGetOpenChangesCount: function() { return _eoOpenChangesCount; },
  LocalFileGetOpenChanges: function() { return ''; },
  LocalFileSetOpenChangesCount: function(count) { _eoOpenChangesCount = count | 0; },
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
    _loadEditorBin(event.payload.data, event.payload.path, { path: event.payload.path });
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

async function _eoOpenFromPath(filePath) {
  var docType = _eoDocTypeForPath(filePath);
  _eoShowOpening(_eoOpeningFileText(filePath));

  try {
    var b64data = await invoke('open_file', { path: filePath });
    var fileName = filePath.replace(/\\/g, '/').split('/').pop();
    window._pendingFileData = { data: b64data, path: filePath, name: fileName };
    if (window._openEditor) {
      window._openEditor(docType);
    } else {
      window._eoLog('[OPEN] WARN: editor launcher unavailable, using fallback');
      // The fallback never reaches openEditor, which is what hides it.
      _eoHideOpening();
      window.AscDesktopEditor.LocalFileOpen(filePath);
    }
  } catch(e) {
    window._eoLog('[EO] open-file: conversion failed: ' + (e.message || e));
    _eoHideOpening();
    await _eoShowOpenError();
  }
}

// An event emitted by the backend at startup is lost if no listener exists yet,
// so the page asks for the launch argument instead, once DOMContentLoaded
// guarantees the later scripts (and window._openEditor) have run.
function _eoTakePendingOpenFile() {
  invoke('take_pending_open_file').then(function(filePath) {
    if (filePath) return _eoOpenFromPath(filePath);
  }).catch(function(e) {
    window._eoLog('[EO] open-file: pending path unavailable: ' + (e.message || e));
  });
}
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', _eoTakePendingOpenFile);
} else {
  _eoTakePendingOpenFile();
}
