// The Markdown page's pure helpers, shared with the Node tests:
// - the bytes of a .md to the text the textarea edits, and back. A textarea's
//   value always ends lines with LF, so the file's BOM and line end are kept
//   aside and put back on save; saving untouched text gives the same bytes.
// - which image references the preview may show (local, inside the
//   document's folder) and which it blocks.
(function(global) {
  'use strict';

  var BOM = [0xef, 0xbb, 0xbf];
  var MARKDOWN_EXTENSIONS = ['md', 'markdown'];

  function startsWithBom(bytes) {
    return bytes.length >= 3 && bytes[0] === BOM[0] && bytes[1] === BOM[1] && bytes[2] === BOM[2];
  }

  // The line end most lines use. A file that mixes both is saved with it.
  function lineEndOf(text) {
    var crlf = (text.match(/\r\n/g) || []).length;
    var lf = (text.match(/\n/g) || []).length - crlf;
    return crlf > lf ? '\r\n' : '\n';
  }

  // { text, bom, eol }. Throws an Error with code 'not-utf8' for bytes that
  // are not UTF-8: decoding them anyway would change them on save.
  function decode(bytes) {
    var bom = startsWithBom(bytes);
    var body = bom ? bytes.subarray(3) : bytes;
    var raw;
    try {
      raw = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(body);
    } catch (e) {
      var error = new Error('the file is not UTF-8 text');
      error.code = 'not-utf8';
      throw error;
    }
    return {
      // What a textarea's value turns the text into anyway.
      text: raw.replace(/\r\n?/g, '\n'),
      bom: bom,
      eol: lineEndOf(raw)
    };
  }

  // The bytes to write for `text` in the format `decode` found.
  function encode(text, format) {
    var lines = text.replace(/\r\n?/g, '\n');
    var withEol = format.eol === '\r\n' ? lines.replace(/\n/g, '\r\n') : lines;
    var body = new TextEncoder().encode(withEol);
    if (!format.bom) return body;
    var bytes = new Uint8Array(body.length + 3);
    bytes.set(BOM, 0);
    bytes.set(body, 3);
    return bytes;
  }

  function isMarkdownPath(name) {
    var ext = String(name || '').split('.').pop().toLowerCase();
    return String(name || '').indexOf('.') !== -1 && MARKDOWN_EXTENSIONS.indexOf(ext) !== -1;
  }

  // What an <img src> in the document is:
  //   { kind: 'local', path }  relative, inside the folder: path has no '..'
  //   { kind: 'remote' }       http, https or protocol-relative: never fetched
  //   { kind: 'refused' }      anything else (absolute, another scheme, '..'
  //                            that climbs out of the folder)
  // The backend checks the real path again (markdown_image_path).
  function classifyImageSource(src) {
    var value = String(src || '').trim();
    if (/^(https?:)?\/\//i.test(value)) return { kind: 'remote' };
    if (!value || /^[a-z][a-z0-9+.-]*:/i.test(value)) return { kind: 'refused' };
    if (value[0] === '/' || value[0] === '\\') return { kind: 'refused' };
    var path = relativePath(value.split(/[?#]/)[0]);
    return path ? { kind: 'local', path: path } : { kind: 'refused' };
  }

  // The reference without '.', '..' or empty segments, or null when it is
  // not a path or climbs above the folder.
  function relativePath(reference) {
    var decoded;
    try {
      decoded = decodeURIComponent(reference);
    } catch (e) {
      return null;
    }
    if (decoded.indexOf('\0') !== -1) return null;
    var kept = [];
    var segments = decoded.split(/[\\/]/);
    for (var i = 0; i < segments.length; i++) {
      var segment = segments[i];
      if (segment === '' || segment === '.') continue;
      if (segment !== '..') {
        kept.push(segment);
        continue;
      }
      if (!kept.length) return null;
      kept.pop();
    }
    return kept.length ? kept.join('/') : null;
  }

  global.EoMarkdownText = {
    decode: decode,
    encode: encode,
    isMarkdownPath: isMarkdownPath,
    classifyImageSource: classifyImageSource,
    MARKDOWN_EXTENSIONS: MARKDOWN_EXTENSIONS
  };
})(typeof window !== 'undefined' ? window : globalThis);
