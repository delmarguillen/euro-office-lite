// The Markdown page: a rendered preview of a .md, and a textarea beside it to
// edit the text. Reached from the start screen, Open and Recents through
// bridge.js (_eoOpenMarkdown), which leaves the file's token under
// PENDING_KEY. The file is read and written as bytes by markdown.rs; never
// through x2t or the ONLYOFFICE editors.
//
// The preview is the document's HTML after DOMPurify, inside an iframe
// sandboxed without allow-scripts: nothing from the document runs, and its
// links do not navigate. The page's own CSP (markdown.html) adds no inline
// script, no eval and no network.
(function() {
  'use strict';

  var tauri = window.__TAURI__;
  var invoke = tauri.core.invoke;
  var Text = window.EoMarkdownText;

  // Written by _eoOpenMarkdown in bridge.js.
  var PENDING_KEY = 'eo-markdown-open';
  var START_PAGE = '../index.html';
  var RENDER_DELAY_MS = 250;
  var SAVE_FILTERS = [{ name: 'Markdown', extensions: ['md', 'markdown'] }];
  var SAVE_DENIED_ERROR = 'save-denied';
  var TOO_LARGE_ERROR = 'markdown-too-large';

  // ── Interface text. Same language choice as bridge.js (_detectLang). ──
  var STRINGS = {
    en: {
      close: 'Close', edit: 'Edit', preview: 'Preview', save: 'Save', saveAs: 'Save as…',
      source: 'Markdown text', previewTitle: 'Preview',
      unsavedChanges: 'Unsaved changes',
      unsavedDiscardClose: 'The current document has unsaved changes. Discard them and close?',
      openFailed: 'Could not open file',
      openErrorMsg: 'The file could not be opened.',
      tooLargeMsg: 'The file is larger than 10 MB and cannot be opened as text.',
      notUtf8Msg: 'The file is not UTF-8 text. Opening it here could change it when saved.',
      saveError: 'Save error',
      saveErrorMsg: 'The file could not be saved.',
      saveDeniedMsg: 'The file could not be saved.\n\nYou do not have permission to write to that location, or the file is read-only.',
      saveTooLargeMsg: 'The text is larger than 10 MB and cannot be saved.',
      remoteImageBlocked: 'Remote image not loaded',
      imageNotShown: 'Image not shown'
    },
    es: {
      close: 'Cerrar', edit: 'Editar', preview: 'Vista previa', save: 'Guardar', saveAs: 'Guardar como…',
      source: 'Texto Markdown', previewTitle: 'Vista previa',
      unsavedChanges: 'Cambios sin guardar',
      unsavedDiscardClose: 'El documento actual tiene cambios sin guardar. ¿Descartarlos y cerrar?',
      openFailed: 'No se pudo abrir el archivo',
      openErrorMsg: 'No se pudo abrir el archivo.',
      tooLargeMsg: 'El archivo ocupa más de 10 MB y no se puede abrir como texto.',
      notUtf8Msg: 'El archivo no es texto UTF-8. Abrirlo aquí podría cambiarlo al guardar.',
      saveError: 'Error al guardar',
      saveErrorMsg: 'No se pudo guardar el archivo.',
      saveDeniedMsg: 'No se pudo guardar el archivo.\n\nNo tienes permiso para escribir en esa ubicación o el archivo es de solo lectura.',
      saveTooLargeMsg: 'El texto ocupa más de 10 MB y no se puede guardar.',
      remoteImageBlocked: 'Imagen remota no cargada',
      imageNotShown: 'Imagen no mostrada'
    }
  };

  function detectLang() {
    var stored = null;
    try { stored = localStorage.getItem('eo-ui-lang'); } catch (e) {}
    var lang = (stored || navigator.language || 'en').toLowerCase();
    if (STRINGS[lang]) return lang;
    var prefix = lang.split('-')[0];
    return STRINGS[prefix] ? prefix : 'en';
  }

  var lang = detectLang();

  function t(key) {
    return STRINGS[lang][key] || STRINGS.en[key] || key;
  }

  function log(message) {
    invoke('js_log', { msg: '[MD] ' + message }).catch(function() {});
  }

  window.addEventListener('error', function(e) { log('error: ' + (e.message || e)); });
  window.addEventListener('unhandledrejection', function(e) {
    log('rejected: ' + ((e.reason && e.reason.message) || e.reason));
  });

  // ── Page elements ──
  var source = document.getElementById('md-source');
  var frame = document.getElementById('md-preview');
  var nameLabel = document.getElementById('md-name');
  var editButton = document.getElementById('md-edit');

  function applyStrings() {
    document.documentElement.lang = lang;
    var labelled = document.querySelectorAll('[data-i18n]');
    for (var i = 0; i < labelled.length; i++) {
      labelled[i].textContent = t(labelled[i].dataset.i18n);
    }
    source.setAttribute('aria-label', t('source'));
    frame.title = t('previewTitle');
  }

  // ── The open file ──
  // The file's own BOM and line end (EoMarkdownText.decode), and the text as
  // last read or saved: the document is modified while the textarea differs.
  var format = { bom: false, eol: '\n' };
  var savedText = '';
  var reportedModified = false;
  var saving = false;

  function isModified() {
    return source.value !== savedText;
  }

  // set_document_modified gates the close warning in Rust and marks the title.
  function reportModified(force) {
    var modified = isModified();
    if (modified === reportedModified && !force) return Promise.resolve();
    reportedModified = modified;
    return invoke('set_document_modified', { modified: modified }).catch(function() {});
  }

  function base64ToBytes(base64) {
    var binary = atob(base64);
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }

  // In slices: String.fromCharCode takes a limited number of arguments.
  function bytesToBase64(bytes) {
    var parts = [];
    for (var i = 0; i < bytes.length; i += 0x8000) {
      parts.push(String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000)));
    }
    return btoa(parts.join(''));
  }

  function errorText(error) {
    return String((error && error.message) || error || '');
  }

  // ── Preview ──
  var markdown = window.markdownit({ html: true, linkify: false, typographer: false })
    .use(window.markdownitFootnote)
    .use(window.markdownitTaskLists);

  var PURIFY_CONFIG = {
    FORBID_TAGS: ['style', 'form'],
    FORBID_ATTR: ['style', 'srcset', 'target'],
    RETURN_DOM_FRAGMENT: true
  };

  // markdown-it aligns table cells with a style attribute, which the CSP
  // refuses; the three alignments become classes before styles are dropped.
  window.DOMPurify.addHook('beforeSanitizeAttributes', function(node) {
    if (!node.getAttribute) return;
    var style = node.getAttribute('style') || '';
    var match = /^\s*text-align:\s*(left|right|center)\s*;?\s*$/.exec(style);
    if (match) node.setAttribute('class', 'eo-align-' + match[1]);
  });

  // Links do not navigate: only a jump inside the document (a footnote) keeps
  // its target. The address stays visible as the link's tooltip.
  window.DOMPurify.addHook('afterSanitizeAttributes', function(node) {
    if (!node.getAttribute) return;
    node.removeAttribute('xlink:href');
    var href = node.getAttribute('href');
    if (href === null) return;
    if (node.tagName === 'A' && href.charAt(0) === '#') return;
    node.removeAttribute('href');
    if (node.tagName === 'A') node.setAttribute('title', href);
  });

  // Local images, by their path inside the document's folder:
  // { url } once read, { failed: true } when refused, a Promise meanwhile.
  var images = new Map();

  function placeholder(text, alt) {
    var span = document.createElement('span');
    span.className = 'eo-image-placeholder';
    span.textContent = alt ? text + ': ' + alt : text;
    return span;
  }

  function imageFailed(img, kind) {
    var key = kind === 'remote' ? 'remoteImageBlocked' : 'imageNotShown';
    img.replaceWith(placeholder(t(key), img.getAttribute('alt')));
  }

  function placeImage(img) {
    var found = Text.classifyImageSource(img.getAttribute('src'));
    img.removeAttribute('src');
    if (found.kind !== 'local') {
      imageFailed(img, found.kind);
      return;
    }
    var cached = images.get(found.path);
    if (cached && cached.url) {
      img.setAttribute('src', cached.url);
      return;
    }
    if (cached && cached.failed) {
      imageFailed(img, 'refused');
      return;
    }
    if (!cached) loadImage(found.path);
  }

  function loadImage(path) {
    var loading = invoke('markdown_image', { src: path }).then(function(image) {
      images.set(path, { url: 'data:' + image.mime + ';base64,' + image.data });
    }, function(error) {
      log('image refused: ' + errorText(error));
      images.set(path, { failed: true });
    }).then(scheduleRender);
    images.set(path, loading);
  }

  function previewFragment(text) {
    var fragment = window.DOMPurify.sanitize(markdown.render(text), PURIFY_CONFIG);
    var imgs = fragment.querySelectorAll('img');
    for (var i = 0; i < imgs.length; i++) placeImage(imgs[i]);
    return fragment;
  }

  var renderTimer = null;

  function scheduleRender() {
    clearTimeout(renderTimer);
    renderTimer = setTimeout(render, RENDER_DELAY_MS);
  }

  // The body is replaced in place, so the preview keeps its scroll position.
  function render() {
    clearTimeout(renderTimer);
    var frameDocument = frame.contentDocument;
    if (!frameDocument || !frameDocument.body) return;
    frameDocument.body.replaceChildren(previewFragment(source.value));
  }

  // The stylesheet is the page's own; the srcdoc document shares its base URL.
  function prepareFrame() {
    return new Promise(function(resolve) {
      frame.addEventListener('load', function() {
        frame.contentDocument.addEventListener('keydown', onKeyDown, true);
        resolve();
      }, { once: true });
      frame.srcdoc = '<!DOCTYPE html><html><head><meta charset="utf-8">' +
        '<link rel="stylesheet" href="markdown-preview.css"></head>' +
        '<body class="markdown-body"></body></html>';
    });
  }

  // ── Edit, save, close ──
  function setEditing(editing) {
    document.body.classList.toggle('editing', editing);
    source.hidden = !editing;
    editButton.setAttribute('aria-pressed', editing ? 'true' : 'false');
    editButton.dataset.i18n = editing ? 'preview' : 'edit';
    editButton.textContent = t(editButton.dataset.i18n);
    if (editing) source.focus();
  }

  function saveErrorText(error) {
    var text = errorText(error);
    if (text.indexOf(SAVE_DENIED_ERROR) === 0) return t('saveDeniedMsg');
    if (text.indexOf(TOO_LARGE_ERROR) === 0) return t('saveTooLargeMsg');
    return t('saveErrorMsg');
  }

  async function pickSaveTarget() {
    return invoke('pick_save_target', { filters: SAVE_FILTERS, requestedExt: 'md' });
  }

  // Save writes over the open file; Save As (asNew) where the user picks.
  async function save(asNew) {
    if (saving) return;
    saving = true;
    try {
      var target = asNew ? await pickSaveTarget() : null;
      if (asNew && !target) return;
      var text = source.value;
      var data = bytesToBase64(Text.encode(text, format));
      var name = await invoke('save_markdown', { data: data, token: target ? target.token : null });
      savedText = text;
      showName(name);
      await invoke('set_window_title', { name: name }).catch(function() {});
      // The new title carries no mark: report the state again even if unchanged.
      await reportModified(true);
    } catch (error) {
      log('save failed: ' + errorText(error));
      await tauri.dialog.message(saveErrorText(error), { title: t('saveError'), kind: 'error' });
    } finally {
      saving = false;
    }
  }

  async function confirmDiscard() {
    if (!isModified()) return true;
    return tauri.dialog.confirm(t('unsavedDiscardClose'),
      { title: t('unsavedChanges'), kind: 'warning' });
  }

  async function leaveToStart() {
    await invoke('set_document_modified', { modified: false }).catch(function() {});
    window.location.href = START_PAGE;
  }

  async function closeDocument() {
    if (!(await confirmDiscard())) return;
    await leaveToStart();
  }

  // The window's close button, sent by the backend while the document is
  // modified (the same event bridge.js answers for the editors).
  var closeDialogOpen = false;
  tauri.event.listen('confirm-close', async function() {
    if (closeDialogOpen) return;
    closeDialogOpen = true;
    try {
      if (!(await confirmDiscard())) return;
      await invoke('force_close');
    } finally {
      closeDialogOpen = false;
    }
  });

  function onKeyDown(e) {
    if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 's') return;
    e.preventDefault();
    e.stopPropagation();
    save(e.shiftKey);
  }

  // ── Opening ──
  function takePendingFile() {
    var pending = null;
    try {
      pending = JSON.parse(localStorage.getItem(PENDING_KEY) || 'null');
      localStorage.removeItem(PENDING_KEY);
    } catch (e) {}
    return pending && pending.token ? pending : null;
  }

  function openErrorText(error) {
    if (error && error.code === 'not-utf8') return t('notUtf8Msg');
    if (errorText(error).indexOf(TOO_LARGE_ERROR) === 0) return t('tooLargeMsg');
    return t('openErrorMsg');
  }

  function showName(name) {
    nameLabel.textContent = name;
    nameLabel.title = name;
  }

  // A reload of this page reopens the open document through current_document.
  async function readFile() {
    var file = takePendingFile() || await invoke('current_document').catch(function() { return null; });
    if (!file) throw new Error('no file to open');
    var opened = await invoke('read_markdown', { token: file.token });
    var decoded = Text.decode(base64ToBytes(opened.data));
    return { name: opened.name, decoded: decoded };
  }

  async function start() {
    applyStrings();
    var framed = prepareFrame();
    var file;
    try {
      file = await readFile();
    } catch (error) {
      log('open failed: ' + errorText(error));
      await tauri.dialog.message(openErrorText(error), { title: t('openFailed'), kind: 'error' });
      await leaveToStart();
      return;
    }
    format = { bom: file.decoded.bom, eol: file.decoded.eol };
    source.value = file.decoded.text;
    savedText = source.value;
    showName(file.name);
    invoke('set_window_title', { name: file.name }).catch(function() {});
    await framed;
    render();
  }

  source.addEventListener('input', function() {
    reportModified(false);
    scheduleRender();
  });
  document.addEventListener('keydown', onKeyDown, true);
  editButton.addEventListener('click', function() { setEditing(source.hidden); });
  document.getElementById('md-save').addEventListener('click', function() { save(false); });
  document.getElementById('md-save-as').addEventListener('click', function() { save(true); });
  document.getElementById('md-close').addEventListener('click', closeDocument);

  start();
})();
