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

function _findEditorWindow(win) {
  try { if (win.AscCommon) return win; } catch(e) {}
  for (var i = 0; i < win.frames.length; i++) {
    var found = _findEditorWindow(win.frames[i]);
    if (found) return found;
  }
  return null;
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

    try {
      var b64data = await invoke('open_file', { path: path });
      _loadEditorBin(b64data, path);
    } catch(e) {
      window._eoLog('[EO] Error opening file:', e);
    }
  },

  LocalFileSave: async function(param, password, docinfo, fileType, jsonOptions) {
    var isSaveAs = param && param.indexOf('saveas=true') !== -1;
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

      if (isSaveAs) {
        var dialog = window.__TAURI__.dialog;
        var savePath = await dialog.save({
          filters: [
            { name: 'Word', extensions: ['docx'] },
            { name: 'Excel', extensions: ['xlsx'] },
            { name: 'PowerPoint', extensions: ['pptx'] },
            { name: 'PDF', extensions: ['pdf'] },
            { name: 'OpenDocument Text', extensions: ['odt'] },
            { name: 'Rich Text', extensions: ['rtf'] },
          ]
        });

        if (savePath) {
          await invoke('save_file_as', { path: savePath });
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
  LocalFileGetModified: () => false,
  LocalFileSetModified: function(modified) {
    invoke('set_document_modified', { modified }).catch(function(){});
  },

  GetOpenedFile: function(data) { return null; },

  Copy: () => document.execCommand('copy'),
  Paste: () => document.execCommand('paste'),
  Cut: () => document.execCommand('cut'),

  Print: async () => {
    return await invoke('print_document');
  },
  IsSupportNativePrint: () => true,

  onDocumentModifiedChanged: (modified) => {
    invoke('set_document_modified', { modified }).catch(function(){});
  },
  SetDocumentName: (name) => {
    invoke('set_window_title', { name }).catch(function(){});
  },

  execCommand: function(cmd, param) {
    if (cmd === 'saveas') {
      window.AscDesktopEditor.LocalFileSave('saveas=true;', '', undefined, 0, '{}');
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
