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
    // fallback: try top window invoke
    try { window.top.__TAURI__.core.invoke('js_log', { msg: msg }); } catch(e2) {}
  }
};

window.AscDesktopEditor = {
  IsLocalFile: () => true,
  GetEditorId: () => 'euro-office-lite',

  getFontsSprite: function(suffix) {
    suffix = suffix || '';
    return '../../../../sdkjs/common/Images/fonts_thumbnail' + suffix + '.png';
  },
  isSupportBinaryFontsSprite: false,

  _editorWindow: null,

  CreateEditorApi: function(api) {
    // Store reference to the editor's window (the iframe where sdkjs lives)
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

    // Also set it in the calling context
    if (!window.AscDesktopEditor._editorWindow) {
      // Find the window that has AscCommon
      var findEditorWindow = function(win) {
        try { if (win.AscCommon) return win; } catch(e) {}
        for (var i = 0; i < win.frames.length; i++) {
          var found = findEditorWindow(win.frames[i]);
          if (found) return found;
        }
        return null;
      };
      window.AscDesktopEditor._editorWindow = findEditorWindow(window);
    }

    console.log('[EO] Editor API registered, editorWindow found:', !!window.AscDesktopEditor._editorWindow);
  },

  LocalStartOpen: function() {
    window._eoLog('[EO] LocalStartOpen called');
    var ew = window.AscDesktopEditor._editorWindow || window;
    var editor = ew.Asc && ew.Asc.editor;

    window._eoLog('[EO] LocalStartOpen: editorWindow:', !!ew, 'editor:', !!editor);

    if (!editor) {
      window._eoLog('[EO] LocalStartOpen: no editor found');
      return;
    }

    var doOpen = function() {
      try {
        window._eoLog('[EO] LocalStartOpen: opening empty document...');
        var emptyData = ew.AscCommon.getEmpty();
        var file = new ew.AscCommon.OpenFileResult();
        file.data = emptyData;
        file.bSerFormat = true;
        editor.openDocument(file);
        ew.AscCommon.History.UserSaveMode = true;
        window._eoLog('[EO] Empty document opened via openDocument');
      } catch(e) {
        window._eoLog('[EO] LocalStartOpen error: ' + e.message);
      }
    };

    setTimeout(doOpen, 100);
  },

  CheckUserId: () => 'local-user',

  LocalFileOpen: async (path) => {
    if (!path) {
      const { open } = window.__TAURI__.dialog;
      path = await open({
        filters: [
          { name: 'Documentos', extensions: ['docx', 'xlsx', 'pptx', 'odt', 'ods', 'odp', 'rtf', 'txt', 'csv', 'pdf'] },
          { name: 'Todos', extensions: ['*'] }
        ]
      });
    }
    if (path) {
      return await invoke('open_file', { path });
    }
  },

  LocalFileSave: async (data) => {
    return await invoke('save_file', { data: data || '' });
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
    console.log('[EO] execCommand:', cmd);
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

  LocalFileSaveChanges: function(changes, deleteIndex, count) {},
  OnSave: function() {},

  GetInstallPlugins: () => JSON.stringify([
    { url: '', pluginsData: [] },
    { url: '', pluginsData: [] }
  ]),

  IsSignaturesSupport: () => false,
  IsProtectionSupport: () => false,
  ConsoleLog: (msg) => console.log('[EO]', msg),

  // Stubs for desktop integration
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

// Desktop theme/renderer variables
window.RendererProcessVariable = {
  theme: { current: 'light', system: 'disabled' },
  localthemes: [],
};


// Stub for DesktopAfterOpen callback
window.DesktopAfterOpen = window.DesktopAfterOpen || function(editor) {
  console.log('[EO] Document opened in editor');
};

// Stub for UpdateInstallPlugins
window.UpdateInstallPlugins = window.UpdateInstallPlugins || function() {};

listen('file-opened', (event) => {
  if (window.AscDesktopEditor._onFileOpened) {
    window.AscDesktopEditor._onFileOpened(event.payload);
  }
});
