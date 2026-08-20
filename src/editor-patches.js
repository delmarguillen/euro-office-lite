    const startScreen = document.getElementById('start-screen');


    function _installFrameDiagnostics(win) {
      try {
        if (!win || win._eoDiagnosticsInstalled) return;
        win._eoDiagnosticsInstalled = true;

        win.addEventListener('error', function(event) {
          var error = event.error || {};
          var source = window._eoSafeSource ?
            window._eoSafeSource(event.filename || '') : (event.filename || '');
          _log('[IFRAME-ERROR]', {
            name: error.name,
            message: event.message || error.message,
            stack: error.stack
          }, 'source=' + source + ':' + (event.lineno || 0) + ':' + (event.colno || 0));
        }, true);

        win.addEventListener('unhandledrejection', function(event) {
          _log('[IFRAME-REJECT]', event.reason || 'unknown');
        });

        var frameSource = '';
        try {
          frameSource = window._eoSafeSource ?
            window._eoSafeSource(win.location.href) : win.location.pathname;
        } catch(e) {}
        // The frame element's id tells SDK-created helper iframes (e.g. sdkjs's
        // asc_pasteFrame, whose document.write inherits the editor URL) apart
        // from the real editor iframe, which shares the same source path.
        var frameId = '';
        try {
          if (win.frameElement) {
            frameId = ' id=' + (win.frameElement.id || win.frameElement.name || '(anonymous)');
          }
        } catch(e) {}
        _log('[EO] iframe diagnostics installed: source=' + frameSource + frameId);
      } catch(e) {
        _log('[EO] iframe diagnostics install failed: ' + (e.message || e));
      }
    }

    function injectBridgeDeep(win) {
      try {
        var hasADE = false;
        try { hasADE = !!win.AscDesktopEditor; } catch(e) {}

        _installFrameDiagnostics(win);

        if (win && !hasADE) {
          try {
            win.NATIVE_EDITOR_ENJINE = undefined;
            win.IS_NATIVE_EDITOR = undefined;
            delete win['native'];
          } catch(e) {}

          var ade = {};
          for (var k in window.AscDesktopEditor) {
            ade[k] = window.AscDesktopEditor[k];
          }
          win.AscDesktopEditor = ade;
          win.RendererProcessVariable = window.RendererProcessVariable;
          win._eoLog = window._eoLog;

          // Patch fetch to merge incomplete locale files with en.json fallback
          if (!win._eoLocalePatchApplied) {
            win._eoLocalePatchApplied = true;
            var _origWinFetch = win.fetch;
            win.fetch = function(url, options) {
              if (typeof url === 'string' && /locale\/[a-z]([a-z-]*)?\.json$/i.test(url)) {
                var langMatch = url.match(/locale\/([a-z]([a-z-]*)?)\.json$/i);
                if (langMatch && langMatch[1] !== 'en') {
                  var enUrl = url.replace(/locale\/[a-z]([a-z-]*)?\.json$/i, 'locale/en.json');
                  return Promise.all([
                    _origWinFetch.call(win, enUrl, options).then(function(r) { return r.json(); }),
                    _origWinFetch.call(win, url, options).then(function(r) { return r.json(); })
                  ]).then(function(results) {
                    var merged = {};
                    var enKeys = Object.keys(results[0]);
                    var langKeys = Object.keys(results[1]);
                    for (var i = 0; i < enKeys.length; i++) merged[enKeys[i]] = results[0][enKeys[i]];
                    for (var i = 0; i < langKeys.length; i++) merged[langKeys[i]] = results[1][langKeys[i]];
                    return new Response(JSON.stringify(merged), {
                      status: 200,
                      headers: { 'Content-Type': 'application/json' }
                    });
                  });
                }
              }
              return _origWinFetch.call(win, url, options);
            };
          }

          // Re-expose the "Download As" File-menu entry, the only visible way to
          // pick an output format when saving: the native dialog shows no format
          // selector on Linux and macOS, and typing the extension by hand was the
          // documented workaround (issue #34). The entry is hidden by the
          // desktop-offline mode patch below, so CSS with !important is used to
          // beat the inline style jQuery sets, whenever the File menu renders.
          // Its format panel is trimmed to the formats the bridge maps to an
          // extension (_eoSaveAsFilters in bridge.js): anything else silently
          // saves in the document's current format instead of the chosen one.
          if (!win._eoSaveAsPanelStyled) {
            win._eoSaveAsPanelStyled = true;
            var supportedFormats = [
              65, 67, 68, 69,      // word: docx, odt, rtf, txt
              257, 259, 260,       // cell: xlsx, ods, csv
              129, 131,            // slide: pptx, odp
              513                  // pdf (all editors)
            ];
            var formatFilter = '';
            var itemFilter = '';
            for (var fi = 0; fi < supportedFormats.length; fi++) {
              formatFilter += ':not([format="' + supportedFormats[fi] + '"])';
              itemFilter += ':not(:has(.btn-doc-format[format="' + supportedFormats[fi] + '"]))';
            }
            var saveAsStyleInterval = setInterval(function() {
              try {
                var head = win.document && win.document.head;
                if (!head) return;
                var style = win.document.createElement('style');
                style.textContent =
                  '#fm-btn-download { display: list-item !important; }\n' +
                  // The tile container carries no format attribute, so hiding
                  // only the inner button would leave empty cells in the grid.
                  // The child rule stays as the fallback if :has() ever fails.
                  '#panel-saveas .format-item' + itemFilter + ' { display: none !important; }\n' +
                  '#panel-saveas .btn-doc-format' + formatFilter + ' { display: none !important; }';
                head.appendChild(style);
                clearInterval(saveAsStyleInterval);
                win._eoLog('[EO] Download As panel style installed');
              } catch(e) {
                clearInterval(saveAsStyleInterval);
              }
            }, 50);
            setTimeout(function() { clearInterval(saveAsStyleInterval); }, 30000);
          }

          // "Download As" is web-apps wording for an online flow. The entry saves
          // to disk here, so it is renamed to "Export". Two things make a single
          // pass insufficient: this runs before the frame has navigated to the
          // editor document (so the timer waits for the live document rather
          // than patching the one being replaced), and web-apps rewrites the
          // caption every time it renders the File menu (so the observer stays
          // connected for the life of the document instead of firing once).
          if (!win._eoExportCaptionPatched && win.MutationObserver) {
            win._eoExportCaptionPatched = true;
            var exportLabel = window._t ? window._t('export') : 'Export';
            var renameExportEntry = function() {
              var entry = win.document.getElementById('fm-btn-download');
              var link = entry && entry.querySelector('a');
              if (!link) return;
              // The anchor is an icon span followed by a bare text node;
              // replacing the text node keeps the icon in place.
              for (var ni = link.childNodes.length - 1; ni >= 0; ni--) {
                var node = link.childNodes[ni];
                if (node.nodeType === 3 && node.nodeValue.trim()) {
                  // The guard also stops the observer from waking itself up.
                  if (node.nodeValue !== exportLabel) node.nodeValue = exportLabel;
                  return;
                }
              }
            };
            var renameExportHeader = function() {
              var panel = win.document.getElementById('panel-saveas');
              var header = panel && panel.querySelector('.header');
              if (header && header.textContent !== exportLabel) {
                header.textContent = exportLabel;
              }
            };
            var exportCaptionInterval = setInterval(function() {
              try {
                // about:blank has a body too, so the SDK being up is what tells
                // the live editor document apart from the one it replaces.
                if (!win.Asc || !win.Asc.editor || !win.document || !win.document.body) return;
                clearInterval(exportCaptionInterval);
                var exportObserver = new win.MutationObserver(function() {
                  renameExportEntry();
                  renameExportHeader();
                });
                exportObserver.observe(win.document.documentElement,
                  { childList: true, subtree: true, characterData: true });
                renameExportEntry();
                renameExportHeader();
                win._eoLog('[EO] Export caption watcher installed');
              } catch(e) {
                clearInterval(exportCaptionInterval);
              }
            }, 50);
            setTimeout(function() { clearInterval(exportCaptionInterval); }, 30000);
          }

          // "Remove note separator" (#42): a File menu entry of the Word editor
          // only, since the surgery is on word/footnotes.xml of a .docx. Same
          // shape as the Export caption watcher above and for the same two
          // reasons: this runs before the frame has navigated to the editor
          // document, and web-apps re-renders the File menu, which drops
          // anything it did not put there itself.
          if (!win._eoNoteSeparatorEntryInstalled && win.MutationObserver &&
              window.AscDesktopEditor._currentDocType === 'word') {
            win._eoNoteSeparatorEntryInstalled = true;
            var noteSepId = 'fm-btn-eo-note-separator';
            var noteSepLabel = window._t ? window._t('removeNoteSeparator') : 'Note lines';
            var addNoteSeparatorEntry = function() {
              var doc = win.document;
              // Also what stops the observer from waking itself up on the entry
              // it just inserted.
              if (doc.getElementById(noteSepId)) return;
              // Cloned from an entry web-apps rendered itself, so the item picks
              // up whatever classes and inner markup this build gives them
              // rather than a copy that ages badly.
              // Save As ("fm-btn-save-desktop") first: it is the one File menu
              // entry web-apps renders with no icon, so its markup is already
              // the shape this entry needs. Print/Save are the fallback if the
              // build did not render it.
              var template = doc.getElementById('fm-btn-save-desktop') ||
                doc.getElementById('fm-btn-print') ||
                doc.getElementById('fm-btn-save');
              if (!template || !template.querySelector('a')) return;

              var entry = template.cloneNode(true);
              entry.id = noteSepId;
              // Layout names, hints and the scaling hook belong to the entry
              // this was cloned from.
              entry.removeAttribute('data-layout-name');
              entry.removeAttribute('ratio');
              entry.style.display = 'list-item';
              var link = entry.querySelector('a');
              link.removeAttribute('data-hint-title');
              link.removeAttribute('id');
              // The label is indented the way the iconless entries are: the
              // stylesheet gives ".fm-btn > a[data-no-icon]" the same left
              // padding that an icon box plus its margin occupies, and that is
              // exactly how web-apps aligns Save As. Keeping a stripped icon
              // node instead loses the "menu-item-icon" class that carries the
              // width and the 8px margin, so the label started flush against
              // the padding while every neighbour started 28px in - which is
              // what made this entry read as centred next to the others.
              var icons = link.querySelectorAll('.menu-item-icon');
              for (var ii = 0; ii < icons.length; ii++) {
                icons[ii].parentNode.removeChild(icons[ii]);
              }
              link.setAttribute('data-no-icon', '');
              for (var ni = link.childNodes.length - 1; ni >= 0; ni--) {
                if (link.childNodes[ni].nodeType === 3) link.removeChild(link.childNodes[ni]);
              }
              link.appendChild(doc.createTextNode(noteSepLabel));

              // The clone carries no handler of its own (cloneNode copies markup,
              // not listeners) and web-apps has no MenuItem registered for it, so
              // the click is stopped here rather than left to a delegated handler
              // that would look this entry up and find nothing.
              entry.addEventListener('click', function(e) {
                e.preventDefault();
                e.stopPropagation();
                if (window._eoRemoveNoteSeparator) window._eoRemoveNoteSeparator();
              }, true);

              template.parentNode.insertBefore(entry, template.nextSibling);
            };
            var noteSepInterval = setInterval(function() {
              try {
                // about:blank has a body too, so the SDK being up is what tells
                // the live editor document apart from the one it replaces.
                if (!win.Asc || !win.Asc.editor || !win.document || !win.document.body) return;
                clearInterval(noteSepInterval);
                var noteSepObserver = new win.MutationObserver(function() {
                  addNoteSeparatorEntry();
                });
                noteSepObserver.observe(win.document.documentElement,
                  { childList: true, subtree: true });
                addNoteSeparatorEntry();
                win._eoLog('[EO] Note separator entry watcher installed');
              } catch(e) {
                clearInterval(noteSepInterval);
              }
            }, 50);
            setTimeout(function() { clearInterval(noteSepInterval); }, 30000);
          }

          window._patchButtonHint(win);

          var saveAsPatched = false;
          var savePatchInterval = setInterval(function() {
            try {
              if (!saveAsPatched && win.Asc && win.Asc.editor) {
                var api = win.Asc.editor;

                // Euro Office is a local desktop application even though Tauri
                // serves its assets through a custom http(s) protocol. Production
                // Web Apps can otherwise snapshot the base SDK's protocol check
                // before the desktop override and expose the online "Download as"
                // flow. Keep the API authoritative for all later mode checks.
                var reportedOffline = 'unavailable';
                try { reportedOffline = String(api.asc_isOffline()); } catch(e) {}
                api.asc_isOffline = api['asc_isOffline'] = function() { return true; };
                win._eoLog('[EO] desktop-offline API override: reported=' + reportedOffline);

                var origDownloadAs = api.asc_DownloadAs;
                api.asc_DownloadAs = api['asc_DownloadAs'] = function(options) {
                  if (options && options.isNaturalDownload) {
                    return origDownloadAs.call(api, options);
                  }

                  // The TXT/CSV export flows put their encoding settings in
                  // options.advancedOptions, but the desktop save path in sdkjs
                  // assumes that field is always the print options object and
                  // calls asc_getNativeOptions() on it. The resulting TypeError
                  // fires after sync_StartAction, so isLongAction() stays true
                  // and every later save (Ctrl+S included) becomes a silent
                  // no-op until restart. Drop anything that is not the print
                  // object; the bridge ignores those settings anyway.
                  if (options && options.advancedOptions &&
                      typeof options.advancedOptions.asc_getNativeOptions !== 'function') {
                    options.advancedOptions = undefined;
                  }

                  var docType = win.AscDesktopEditor._currentDocType || '';

                  // Cell and Slide gate asc_Save behind canSave. A Save As/export must
                  // still work for an unmodified document, so enter the desktop save
                  // bridge directly, preserving the PDF print options.
                  if ((docType === 'cell' || docType === 'slide') &&
                      typeof win.DesktopOfflineAppDocumentStartSave === 'function') {
                    if (docType === 'cell' && typeof api.asc_closeCellEditor === 'function') {
                      api.asc_closeCellEditor();
                    }
                    win.DesktopOfflineAppDocumentStartSave(true, undefined, undefined, undefined, options);
                    return;
                  }

                  api.asc_Save(false, true, undefined, options);
                };

                if (typeof win.DesktopOfflineAppDocumentStartSave !== 'function') {
                  win.DesktopOfflineAppDocumentStartSave = win['DesktopOfflineAppDocumentStartSave'] = function(isSaveAs) {
                    var _param = isSaveAs ? 'saveas=true;' : '';
                    win.AscDesktopEditor.LocalFileSave(_param, api.currentPassword || '', undefined, 0,
                      JSON.stringify(api['getAdditionalSaveParams'] ? api['getAdditionalSaveParams']() : {}));
                  };
                }

                win.document.addEventListener('keydown', function(e) {
                  if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === 'S') {
                    e.preventDefault();
                    e.stopPropagation();
                    api.asc_DownloadAs();
                  }
                  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key === 'o') {
                    e.preventDefault();
                    e.stopPropagation();
                    window.AscDesktopEditor.LocalFileOpen();
                  }
                  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key === 'p') {
                    e.preventDefault();
                    e.stopPropagation();
                    window.AscDesktopEditor.Print();
                  }
                }, true);

                saveAsPatched = true;
                clearInterval(savePatchInterval);
              }
            } catch(e) {
              clearInterval(savePatchInterval);
            }
          }, 50);
          setTimeout(function() { clearInterval(savePatchInterval); }, 30000);

          // The production Web Apps bundle computes File-menu visibility once
          // while it initializes. Reassert desktop/offline mode on the controller
          // state as well as the SDK API, then ask the menu to reapply that state.
          // This keeps the local "Save as" and PDF export actions gated by
          // canDownload. It also hides the "Download as" entry, which the style
          // installed above brings back on purpose as the format picker.
          var desktopModePatched = false;
          var desktopModeLastError = '';
          var desktopModeInterval = setInterval(function() {
            if (desktopModePatched) return;
            try {
              var editorApps = [
                { name: 'word', app: win.DE },
                { name: 'cell', app: win.SSE },
                { name: 'slide', app: win.PE }
              ];
              for (var ei = 0; ei < editorApps.length; ei++) {
                var editorApp = editorApps[ei];
                if (!editorApp.app || !editorApp.app.getController) continue;

                var mainController = editorApp.app.getController('Main');
                var leftMenuController = editorApp.app.getController('LeftMenu');
                if (!mainController || !mainController.appOptions ||
                    !leftMenuController || !leftMenuController.mode) continue;

                var previousDesktop = mainController.appOptions.isDesktopApp;
                var previousOffline = mainController.appOptions.isOffline;
                mainController.appOptions.isDesktopApp = true;
                mainController.appOptions.isOffline = true;
                leftMenuController.mode.isDesktopApp = true;
                leftMenuController.mode.isOffline = true;

                if (typeof leftMenuController.setMode === 'function') {
                  leftMenuController.setMode(leftMenuController.mode);
                }

                win._eoLog('[EO] desktop-offline UI mode: editor=' + editorApp.name +
                  ' previousDesktop=' + previousDesktop +
                  ' previousOffline=' + previousOffline +
                  ' canDownload=' + leftMenuController.mode.canDownload);
                desktopModePatched = true;
                clearInterval(desktopModeInterval);
                break;
              }
            } catch(e) {
              var modeError = String(e.message || e);
              if (modeError !== desktopModeLastError) {
                desktopModeLastError = modeError;
                win._eoLog('[EO] desktop-offline UI mode retry: ' + modeError);
              }
            }
          }, 50);
          setTimeout(function() {
            clearInterval(desktopModeInterval);
            if (!desktopModePatched) {
              win._eoLog('[EO] desktop-offline UI mode was not applied before timeout');
            }
          }, 30000);

          // Supply printer list to print preview panel
          var printSetupDone = false;
          var printSetupInterval = setInterval(function() {
            if (printSetupDone) return;
            try {
              var apps = [win.DE, win.SSE, win.PE];
              for (var ai = 0; ai < apps.length; ai++) {
                var app = apps[ai];
                if (!app || !app.getController) continue;
                try {
                  var lm = app.getController('LeftMenu');
                  if (lm && lm.mode) {

                    if (lm.leftMenu && lm.leftMenu.showMenu) {
                      var origShowMenu = lm.leftMenu.showMenu;
                      lm.leftMenu.showMenu = function(tag) {
                        if (tag === 'file:printpreview') {
                          var isWindows = navigator.platform && navigator.platform.indexOf('Win') !== -1;
                          if (isWindows) {
                            window.__TAURI__.core.invoke('plugin:printer|get_printers').then(function(rawJson) {
                              try {
                                var pluginPrinters = typeof rawJson === 'string' ? JSON.parse(rawJson) : rawJson;
                                if (!Array.isArray(pluginPrinters)) pluginPrinters = [pluginPrinters];
                                var defaultPrinter = null;
                                var printers = [];
                                for (var pi = 0; pi < pluginPrinters.length; pi++) {
                                  var pp = pluginPrinters[pi];
                                  var pName = pp.Name || pp.name || pp.id || ('Printer ' + pi);
                                  printers.push({
                                    name: pName,
                                    color_supported: pp.ColorSupported || pp.color_supported || false,
                                    duplex_supported: pp.DuplexSupported || pp.duplex_supported || false
                                  });
                                  if (pp.IsDefault || pp.is_default) defaultPrinter = pName;
                                }
                                if (!defaultPrinter && printers.length > 0) defaultPrinter = printers[0].name;
                                var configJson = JSON.stringify({ printers: printers, current_printer: defaultPrinter });
                                window._eoLog('[PRINT] printers=' + printers.length + ' default=' + (defaultPrinter || 'none'));
                                if (win.on_native_message) {
                                  win.on_native_message('printer:config', configJson);
                                }
                              } catch(pe) {
                                window._eoLog('[EO] Print: error parsing plugin printers: ' + pe);
                              }
                            }).catch(function(err) {
                              window._eoLog('[EO] Print: plugin get_printers error: ' + err);
                            });
                          } else {
                            window._eoLog('[EO] Print: native printer plugin unavailable; using PDF viewer fallback');
                          }
                        }
                        return origShowMenu.apply(this, arguments);
                      };
                    }

                    printSetupDone = true;
                    clearInterval(printSetupInterval);
                    return;
                  }
                } catch(e2) {}
              }
            } catch(e) {}
          }, 200);
          setTimeout(function() { clearInterval(printSetupInterval); }, 30000);

          var fontPatched = false;
          var fontsPreloaded = false;
          var fontPatchInterval = setInterval(function() {
            try {
              _applySystemFontsToFrame(win);
              _installFontThumbnailFallback(win);
              if (win.AscFonts && win.AscFonts.CFontFileLoader) {
                if (!fontPatched) {
                  var currentFn = String(win.AscFonts.CFontFileLoader.prototype.LoadFontAsync);
                  if (currentFn.indexOf('/fonts/') === -1 || currentFn.indexOf('ascdesktop://') !== -1) {
                    _patchLoadFontAsync(win);
                  }
                  fontPatched = true;
                }
                if (!fontsPreloaded && win.AscFonts.g_font_files && win.AscFonts.g_font_files.length > 0) {
                  // Preloading the bundled 16 files is cheap; preloading hundreds
                  // of system fonts synchronously would freeze startup and waste RAM.
                  if (!win._eoSystemFontsInjected) _preloadAllFonts(win);
                  fontsPreloaded = true;
                }
                if (win.AscFonts.g_fontApplication && win.AscFonts.g_fontApplication.g_fontSelections) {
                  _injectFontSelections(win);
                }
              }
            } catch(e) {
              clearInterval(fontPatchInterval);
            }
          }, 5);
          setTimeout(function() { clearInterval(fontPatchInterval); }, 30000);

          // ── TABS FIX: strip Tauri dev-server HTML wrapper from templates ──
          // Tauri dev server wraps served files in <html><head><script>...
          // This corrupts RequireJS text! plugin loads (.template files).
          // Fix: patch _.template to strip the wrapper before compiling.
          var _tabsTemplatePatched = false;
          var tabsTemplateInterval = setInterval(function() {
            if (_tabsTemplatePatched) return;
            try {
              if (!win._ || !win._.template) return;

              var origTemplate = win._.template;
              var headEndTag = '<' + '/head>';
              var htmlEndTag = '<' + '/html>';
              var bodyOpenRe = new RegExp('^\\s*<body[^>]*>\\s*', 'i');
              var bodyCloseRe = new RegExp('\\s*<' + '/body>\\s*$', 'i');

              win._.template = function(text, settings, oldSettings) {
                if (typeof text === 'string' && text.indexOf(headEndTag) !== -1 && text.substring(0, 5) === '<html') {
                  var headEnd = text.indexOf(headEndTag);
                  if (headEnd !== -1) {
                    var stripped = text.substring(headEnd + headEndTag.length);
                    var htmlEnd = stripped.lastIndexOf(htmlEndTag);
                    if (htmlEnd !== -1) {
                      stripped = stripped.substring(0, htmlEnd);
                    }
                    stripped = stripped.replace(bodyOpenRe, '').replace(bodyCloseRe, '');
                    if (stripped.indexOf('&lt;%') !== -1 || stripped.indexOf('%&gt;') !== -1) {
                      stripped = stripped
                        .replace(/&lt;%/g, '<%')
                        .replace(/%&gt;/g, '%>');
                    }
                    text = stripped;
                  }
                }
                return origTemplate.call(this, text, settings, oldSettings);
              };

              _tabsTemplatePatched = true;
              clearInterval(tabsTemplateInterval);
            } catch(e) {}
          }, 5);
          setTimeout(function() { clearInterval(tabsTemplateInterval); }, 30000);
          // ── END TABS FIX ──────────────────────────────────────────────
        }
      } catch(e) {
        _log('[INJECT] ERROR:', e.message);
      }
    }

    // Patch createElement to intercept iframe creation and inject bridge early
    var origCreateElement = document.createElement.bind(document);
    document.createElement = function(tag) {
      var el = origCreateElement(tag);
      if (tag.toLowerCase() === 'iframe') {
        var origSrcDescriptor = Object.getOwnPropertyDescriptor(HTMLIFrameElement.prototype, 'src');
        var origSrc = origSrcDescriptor.set;
        var patched = false;

        el.addEventListener('load', function() {
          injectBridgeDeep(el.contentWindow);
          // Also watch for nested iframes
          if (el.contentDocument) {
            var innerObserver = new MutationObserver(function(mutations) {
              mutations.forEach(function(m) {
                m.addedNodes.forEach(function(node) {
                  if (node.tagName === 'IFRAME') {
                    node.addEventListener('load', function() {
                      injectBridgeDeep(node.contentWindow);
                    });
                  }
                });
              });
            });
            innerObserver.observe(el.contentDocument, { childList: true, subtree: true });
          }
        });

        // Inject as soon as contentWindow becomes available
        var checkInterval = setInterval(function() {
          try {
            if (el.contentWindow) {
              injectBridgeDeep(el.contentWindow);
            }
            if (el.contentDocument && el.contentDocument.readyState === 'complete') {
              clearInterval(checkInterval);
            }
          } catch(e) {
            clearInterval(checkInterval);
          }
        }, 10);

        setTimeout(function() { clearInterval(checkInterval); }, 30000);
      }
      return el;
    };

    window._openEditor = openEditor;
    async function openEditor(docType) {
      // Font generation finishes in Tauri setup. Fetch its result before the
      // editor iframe is created so injection does not race LoadDocumentFonts.
      await _prepareSystemFonts();
      if (!window._pendingFileData) {
        try {
          var b64 = await window.__TAURI__.core.invoke('create_new', { docType: docType });
          var defaultName = docType === 'cell' ? _t('newSpreadsheet') : docType === 'slide' ? _t('newPresentation') : _t('newDocument');
          window._pendingFileData = { data: b64, path: null, name: defaultName };
        } catch(e) {
          window._eoLog('[EO] create_new failed: ' + (e.message || e));
        }
      }
      window.AscDesktopEditor._currentDocType = docType;
      startScreen.classList.add('hidden');
      document.getElementById('placeholder').classList.add('active');

      new DocsAPI.DocEditor('placeholder', {
        type: 'desktop',
        width: '100%',
        height: '100%',
        documentType: docType,
        document: {
          title: (window._pendingFileData && window._pendingFileData.name) || _t('newDocument'),
          url: '_offline_',
          fileType: docType === 'word' ? 'docx' : docType === 'cell' ? 'xlsx' : 'pptx',
          key: 'local-' + Date.now(),
          permissions: {
            edit: true,
            download: true,
            print: true
          }
        },
        editorConfig: {
          mode: 'edit',
          lang: window._eoCurrentLang || 'en',
          targetApp: 'desktop',
          user: {
            id: 'local-user',
            name: _t('user')
          },
          customization: {
            about: false,
            feedback: false
          }
        },
        events: {
          onAppReady: function() {
            var allFrames = document.querySelectorAll('iframe');
            _log('[EO] App ready: docType=' + docType + ' frames=' + allFrames.length);
            for (var fi = 0; fi < allFrames.length; fi++) {
              try {
                var fw = allFrames[fi].contentWindow;
                if (!fw || !fw.AscFonts || !fw.AscFonts.CFontFileLoader) continue;
                _patchLoadFontAsync(fw);
                if (!fw._eoSystemFontsInjected) _preloadAllFonts(fw);
                _injectFontSelections(fw);
              } catch(e) {}
            }
            _loadSystemFonts();
          },
          onDocumentReady: function() {
            _log('[EO] Document ready: docType=' + docType);
          },
          onError: function(event) {
            _log('[EO] Editor error event: docType=' + docType,
              event && event.data ? event.data : { message: 'missing event data' });
          },
          onWarning: function(event) {
            _log('[EO] Editor warning event: docType=' + docType,
              event && event.data ? event.data : { message: 'missing event data' });
          }
        }
      });
    }

    document.addEventListener('keydown', function(e) {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === 'S') {
        e.preventDefault();
        e.stopPropagation();
        var ref = _findEditorWindow(window);
        if (ref && ref.Asc && ref.Asc.editor) {
          ref.Asc.editor.asc_DownloadAs();
        } else {
          window.AscDesktopEditor.LocalFileSave('saveas=true;', '', undefined, 0, '{}');
        }
      }
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key === 'o') {
        e.preventDefault();
        e.stopPropagation();
        window.AscDesktopEditor.LocalFileOpen();
      }
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key === 'p') {
        e.preventDefault();
        e.stopPropagation();
        window.AscDesktopEditor.Print();
      }
    }, true);

    document.querySelectorAll('.btn[data-type]').forEach(function(btn) {
      btn.addEventListener('click', function() {
        openEditor(btn.dataset.type);
      });
    });

    document.querySelector('.btn[data-open]').addEventListener('click', async function() {
      if (!window.AscDesktopEditor) return;

      var dialog = window.__TAURI__.dialog;
      var path = await dialog.open({
        filters: [
          { name: _t('documents'), extensions: ['docx', 'xlsx', 'pptx', 'odt', 'ods', 'odp', 'rtf', 'txt', 'csv', 'pdf'] },
          { name: _t('all'), extensions: ['*'] }
        ]
      });
      if (!path) return;
      _openPath(path);
    });

    // Every opening goes through open_file, which is where the Rust side records
    // the recent files entry.
    async function _openPath(path, silent) {
      try {
        var b64data = await window.__TAURI__.core.invoke('open_file', { path: path });
        var fileName = path.replace(/\\/g, '/').split('/').pop();
        window._pendingFileData = { data: b64data, path: path, name: fileName };
        openEditor(window._eoDocTypeForPath(path));
        return true;
      } catch(e) {
        // Without a dialog the app just drops back to the start screen and the
        // reason only reaches the log: the user cannot tell a damaged file from
        // a broken app (Issue #38).
        window._eoLog('[EO] Error opening file:', e);
        if (!silent) await window._eoShowOpenError();
        return false;
      }
    }

    (function checkPendingOpenPath() {
      var pendingPath = localStorage.getItem('eo-pending-open-path');
      if (!pendingPath) return;
      localStorage.removeItem('eo-pending-open-path');

      startScreen.classList.add('hidden');

      var docType = window._eoDocTypeForPath(pendingPath);

      window.__TAURI__.core.invoke('open_file', { path: pendingPath }).then(function(b64data) {
        var fileName = pendingPath.replace(/\\/g, '/').split('/').pop();
        window._pendingFileData = { data: b64data, path: pendingPath, name: fileName };
        openEditor(docType);
      }).catch(function(e) {
        window._eoLog('[EO] Error reopening file:', e);
        startScreen.classList.remove('hidden');
        window._eoShowOpenError().catch(function(){});
      });
    })();

    // ── Recent files (Issue #13) ──
    function _formatRecentDate(seconds) {
      try {
        return new Date(seconds * 1000).toLocaleDateString(window._eoCurrentLang || 'en',
          { year: 'numeric', month: 'short', day: 'numeric' });
      } catch(e) {
        return '';
      }
    }

    async function _openRecentFile(path) {
      // Silent: this path keeps its own message for the dominant case, a file
      // that disappeared between rendering the list and the click.
      if (await _openPath(path, true)) return;
      await window.__TAURI__.dialog.message(_t('openFailedMsg'),
        { title: _t('openFailed'), kind: 'error' });
      _refreshRecentFiles();
    }

    function _renderRecentFiles(state) {
      var list = document.getElementById('recent-list');
      var files = state.enabled ? state.files : [];

      document.getElementById('recent-remember').checked = state.enabled;
      // With the switch off the whole block collapses to the switch itself: a
      // heading with nothing under it reads as leftover UI.
      document.getElementById('recent-head').hidden = !state.enabled;
      document.getElementById('recent-empty').hidden = !state.enabled || files.length > 0;
      document.getElementById('recent-clear').hidden = state.files.length === 0;

      list.textContent = '';
      files.forEach(function(file) {
        var row = document.createElement('button');
        row.type = 'button';
        row.className = 'recent-item';
        row.title = file.path;
        // File names are user data: build the row with textContent so a name
        // containing markup can never be parsed as HTML.
        var fields = [
          { className: 'recent-name', text: file.name },
          { className: 'recent-path', text: file.path },
          { className: 'recent-date', text: _formatRecentDate(file.openedAt) }
        ];
        fields.forEach(function(field) {
          var span = document.createElement('span');
          span.className = field.className;
          span.textContent = field.text;
          row.appendChild(span);
        });
        row.addEventListener('click', function() { _openRecentFile(file.path); });

        var item = document.createElement('li');
        item.appendChild(row);
        list.appendChild(item);
      });
    }

    async function _refreshRecentFiles() {
      try {
        _renderRecentFiles(await window.__TAURI__.core.invoke('recent_files_state'));
      } catch(e) {
        _log('[EO] Recent files unavailable: ' + (e.message || e));
      }
    }

    document.getElementById('recent-clear').addEventListener('click', function() {
      window.__TAURI__.core.invoke('clear_recent_files')
        .then(_refreshRecentFiles)
        .catch(function(e) { _log('[EO] Recent files clear failed: ' + (e.message || e)); });
    });

    document.getElementById('recent-remember').addEventListener('change', function() {
      window.__TAURI__.core.invoke('set_recent_files_enabled', { enabled: this.checked })
        .then(_refreshRecentFiles)
        .catch(function(e) { _log('[EO] Recent files toggle failed: ' + (e.message || e)); });
    });

    _refreshRecentFiles();

    // ── Language selector initialization ──
    function _updateStartScreenStrings() {
      document.documentElement.lang = window._eoCurrentLang;
      document.querySelectorAll('[data-i18n]').forEach(function(el) {
        el.textContent = _t(el.dataset.i18n);
      });
      var langLabel = document.getElementById('lang-label');
      if (langLabel) langLabel.textContent = _t('language');
    }

    (function initLangSelector() {
      var select = document.getElementById('lang-select');
      var langs = window._SUPPORTED_LANGS || [];

      for (var i = 0; i < langs.length; i++) {
        var opt = document.createElement('option');
        opt.value = langs[i].code;
        opt.textContent = langs[i].nativeName;
        if (langs[i].code === window._eoCurrentLang) opt.selected = true;
        select.appendChild(opt);
      }

      select.addEventListener('change', function() {
        window._eoSetLang(select.value);
        _updateStartScreenStrings();
        // Dates in the list are formatted for the UI language.
        _refreshRecentFiles();
      });

      _updateStartScreenStrings();
    })();
