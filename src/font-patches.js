    var _log = function() {
      if (window._eoLog) { window._eoLog.apply(null, arguments); }
      else { console.log.apply(console, arguments); }
    };

    function _isAbsolutePath(p) {
      if (!p || p.length < 3) return false;
      if (p[0] === '/' && p[1] !== '/') return true;
      if (p[1] === ':' && (p[2] === '/' || p[2] === '\\')) return true;
      return false;
    }

    function _normalizeAbsolutePath(p) {
      // x2t writes Windows extended paths as /?/C:/... in AllFonts.js.
      return p && /^\/\?\/[A-Za-z]:\//.test(p) ? p.substring(3) : p;
    }

    function _patchLoadFontAsync(win) {
      win.AscFonts.CFontFileLoader.prototype.LoadFontAsync = function(basePath, callback) {
        this.callback = callback;
        if (-1 !== this.Status) return true;

        this.Status = 2;
        var xhr = new XMLHttpRequest();
        xhr.fontFile = this;
        var fontId = _normalizeAbsolutePath(this.Id);
        var url = _isAbsolutePath(fontId)
          ? window._eoAscProtoBase + 'abs/' + fontId
          : '/fonts/' + this.Id;
        xhr.open('GET', url, true);
        xhr.responseType = 'arraybuffer';

        xhr.onload = function() {
          if (this.status !== 200) {
            this.fontFile.Status = 1;
            return;
          }
          this.fontFile.Status = 0;
          var fontStreams = win.AscFonts.g_fonts_streams;
          var streamIndex = fontStreams.length;
          var data = new Uint8Array(this.response);
          fontStreams[streamIndex] = new win.AscFonts.FontStream(data, data.length);
          this.fontFile.SetStreamIndex(streamIndex);
          if (null != this.fontFile.callback) this.fontFile.callback();
          if (this.fontFile["externalCallback"]) this.fontFile["externalCallback"]();
        };

        xhr.send(null);
        return false;
      };
      win.AscFonts.CFontFileLoader.prototype["LoadFontAsync"] = win.AscFonts.CFontFileLoader.prototype.LoadFontAsync;
    }

    function _preloadAllFonts(win) {
      var ff = win.AscFonts.g_font_files;
      if (!ff) return 0;
      var loaded = 0;
      for (var i = 0; i < ff.length; i++) {
        if (ff[i].Status !== -1) continue;
        var fontId = ff[i].Id;
        if (!fontId) continue;
        try {
          var xhr = new XMLHttpRequest();
          var normalizedFontId = _normalizeAbsolutePath(fontId);
          var furl = _isAbsolutePath(normalizedFontId)
            ? window._eoAscProtoBase + 'abs/' + normalizedFontId
            : '/fonts/' + fontId;
          xhr.open('GET', furl, false);
          xhr.overrideMimeType('text/plain; charset=x-user-defined');
          xhr.send(null);
          if (xhr.status === 200) {
            var text = xhr.responseText;
            var data = new Uint8Array(text.length);
            for (var j = 0; j < text.length; j++) data[j] = text.charCodeAt(j) & 0xFF;
            ff[i].LoadFontFromData(data);
            loaded++;
          }
        } catch(e) {}
      }
      return loaded;
    }

    function _injectFontSelections(win) {
      try {
        var gfa = win.AscFonts && win.AscFonts.g_fontApplication;
        if (!gfa || !gfa.g_fontSelections || !gfa.g_fontSelections.IsInit) return;
        if (win._eoFontSelectionsInjected) return;

        var list = gfa.g_fontSelections.List;
        if (!list || list.length === 0) return;

        var proto = Object.getPrototypeOf(list[list.length - 1]);

        var fontFamilies = [
          { name: 'Liberation Sans', fixed: false, familyClass: 2053, variants: [
            { file: 'LiberationSans-Regular.ttf',    bold: false, italic: false, weight: 400, panose: [2,11,6,4,2,2,2,2,2,4], avgW: 1187, asc: 1491, desc: -431, lineGap: 307, xH: 1082, capH: 1409, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationSans-Italic.ttf',     bold: false, italic: true,  weight: 400, panose: [2,11,6,4,2,2,2,9,2,4], avgW: 1185, asc: 1491, desc: -425, lineGap: 307, xH: 1082, capH: 1409, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationSans-Bold.ttf',       bold: true,  italic: false, weight: 700, panose: [2,11,7,4,2,2,2,2,2,4], avgW: 1248, asc: 1491, desc: -431, lineGap: 307, xH: 1082, capH: 1409, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationSans-BoldItalic.ttf', bold: true,  italic: true,  weight: 700, panose: [2,11,7,4,2,2,2,9,2,4], avgW: 1249, asc: 1491, desc: -431, lineGap: 307, xH: 1082, capH: 1409, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 }
          ]},
          { name: 'Liberation Serif', fixed: false, familyClass: 261, variants: [
            { file: 'LiberationSerif-Regular.ttf',    bold: false, italic: false, weight: 400, panose: [2,2,6,3,5,4,5,2,3,4], avgW: 1124, asc: 1420, desc: -442, lineGap: 307, xH: 940, capH: 1341, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationSerif-Italic.ttf',     bold: false, italic: true,  weight: 400, panose: [2,2,5,3,5,4,5,9,3,4], avgW: 1098, asc: 1422, desc: -442, lineGap: 307, xH: 940, capH: 1341, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationSerif-Bold.ttf',       bold: true,  italic: false, weight: 700, panose: [2,2,8,3,7,5,5,2,3,4], avgW: 1180, asc: 1387, desc: -442, lineGap: 307, xH: 940, capH: 1341, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationSerif-BoldItalic.ttf', bold: true,  italic: true,  weight: 700, panose: [2,2,7,3,6,5,5,9,3,4], avgW: 1141, asc: 1387, desc: -442, lineGap: 307, xH: 940, capH: 1341, ur1: 0xE0000AFF, ur2: 0x500078FF, ur3: 0x00000021, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 }
          ]},
          { name: 'Liberation Mono', fixed: true, familyClass: 2053, variants: [
            { file: 'LiberationMono-Regular.ttf',    bold: false, italic: false, weight: 400, panose: [2,7,4,9,2,2,5,2,4,4], avgW: 1229, asc: 1255, desc: -386, lineGap: 0, xH: 1082, capH: 1349, ur1: 0xE0000AFF, ur2: 0x400078FF, ur3: 0x00000001, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationMono-Italic.ttf',     bold: false, italic: true,  weight: 400, panose: [2,7,4,9,2,2,5,9,4,4], avgW: 1229, asc: 1255, desc: -386, lineGap: 0, xH: 1082, capH: 1349, ur1: 0xE0000AFF, ur2: 0x400078FF, ur3: 0x00000001, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationMono-Bold.ttf',       bold: true,  italic: false, weight: 700, panose: [2,7,7,9,2,2,5,2,4,4], avgW: 1229, asc: 1297, desc: -428, lineGap: 0, xH: 1082, capH: 1349, ur1: 0xE0000AFF, ur2: 0x400078FF, ur3: 0x00000001, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 },
            { file: 'LiberationMono-BoldItalic.ttf', bold: true,  italic: true,  weight: 700, panose: [2,7,7,9,2,2,5,9,4,4], avgW: 1229, asc: 1297, desc: -428, lineGap: 0, xH: 1082, capH: 1349, ur1: 0xE0000AFF, ur2: 0x400078FF, ur3: 0x00000001, ur4: 0, cp1: 0x600001BF, cp2: 0xDFF70000 }
          ]},
          { name: 'Carlito', fixed: false, familyClass: 0, variants: [
            { file: 'Carlito-Regular.ttf',    bold: false, italic: false, weight: 400, panose: [2,15,5,2,2,2,4,3,2,4], avgW: 1048, asc: 1536, desc: -512, lineGap: 452, xH: 978, capH: 1314, ur1: 0xE10002FF, ur2: 0x5000ECFF, ur3: 0x00000009, ur4: 0, cp1: 0x2000019F, cp2: 0x00000000 },
            { file: 'Carlito-Italic.ttf',     bold: false, italic: true,  weight: 400, panose: [2,15,5,2,2,2,4,3,2,4], avgW: 1041, asc: 1536, desc: -512, lineGap: 452, xH: 983, capH: 1314, ur1: 0xE10002FF, ur2: 0x5000ECFF, ur3: 0x00000009, ur4: 0, cp1: 0x2000019F, cp2: 0x00000000 },
            { file: 'Carlito-Bold.ttf',       bold: true,  italic: false, weight: 700, panose: [2,15,5,2,2,2,4,3,2,4], avgW: 1074, asc: 1536, desc: -512, lineGap: 452, xH: 993, capH: 1328, ur1: 0xE10002FF, ur2: 0x5000ECFF, ur3: 0x00000009, ur4: 0, cp1: 0x2000019F, cp2: 0x00000000 },
            { file: 'Carlito-BoldItalic.ttf', bold: true,  italic: true,  weight: 700, panose: [2,15,5,2,2,2,4,3,2,4], avgW: 1066, asc: 1536, desc: -512, lineGap: 452, xH: 993, capH: 1328, ur1: 0xE10002FF, ur2: 0x5000ECFF, ur3: 0x00000009, ur4: 0, cp1: 0x2000019F, cp2: 0x00000000 }
          ]}
        ];

        for (var fi = 0; fi < fontFamilies.length; fi++) {
          var fam = fontFamilies[fi];
          for (var vi = 0; vi < fam.variants.length; vi++) {
            var v = fam.variants[vi];
            var fs = Object.create(proto);
            fs.m_wsFontName = fam.name;
            fs.m_wsFontPath = v.file;
            fs.m_lIndex = 0;
            fs.m_bBold = v.bold;
            fs.m_bItalic = v.italic;
            fs.m_bIsFixed = fam.fixed;
            fs.m_aPanose = new Int8Array(v.panose);
            fs.m_ulUnicodeRange1 = v.ur1;
            fs.m_ulUnicodeRange2 = v.ur2;
            fs.m_ulUnicodeRange3 = v.ur3;
            fs.m_ulUnicodeRange4 = v.ur4;
            fs.m_ulCodePageRange1 = v.cp1;
            fs.m_ulCodePageRange2 = v.cp2;
            fs.m_usWeigth = v.weight;
            fs.m_usWidth = 5;
            fs.m_sFamilyClass = fam.familyClass;
            fs.m_eFontFormat = 0;
            fs.m_shAvgCharWidth = v.avgW;
            fs.m_shAscent = v.asc;
            fs.m_shDescent = v.desc;
            fs.m_shLineGap = v.lineGap;
            fs.m_shXHeight = v.xH;
            fs.m_shCapHeight = v.capH;
            fs.m_usType = 0;
            fs.m_names = null;

            list.splice(list.length - 1, 0, fs);
            gfa.g_fontSelections.ListMap[v.file] = list.length - 2;
          }
        }

        var oSelect = { wsName: 'Arial' };
        gfa.DefaultIndex = gfa.g_fontDictionary.GetFontIndex(oSelect, list, undefined);

        win._eoFontSelectionsInjected = true;
      } catch(e) {}
    }

    var _systemFontsJs = null;
    var _systemFontsPromise = null;

    function _prepareSystemFonts() {
      if (_systemFontsJs) return Promise.resolve(_systemFontsJs);
      if (_systemFontsPromise) return _systemFontsPromise;
      _systemFontsPromise = window.__TAURI__.core.invoke('get_system_fonts').then(function(js) {
        _systemFontsJs = js || '';
        return _systemFontsJs;
      }).catch(function(e) {
        _log('[EO] get_system_fonts failed: ' + e.message);
        return '';
      });
      return _systemFontsPromise;
    }

    function _resetFontSelections(fw) {
      var gfa = fw.AscFonts && fw.AscFonts.g_fontApplication;
      var selections = gfa && gfa.g_fontSelections;
      if (!selections || !fw['g_fonts_selection_bin']) return;

      selections.List = [];
      selections.ListMap = {};
      selections.Languages = [];
      selections.m_pRanges = null;
      selections.m_pRangesNums = null;
      selections.IsInit = false;
      selections.CurrentLoadedObj = null;
      selections.Init();

      var oSelect = { wsName: 'Arial' };
      gfa.DefaultIndex = gfa.g_fontDictionary.GetFontIndex(oSelect, selections.List, undefined);
      // The generated selection binary already includes bundled and system fonts.
      fw._eoFontSelectionsInjected = true;
    }

    function _refreshEditorFonts(fw, loader) {
      if (!loader || !loader.Api || !loader.Api.sync_InitEditorFonts) return;

      // If the UI already consumed the bundled list, reset its collection before
      // re-emitting the complete list so the original ten entries are not duplicated.
      var collectionReset = false;
      var apps = [fw.DE, fw.SSE, fw.PE];
      for (var ai = 0; ai < apps.length && !collectionReset; ai++) {
        try {
          var app = apps[ai];
          if (!app || !app.getController) continue;
          var controller = app.getController('Common.Controllers.Fonts') || app.getController('Fonts');
          var store = controller && controller.getCollection && controller.getCollection('Common.Collections.Fonts');
          if (store && store.reset) {
            store.reset();
            collectionReset = true;
          }
        } catch(e) {}
      }

      // With no collection yet, the normal first LoadDocumentFonts call will emit it.
      if (!collectionReset) return;
      var guiFonts = [];
      for (var i = 0; i < loader.fontInfos.length; i++) {
        var info = loader.fontInfos[i];
        if (info.Name !== 'ASCW3') {
          guiFonts.push(new fw.AscFonts.CFont(info.Name, '', info.Thumbnail));
        }
      }
      loader.Api.sync_InitEditorFonts(guiFonts);
    }

    function _installFontThumbnailFallback(fw) {
      try {
        var Combo = fw.Common && fw.Common.UI && fw.Common.UI.ComboBoxFonts;
        if (!Combo || !Combo.prototype || Combo.prototype._eoFontFallbackPatched) return;
        Combo.prototype._eoFontFallbackPatched = true;

        // Draw thumbnails dark; the editor's CSS filter adapts them to the theme.
        function drawFontName(combo, fontName, canvas, ctx) {
          var sprite = combo.spriteThumbs || {};
          if (!canvas) canvas = fw.document.createElement('canvas');
          canvas.width = sprite.width || 300;
          canvas.height = sprite.heightOne || 28;
          canvas.style.width = '300px';
          canvas.style.height = '28px';
          ctx = ctx || canvas.getContext('2d');
          ctx.clearRect(0, 0, canvas.width, canvas.height);
          var scale = canvas.height / 28;
          ctx.fillStyle = '#333';
          ctx.textBaseline = 'middle';
          ctx.font = Math.max(12, Math.round(15 * scale)) + 'px "' +
            fontName.replace(/["\\]/g, '') + '", sans-serif';
          ctx.fillText(fontName, Math.round(6 * scale), canvas.height / 2,
            canvas.width - Math.round(12 * scale));
          return canvas;
        }

        // fillFonts registers store 'add'/'remove' listeners without detaching
        // the previous ones. Upstream runs it once, but the system-fonts
        // re-emit from _refreshEditorFonts runs it a second time; the stacked
        // listeners then prepend every new recent-font row twice (one store
        // record, two identical li elements). Detach before the original
        // re-registers so exactly one listener pair remains.
        var originalFillFonts = Combo.prototype.fillFonts;
        Combo.prototype.fillFonts = function(store, select) {
          if (this.store && this.onInsertItem) {
            this.store.off('add', this.onInsertItem, this);
            this.store.off('remove', this.onRemoveItem, this);
          }
          return originalFillFonts.apply(this, arguments);
        };

        // Idempotence guard: a row per record id must be unique even if the
        // listeners above ever stack again through an async re-registration.
        var originalOnInsertItem = Combo.prototype.onInsertItem;
        Combo.prototype.onInsertItem = function(item) {
          if (item && item.id && fw.$(this.el).find('li[id="' + item.id + '"]').length) return;
          return originalOnInsertItem.apply(this, arguments);
        };

        var originalLoadSprite = Combo.prototype.loadSprite;
        Combo.prototype.loadSprite = function(callback) {
          var combo = this;
          return originalLoadSprite.call(this, function() {
            var sprite = combo.spriteThumbs;
            if (sprite && !sprite._eoFontFallbackPatched) {
              sprite._eoFontFallbackPatched = true;
              var originalGetImage = sprite.getImage;
              sprite.getImage = function(index, canvas, ctx) {
                var fontName = fw._eoFontThumbnailNames && fw._eoFontThumbnailNames[index];
                // Every generated thumbnail index belongs to the missing runtime
                // sprite, including low indexes that overlap the bundled sprite.
                if (fontName) {
                  return drawFontName(combo, fontName, canvas, ctx);
                }
                return originalGetImage.call(this, index, canvas, ctx);
              };
            }
            callback();
          });
        };

        // The generated thumbnail index is not unique (and -1 is shared by many
        // families), so list rows must use the model's actual name. Resolve the
        // record through the row's own li id, never by list position: recent
        // fonts, dividers or menu re-renders shift positional indexes, painting
        // one font's name on a row whose click applies a different font.
        Combo.prototype.updateVisibleFontsTiles = function(e, scrollY) {
          var combo = this, j = 0;
          var listItems = fw.$(combo.el).find('a');
          var anchorCount = listItems.length;
          if (!combo.tiles) combo.tiles = [];
          if (anchorCount !== combo.tiles.length) {
            for (j = combo.tiles.length; j < anchorCount; ++j) combo.tiles.unshift(null);
          }
          if (typeof scrollY === 'undefined') {
            scrollY = parseInt(fw.$(combo.el).find('.ps-scrollbar-x-rail').css('bottom'));
          }
          var scrollH = fw.$(combo.el).find('.dropdown-menu').height();
          var itemHeight = combo.getListItemHeight ? combo.getListItemHeight() : 28;
          var count = Math.max(Math.floor(scrollH / itemHeight) + 3, 0);
          var from = Math.max(Math.floor(-(scrollY / itemHeight)) - 1, 0);
          var to = from + count;

          for (j = 0; j < anchorCount; ++j) {
            var anchor = listItems[j];
            // A canvas left attached to a replaced or reordered anchor no
            // longer matches this row; drop it so the row is redrawn.
            if (combo.tiles[j] && combo.tiles[j].parentNode !== anchor) {
              if (combo.tiles[j].parentNode) combo.tiles[j].parentNode.removeChild(combo.tiles[j]);
              combo.tiles[j] = null;
            }
            if (from <= j && j < to) {
              if (null === combo.tiles[j] && anchor) {
                var rowId = fw.$(anchor).closest('li').attr('id');
                var record = rowId ? combo.store.findWhere({ id: rowId }) : null;
                var fontName = record && record.get('name');
                var fontImage = fontName ? drawFontName(combo, fontName) : null;
                combo.tiles[j] = fontImage;
                if (fontImage) anchor.appendChild(fontImage);
              }
            } else if (combo.tiles[j]) {
              if (combo.tiles[j].parentNode) combo.tiles[j].parentNode.removeChild(combo.tiles[j]);
              combo.tiles[j] = null;
            }
          }
        };

        var originalGetImageUri = Combo.prototype.getImageUri;
        Combo.prototype.getImageUri = function(opts) {
          if (opts && opts.name && fw._eoSystemFontsInjected) {
            return drawFontName(this, opts.name).toDataURL();
          }
          return originalGetImageUri.call(this, opts);
        };
      } catch(e) {
        _log('[EO] Font thumbnail fallback error: ' + e.message);
      }
    }

    function _applySystemFontsToFrame(fw) {
      if (!_systemFontsJs || !fw || fw._eoSystemFontsInjected ||
          !fw.AscFonts || !fw.AscFonts.checkAllFonts) return false;
      try {
        fw._eoSystemFontsInjected = true;
        new fw.Function(_systemFontsJs)();

        // checkAllFonts is normally called once and does not clear its name map.
        // A second call must remove stale aliases whose indexes belonged to the
        // bundled ten-font array.
        var fontMap = fw.AscFonts.g_map_font_index || {};
        Object.keys(fontMap).forEach(function(name) { delete fontMap[name]; });
        fw.AscFonts.checkAllFonts();
        _patchLoadFontAsync(fw);

        var loader = fw.AscCommon && fw.AscCommon.g_font_loader;
        if (loader) {
          loader.fontFiles = fw.AscFonts.g_font_files;
          loader.fontInfos = fw.AscFonts.g_font_infos;
          loader.map_font_index = fw.AscFonts.g_map_font_index;
        }
        _resetFontSelections(fw);

        fw._eoFontThumbnailNames = {};
        for (var ti = 0; ti < fw.AscFonts.g_font_infos.length; ti++) {
          var fontInfo = fw.AscFonts.g_font_infos[ti];
          fw._eoFontThumbnailNames[fontInfo.Thumbnail] = fontInfo.Name;
        }
        _installFontThumbnailFallback(fw);
        _refreshEditorFonts(fw, loader);

        return true;
      } catch(e) {
        fw._eoSystemFontsInjected = false;
        _log('[EO] System fonts inject error: ' + e.message);
        return false;
      }
    }

    async function _loadSystemFonts() {
      try {
        var js = await _prepareSystemFonts();
        if (!js) return;
        var allFrames = document.querySelectorAll('iframe');
        for (var fi = 0; fi < allFrames.length; fi++) {
          try {
            var fw = allFrames[fi].contentWindow;
            _applySystemFontsToFrame(fw);
          } catch(e) {}
        }
      } catch(e) {}
    }
