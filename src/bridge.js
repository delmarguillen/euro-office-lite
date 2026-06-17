const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

window.AscDesktopEditor = {
  IsLocalFile: () => true,
  GetEditorId: () => 'euro-office-tauri',

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

  LocalFileGetSourcePath: async () => {
    return await invoke('get_current_path');
  },

  Copy: () => document.execCommand('copy'),
  Paste: () => document.execCommand('paste'),
  Cut: () => document.execCommand('cut'),

  Print: async () => {
    return await invoke('print_document');
  },
  IsSupportNativePrint: () => true,

  onDocumentModifiedChanged: (modified) => {
    invoke('set_document_modified', { modified });
  },
  SetDocumentName: (name) => {
    invoke('set_window_title', { name });
  },

  execCommand: async (cmd, param) => {
    return await invoke('exec_command', { cmd, param });
  },

  LoadFontBase64: async (name) => {
    return await invoke('load_font', { name });
  },

  GetInstallPlugins: () => '{"pluginsData":[]}',
  IsSignaturesSupport: () => false,
  IsProtectionSupport: () => false,
  ConsoleLog: (msg) => console.log('[EO]', msg),
};

listen('file-opened', (event) => {
  if (window.AscDesktopEditor._onFileOpened) {
    window.AscDesktopEditor._onFileOpened(event.payload);
  }
});
