// The Markdown page: a rendered preview of a .md, and an editor beside it for
// the text: CodeMirror (vendor/codemirror.min.js), loaded the first time Edit
// is pressed, or the textarea if it does not load. Reached from the start screen, Open and Recents through
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
  // Read by the start screen to open a document in the editors (editor-patches.js).
  var OPEN_IN_EDITOR_KEY = 'eo-pending-open';
  var START_PAGE = '../index.html';
  var RENDER_DELAY_MS = 250;
  // While modified, the text is copied for crash recovery at most this often.
  var RECOVERY_DELAY_MS = 3000;
  var SAVE_FILTERS = [{ name: 'Markdown', extensions: ['md', 'markdown'] }];
  var EXPORT_FILTERS = [{ name: 'HTML', extensions: ['html', 'htm'] }];
  var PREVIEW_STYLESHEET = 'markdown-preview.css';
  var SAVE_DENIED_ERROR = 'save-denied';
  var TOO_LARGE_ERROR = 'markdown-too-large';
  // The file is open in another window (new_window.rs, OPEN_ELSEWHERE).
  var OPEN_ELSEWHERE_ERROR = 'open-elsewhere';
  var CODEMIRROR_SCRIPT = 'vendor/codemirror.min.js';
  // A new document's name until its first save, and what Save As offers.
  var NEW_NAME = 'Untitled.md';
  // A new document is written with LF and no BOM.
  var NEW_FORMAT = { bom: false, eol: '\n' };

  // ── Interface text. Same language choice as bridge.js (_detectLang). ──
  var STRINGS = {
    en: {
      close: 'Close', edit: 'Edit', preview: 'Preview', save: 'Save', saveAs: 'Save as…',
      source: 'Markdown text', previewTitle: 'Preview',
      unsavedChanges: 'Unsaved changes',
      unsavedDiscardClose: 'The current document has unsaved changes. Discard them and close?',
      openFailed: 'Could not open file',
      recoverFailedMsg: 'The recovered document could not be opened. It is still offered on the start screen.',
      openErrorMsg: 'The file could not be opened.',
      openElsewhereMsg: 'This file is already open in another window. Use that window to work on it.',
      tooLargeMsg: 'The file is larger than 10 MB and cannot be opened as text.',
      notUtf8Msg: 'The file is not UTF-8 text. Opening it here could change it when saved.',
      saveError: 'Save error',
      saveErrorMsg: 'The file could not be saved.',
      saveDeniedMsg: 'The file could not be saved.\n\nYou do not have permission to write to that location, or the file is read-only.',
      saveTooLargeMsg: 'The text is larger than 10 MB and cannot be saved.',
      remoteImageBlocked: 'Remote image not loaded',
      imageNotShown: 'Image not shown',
      documents: 'Documents', all: 'All',
      exportHtml: 'Export as HTML',
      exportError: 'Export error',
      exportErrorMsg: 'The HTML file could not be written.'
    },
    es: {
      close: 'Cerrar', edit: 'Editar', preview: 'Vista previa', save: 'Guardar', saveAs: 'Guardar como…',
      source: 'Texto Markdown', previewTitle: 'Vista previa',
      unsavedChanges: 'Cambios sin guardar',
      unsavedDiscardClose: 'El documento actual tiene cambios sin guardar. ¿Descartarlos y cerrar?',
      openFailed: 'No se pudo abrir el archivo',
      recoverFailedMsg: 'No se pudo abrir el documento recuperado. Se sigue ofreciendo en la pantalla de inicio.',
      openErrorMsg: 'No se pudo abrir el archivo.',
      openElsewhereMsg: 'Este archivo ya está abierto en otra ventana. Usa esa ventana para trabajar con él.',
      tooLargeMsg: 'El archivo ocupa más de 10 MB y no se puede abrir como texto.',
      notUtf8Msg: 'El archivo no es texto UTF-8. Abrirlo aquí podría cambiarlo al guardar.',
      saveError: 'Error al guardar',
      saveErrorMsg: 'No se pudo guardar el archivo.',
      saveDeniedMsg: 'No se pudo guardar el archivo.\n\nNo tienes permiso para escribir en esa ubicación o el archivo es de solo lectura.',
      saveTooLargeMsg: 'El texto ocupa más de 10 MB y no se puede guardar.',
      remoteImageBlocked: 'Imagen remota no cargada',
      imageNotShown: 'Imagen no mostrada',
      documents: 'Documentos', all: 'Todos',
      exportHtml: 'Exportar como HTML',
      exportError: 'Error al exportar',
      exportErrorMsg: 'No se pudo escribir el archivo HTML.'
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
  var editorHost = document.getElementById('md-editor');
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
  // last read or saved: the document is modified while the editor differs.
  // savedText is null for recovered text, which is modified until it is saved.
  var format = NEW_FORMAT;
  var savedText = '';
  // False for a new document until its first save, which goes through Save As.
  var hasFile = false;
  var reportedModified = false;
  var saving = false;

  // Set once CodeMirror has taken over from the textarea.
  var codeMirror = null;

  // Both editors give the text with LF line ends.
  function currentText() {
    if (codeMirror) return codeMirror.getText();
    return source.value;
  }

  function isModified() {
    return currentText() !== savedText;
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

  // The document's HTML after DOMPurify, images still as written. Shared by
  // the preview and Export as HTML.
  function sanitizedFragment(text) {
    return window.DOMPurify.sanitize(markdown.render(text), PURIFY_CONFIG);
  }

  function previewFragment(text) {
    var fragment = sanitizedFragment(text);
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
    frameDocument.body.replaceChildren(previewFragment(currentText()));
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

  // ── Editor ──
  // The script is added only when Edit is first pressed, so reading a file
  // never downloads or runs CodeMirror. Resolves to false if it fails.
  var codeMirrorLoading = null;

  function loadCodeMirror() {
    if (codeMirrorLoading) return codeMirrorLoading;
    codeMirrorLoading = new Promise(function(resolve) {
      var script = document.createElement('script');
      script.src = CODEMIRROR_SCRIPT;
      script.onload = function() { resolve(!!window.EoCodeMirror); };
      script.onerror = function() { resolve(false); };
      document.head.appendChild(script);
    });
    return codeMirrorLoading;
  }

  function onTextChanged() {
    reportModified(false);
    scheduleRender();
    scheduleRecoveryCopy();
  }

  // ── Crash recovery ──
  // While the document is modified, markdown_recovery_write keeps a copy of
  // the bytes Save would write in the app's recovery folder (recovery.rs).
  // Writes go one after another, and Save waits for the last one: a copy
  // written after a save would be offered for text that is already saved.
  // The backend drops the copy after a successful save and at exit; this page
  // drops it with recovery_end when the user discards the changes.
  var recoveryTimer = null;
  var recoveryWrite = Promise.resolve();
  var recoveryStopped = false;

  function scheduleRecoveryCopy() {
    if (recoveryTimer || recoveryStopped) return;
    recoveryTimer = setTimeout(writeRecoveryCopy, RECOVERY_DELAY_MS);
  }

  function writeRecoveryCopy() {
    recoveryTimer = null;
    if (recoveryStopped || !isModified()) return;
    if (saving) {
      scheduleRecoveryCopy();
      return;
    }
    var data = bytesToBase64(Text.encode(currentText(), format));
    recoveryWrite = recoveryWrite.then(function() {
      return invoke('markdown_recovery_write', { data: data });
    }).catch(function(error) {
      log('recovery copy failed: ' + errorText(error));
    });
  }

  // No copy after this; resolves once the last one is written.
  function stopRecoveryCopies() {
    recoveryStopped = true;
    clearTimeout(recoveryTimer);
    recoveryTimer = null;
    return recoveryWrite;
  }

  async function discardRecoveryCopy() {
    await stopRecoveryCopies();
    await invoke('recovery_end', { discard: true }).catch(function() {});
  }

  // CodeMirror starts from the textarea's text, which stays as it was. It
  // refuses to start in a webview without adoptedStyleSheets (see the bundle's
  // entry.mjs), and the textarea is used instead.
  function startCodeMirror() {
    try {
      codeMirror = window.EoCodeMirror.createEditor({
        host: editorHost,
        text: source.value,
        label: t('source'),
        onChange: onTextChanged
      });
      return true;
    } catch (error) {
      log('CodeMirror failed: ' + errorText(error));
      codeMirror = null;
      return false;
    }
  }

  async function prepareEditor() {
    if (codeMirror) return;
    if (!(await loadCodeMirror())) {
      log('CodeMirror not loaded, using the textarea');
      return;
    }
    startCodeMirror();
  }

  function focusEditor() {
    if (codeMirror) {
      codeMirror.focus();
      return;
    }
    source.focus();
  }

  // ── Edit, save, close ──
  var editing = false;

  async function setEditing(on) {
    editing = on;
    if (on) await prepareEditor();
    document.body.classList.toggle('editing', on);
    editorHost.hidden = !(on && codeMirror);
    source.hidden = !(on && !codeMirror);
    editButton.setAttribute('aria-pressed', on ? 'true' : 'false');
    editButton.dataset.i18n = on ? 'preview' : 'edit';
    editButton.textContent = t(editButton.dataset.i18n);
    if (on) focusEditor();
  }

  function saveErrorText(error) {
    var text = errorText(error);
    if (text.indexOf(SAVE_DENIED_ERROR) === 0) return t('saveDeniedMsg');
    if (text.indexOf(TOO_LARGE_ERROR) === 0) return t('saveTooLargeMsg');
    return t('saveErrorMsg');
  }

  // A document with no file yet offers its name (NEW_NAME for a new one).
  // The backend decides the folder: for a recovered document, where it was
  // saved before; otherwise as for a new document in the editors.
  async function pickSaveTarget() {
    return invoke('pick_save_target', {
      filters: SAVE_FILTERS,
      requestedExt: 'md',
      suggestedName: hasFile ? null : currentName
    });
  }

  // Save writes over the open file; Save As (asNew) where the user picks. A
  // new document's first Save is a Save As.
  async function save(asNew) {
    if (saving) return;
    saving = true;
    if (!hasFile) asNew = true;
    try {
      var target = asNew ? await pickSaveTarget() : null;
      if (asNew && !target) return;
      var text = currentText();
      var data = bytesToBase64(Text.encode(text, format));
      await recoveryWrite;
      var name = await invoke('save_markdown', { data: data, token: target ? target.token : null });
      savedText = text;
      hasFile = true;
      // Relative images now resolve against the new file's folder.
      if (target) {
        images.clear();
        render();
      }
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
    await discardRecoveryCopy();
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
      // The exit itself drops the copy (main.rs, RunEvent::Exit).
      await stopRecoveryCopies();
      await invoke('force_close');
    } finally {
      closeDialogOpen = false;
    }
  });

  // Another document: Markdown reloads this page, anything else opens in
  // the editors from the start screen (editor-patches.js, eo-pending-open).
  async function openAnother() {
    if (!(await confirmDiscard())) return;
    var file = await invoke('pick_document', { documents: t('documents'), all: t('all') });
    if (!file || !file.token) return;
    await discardRecoveryCopy();
    await invoke('set_document_modified', { modified: false }).catch(function() {});
    var entry = JSON.stringify({ token: file.token, name: file.name });
    if (Text.isMarkdownPath(file.name)) {
      localStorage.setItem(PENDING_KEY, entry);
      window.location.reload();
      return;
    }
    localStorage.setItem(OPEN_IN_EDITOR_KEY, entry);
    window.location.href = START_PAGE;
  }

  // Ctrl+S, Ctrl+Shift+S, Ctrl+O and Ctrl+W, wherever the focus is: this
  // listens on the document in the capture phase, before CodeMirror's keymap.
  // Ctrl+Q is the app menu's (app_quit.rs).
  var SHORTCUTS = {
    s: function(e) { save(e.shiftKey); },
    o: function() { openAnother(); },
    w: function() { closeDocument(); }
  };

  function onKeyDown(e) {
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    var action = SHORTCUTS[e.key.toLowerCase()];
    if (!action) return;
    if (e.key.toLowerCase() !== 's' && e.shiftKey) return;
    e.preventDefault();
    e.stopPropagation();
    action(e);
  }

  // ── Export as HTML ──
  // A file that stands alone: the sanitized HTML of the preview, its
  // stylesheet in a <style>, no script. Local images keep the reference
  // written in the document (relative to the .md); remote and refused ones
  // become the preview's placeholder.
  var exporting = false;

  function exportFragment(text) {
    var fragment = sanitizedFragment(text);
    var imgs = fragment.querySelectorAll('img');
    for (var i = 0; i < imgs.length; i++) {
      var found = Text.classifyImageSource(imgs[i].getAttribute('src'));
      if (found.kind !== 'local') imageFailed(imgs[i], found.kind);
    }
    return fragment;
  }

  // The preview's stylesheet as text, from the frame that already loaded it:
  // the page's CSP does not let it fetch the file.
  function previewStyles() {
    var sheets = frame.contentDocument.styleSheets;
    for (var i = 0; i < sheets.length; i++) {
      if (!sheets[i].href || !sheets[i].href.endsWith(PREVIEW_STYLESHEET)) continue;
      return Array.prototype.map.call(sheets[i].cssRules, function(rule) { return rule.cssText; }).join('\n');
    }
    throw new Error('the preview stylesheet is not loaded');
  }

  function baseName(name) {
    return name.replace(/\.(md|markdown)$/i, '');
  }

  function exportTitle(fragment) {
    var heading = fragment.querySelector('h1, h2, h3, h4, h5, h6');
    var text = heading ? heading.textContent.trim() : '';
    return text || baseName(currentName);
  }

  // The exported file's text. Built in a document of its own, which loads
  // and runs nothing; the CSP <meta> is one more stop for scripts.
  function exportHtmlText(text) {
    var fragment = exportFragment(text);
    var doc = document.implementation.createHTMLDocument(exportTitle(fragment));
    var charset = doc.createElement('meta');
    charset.setAttribute('charset', 'utf-8');
    doc.head.prepend(charset);
    var policy = doc.createElement('meta');
    policy.setAttribute('http-equiv', 'Content-Security-Policy');
    policy.setAttribute('content', "script-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'");
    charset.after(policy);
    var style = doc.createElement('style');
    style.textContent = previewStyles();
    doc.head.appendChild(style);
    doc.body.className = 'markdown-body';
    doc.body.appendChild(doc.importNode(fragment, true));
    return '<!DOCTYPE html>\n' + doc.documentElement.outerHTML + '\n';
  }

  async function exportHtml() {
    if (exporting) return;
    exporting = true;
    try {
      var target = await invoke('pick_save_target', {
        filters: EXPORT_FILTERS,
        requestedExt: 'html',
        suggestedName: baseName(currentName) + '.html'
      });
      if (!target) return;
      var html = new TextEncoder().encode(exportHtmlText(currentText()));
      await invoke('export_markdown_html', { data: bytesToBase64(html), token: target.token });
    } catch (error) {
      log('export failed: ' + errorText(error));
      await tauri.dialog.message(t('exportErrorMsg'), { title: t('exportError'), kind: 'error' });
    } finally {
      exporting = false;
    }
  }

  // ── Opening ──
  // { token, name } for a file, { new: true } for New Markdown, { recover: id }
  // for a document offered on the start screen, or null.
  function takePending() {
    var pending = null;
    try {
      pending = JSON.parse(localStorage.getItem(PENDING_KEY) || 'null');
      localStorage.removeItem(PENDING_KEY);
    } catch (e) {}
    if (pending && (pending.token || pending.new || pending.recover)) return pending;
    return null;
  }

  function isOpenElsewhere(error) {
    return errorText(error) === OPEN_ELSEWHERE_ERROR;
  }

  // The same warning bridge.js shows (_eoShowOpenElsewhere).
  function showOpenFailed(error) {
    if (isOpenElsewhere(error)) {
      return tauri.dialog.message(t('openElsewhereMsg'), { title: t('openFailed'), kind: 'warning' });
    }
    return tauri.dialog.message(openErrorText(error), { title: t('openFailed'), kind: 'error' });
  }

  function openErrorText(error) {
    if (error && error.code === 'not-utf8') return t('notUtf8Msg');
    if (errorText(error).indexOf(TOO_LARGE_ERROR) === 0) return t('tooLargeMsg');
    return t('openErrorMsg');
  }

  var currentName = '';

  function showName(name) {
    currentName = name;
    nameLabel.textContent = name;
    nameLabel.title = name;
  }

  // A reload of this page without a recovery copy reopens the open document
  // through current_document.
  async function readFile(pending) {
    var file = pending || await invoke('current_document').catch(function() { return null; });
    if (!file) throw new Error('no file to open');
    var opened = await invoke('read_markdown', { token: file.token });
    var decoded = Text.decode(base64ToBytes(opened.data));
    return { name: opened.name, decoded: decoded };
  }

  async function startNew(framed) {
    await invoke('new_markdown').catch(function(error) { log('new failed: ' + errorText(error)); });
    showName(NEW_NAME);
    invoke('set_window_title', { name: NEW_NAME }).catch(function() {});
    await framed;
    render();
    await setEditing(true);
  }

  // The folder stays where it was if this fails: the start screen offers it
  // again, and nothing here discards it.
  async function startRecovered(framed, id) {
    var recovered;
    var decoded;
    try {
      recovered = await invoke('markdown_recovery_load', { id: id });
      decoded = Text.decode(base64ToBytes(recovered.data));
    } catch (error) {
      log('recover failed: ' + errorText(error));
      await tauri.dialog.message(t('recoverFailedMsg'), { title: t('openFailed'), kind: 'error' });
      await leaveToStart();
      return;
    }
    format = { bom: decoded.bom, eol: decoded.eol };
    source.value = decoded.text;
    savedText = null;
    showName(recovered.name);
    invoke('set_window_title', { name: recovered.name }).catch(function() {});
    await reportModified(true);
    await framed;
    render();
    await setEditing(true);
  }

  // A reload (F5, devtools) while the document had a recovery copy: the
  // copy comes back marked modified and stays offered until saved or
  // discarded (markdown_recovery_reload). False when there is no copy.
  async function startReloaded(framed) {
    var reloaded = await invoke('markdown_recovery_reload').catch(function(error) {
      log('reload copy failed: ' + errorText(error));
      return null;
    });
    if (!reloaded) return false;
    var decoded;
    try {
      decoded = Text.decode(base64ToBytes(reloaded.data));
    } catch (error) {
      log('reload copy unreadable: ' + errorText(error));
      return false;
    }
    format = { bom: decoded.bom, eol: decoded.eol };
    hasFile = reloaded.hasFile;
    source.value = decoded.text;
    savedText = null;
    showName(reloaded.name);
    invoke('set_window_title', { name: reloaded.name }).catch(function() {});
    await reportModified(true);
    await framed;
    render();
    await setEditing(true);
    return true;
  }

  async function start() {
    applyStrings();
    var framed = prepareFrame();
    var pending = takePending();
    if (pending && pending.new) {
      await startNew(framed);
      return;
    }
    if (pending && pending.recover) {
      await startRecovered(framed, pending.recover);
      return;
    }
    if (!pending && await startReloaded(framed)) return;
    var file;
    try {
      file = await readFile(pending);
    } catch (error) {
      log('open failed: ' + errorText(error));
      await showOpenFailed(error);
      await leaveToStart();
      return;
    }
    format = { bom: file.decoded.bom, eol: file.decoded.eol };
    hasFile = true;
    source.value = file.decoded.text;
    savedText = source.value;
    showName(file.name);
    invoke('set_window_title', { name: file.name }).catch(function() {});
    await framed;
    render();
  }

  source.addEventListener('input', onTextChanged);
  document.addEventListener('keydown', onKeyDown, true);
  editButton.addEventListener('click', function() { setEditing(!editing); });
  document.getElementById('md-save').addEventListener('click', function() { save(false); });
  document.getElementById('md-save-as').addEventListener('click', function() { save(true); });
  document.getElementById('md-close').addEventListener('click', closeDocument);
  document.getElementById('md-export').addEventListener('click', exportHtml);

  start();
})();
