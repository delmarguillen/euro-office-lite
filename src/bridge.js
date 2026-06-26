const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

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
  window.location.reload();
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

    var file = new ref.ew.AscCommon.OpenFileResult();
    file.data = bytes;
    file.bSerFormat = true;
    ref.editor.openDocument(file);
    ref.ew.AscCommon.History.UserSaveMode = true;

    if (fileName) {
      var name = fileName.replace(/\\/g, '/').split('/').pop();
      invoke('set_window_title', { name: name }).catch(function(){});
    }
  } catch(e) {
    window._eoLog('[EO] Error loading document:', e.message);
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
        }
      } catch(e) {
        window._eoLog('[EO] LocalStartOpen error:', e.message);
      }
    };

    setTimeout(doOpen, 100);
  },

  CheckUserId: () => 'local-user',

  LocalFileOpen: async function(path) {
    if (!path) {
      var dialog = window.__TAURI__.dialog;
      path = await dialog.open({
        filters: [
          { name: 'Documentos', extensions: ['docx', 'xlsx', 'pptx', 'odt', 'ods', 'odp', 'rtf', 'txt', 'csv', 'pdf'] },
          { name: 'Todos', extensions: ['*'] }
        ]
      });
    }
    if (!path) return;

    if (window.AscDesktopEditor._currentDocType) {
      if (window.AscDesktopEditor._isModified) {
        var discard = await window.__TAURI__.dialog.confirm(
          'El documento actual tiene cambios sin guardar. ¿Desea descartarlos y abrir otro archivo?',
          { title: 'Cambios sin guardar', kind: 'warning' }
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
            { name: 'Texto plano', extensions: ['txt'] },
            { name: 'PDF', extensions: ['pdf'] },
          ];
        }
        var dialog = window.__TAURI__.dialog;
        var savePath = await dialog.save({ filters: filters });

        if (savePath) {
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
              'No se pudo guardar el archivo.\n\nFormato de destino no compatible con este tipo de documento.',
              { title: 'Error al guardar', kind: 'error' }
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

  LocalFileGetSourcePath: () => '',
  LocalFileGetSaved: () => false,
  LocalFileGetImageUrl: (url) => url,
  LocalFileGetModified: function() { return window.AscDesktopEditor._isModified; },
  LocalFileSetModified: function(modified) {
    window.AscDesktopEditor._isModified = modified;
    invoke('set_document_modified', { modified }).catch(function(){});
  },

  GetOpenedFile: function(data) { return null; },

  Copy: () => document.execCommand('copy'),
  Paste: () => document.execCommand('paste'),
  Cut: () => document.execCommand('cut'),

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
  SetDocumentName: (name) => {
    invoke('set_window_title', { name }).catch(function(){});
  },

  execCommand: function(cmd, param) {
    if (cmd === 'saveas') {
      window.AscDesktopEditor.LocalFileSave('saveas=true;', '', undefined, 0, '{}');
    } else if (cmd === 'editor:event') {
      try {
        var evt = JSON.parse(param);
        if (evt.action === 'file:open') {
          window.AscDesktopEditor.LocalFileOpen();
        } else if (evt.action === 'file:close') {
          (async function() {
            if (window.AscDesktopEditor._isModified) {
              var discard = await window.__TAURI__.dialog.confirm(
                'El documento actual tiene cambios sin guardar. ¿Desea descartarlos y cerrar?',
                { title: 'Cambios sin guardar', kind: 'warning' }
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
  window._eoLog('[CLOSE] confirm-close event received from Rust');
  window._eoLog('[CLOSE] _isModified: ' + window.AscDesktopEditor._isModified);
  if (_closeDialogOpen) {
    window._eoLog('[CLOSE] Dialog already open, ignoring');
    return;
  }
  _closeDialogOpen = true;
  try {
    if (window.AscDesktopEditor._isModified) {
      window._eoLog('[CLOSE] Showing confirmation dialog');
      var discard = await window.__TAURI__.dialog.confirm(
        'El documento actual tiene cambios sin guardar. ¿Desea descartarlos y cerrar?',
        { title: 'Cambios sin guardar', kind: 'warning' }
      );
      window._eoLog('[CLOSE] User chose: ' + (discard ? 'discard' : 'cancel'));
      if (!discard) return;
    }
    window._eoLog('[CLOSE] Calling force_close');
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
