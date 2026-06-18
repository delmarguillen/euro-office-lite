const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

window.AscDesktopEditor = {
  IsLocalFile: () => true,
  GetEditorId: () => 'euro-office-lite',

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
    var ew = window.AscDesktopEditor._editorWindow || window;
    var editor = ew.Asc && ew.Asc.editor;

    if (!editor) {
      console.error('[EO] LocalStartOpen: no editor found');
      return;
    }

    setTimeout(function() {
      try {
        var emptyData = ew.AscCommon.getEmpty();
        var file = new ew.AscCommon.OpenFileResult();
        file.data = emptyData;
        file.bSerFormat = true;
        editor.openDocument(file);
        ew.AscCommon.History.UserSaveMode = true;
        console.log('[EO] Empty document opened via openDocument');
      } catch(e) {
        console.error('[EO] LocalStartOpen error:', e);
      }
    }, 100);
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

  LoadFontBase64: async (name) => {
    return await invoke('load_font', { name });
  },

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
