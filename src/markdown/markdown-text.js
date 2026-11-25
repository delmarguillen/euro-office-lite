// The Markdown page's pure helpers, shared with the Node tests:
// - the bytes of a .md to the text the textarea edits, and back. A textarea's
//   value always ends lines with LF, so the file's BOM and line end are kept
//   aside and put back on save; saving untouched text writes the file's own
//   bytes again, whatever line ends they mix.
// - which image references the preview may show (local, inside the
//   document's folder) and which it blocks.
// - whether the document has unsaved changes while a save runs, and what
//   Close and Quit do then.
(function(global) {
  'use strict';

  var BOM = [0xef, 0xbb, 0xbf];
  var MARKDOWN_EXTENSIONS = ['md', 'markdown'];

  function startsWithBom(bytes) {
    return bytes.length >= 3 && bytes[0] === BOM[0] && bytes[1] === BOM[1] && bytes[2] === BOM[2];
  }

  function countOf(text, pattern) {
    return (text.match(pattern) || []).length;
  }

  // The line end most lines use: CRLF, LF or a lone CR. An edited file that
  // mixes them is saved with it; a tie keeps LF.
  function lineEndOf(text) {
    var crlf = countOf(text, /\r\n/g);
    var lf = countOf(text, /\n/g) - crlf;
    var cr = countOf(text, /\r/g) - crlf;
    if (crlf > lf && crlf >= cr) return '\r\n';
    if (cr > lf) return '\r';
    return '\n';
  }

  function notUtf8Error() {
    var error = new Error('the file is not UTF-8 text');
    error.code = 'not-utf8';
    return error;
  }

  // { text, bom, eol, bytes }. Throws an Error with code 'not-utf8' for bytes that
  // are not UTF-8: decoding them anyway would change them on save. Used as it
  // is for the page's own recovery copies, which hold whatever encode wrote,
  // a pasted NUL included.
  function decode(bytes) {
    var bom = startsWithBom(bytes);
    var body = bom ? bytes.subarray(3) : bytes;
    var raw;
    try {
      raw = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(body);
    } catch (e) {
      throw notUtf8Error();
    }
    return {
      // What a textarea's value turns the text into anyway.
      text: raw.replace(/\r\n?/g, '\n'),
      bom: bom,
      eol: lineEndOf(raw),
      // What saving `text` untouched writes: one eol cannot give back a
      // file that mixes line ends.
      bytes: bytes
    };
  }

  // decode for a file being opened. A NUL is valid UTF-8 but never in a text
  // file, and UTF-16 without a BOM is full of them: such a file is refused as
  // not UTF-8.
  function decodeFile(bytes) {
    var body = startsWithBom(bytes) ? bytes.subarray(3) : bytes;
    if (body.indexOf(0) !== -1) throw notUtf8Error();
    return decode(bytes);
  }

  // The bytes to write for `text` in the format `decode` found: its own bytes
  // when the text is as decoded.
  function encode(text, format) {
    if (format.bytes && text === format.text) return format.bytes;
    var lines = text.replace(/\r\n?/g, '\n');
    var withEol = format.eol === '\n' ? lines : lines.replace(/\n/g, format.eol);
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

  // ── Unsaved changes while a save runs ──
  // saved: the text last read or saved, null for recovered text never saved.
  // writing: the text a save is writing, null when none is.
  function savedState(saved) {
    return { saved: saved, writing: null };
  }

  // event: { type: 'write-start', text } or { type: 'write-end', ok }.
  function afterSaveEvent(state, event) {
    if (event.type === 'write-start') return { saved: state.saved, writing: event.text };
    if (event.type !== 'write-end') throw new Error('unknown save event ' + event.type);
    var saved = event.ok ? state.writing : state.saved;
    return { saved: saved, writing: null };
  }

  // For set_document_modified and set_save_pending: until the write ends the
  // file may still change, even with the text back to the saved one.
  function unsavedChanges(state, text) {
    return { modified: text !== state.saved, savePending: state.writing !== null };
  }

  // Whether the text on screen is in no file and needs a recovery copy. While
  // a write runs the file is about to hold the text being written.
  function needsRecoveryCopy(state, text) {
    if (state.writing !== null) return text !== state.writing;
    return text !== state.saved;
  }

  // Close, the window's close button and Quit: 'wait' for the write and ask
  // again, 'ask' before discarding changes, or 'close'.
  function closeAction(state, text) {
    if (state.writing !== null) return 'wait';
    if (text !== state.saved) return 'ask';
    return 'close';
  }

  global.EoMarkdownText = {
    decode: decode,
    decodeFile: decodeFile,
    encode: encode,
    isMarkdownPath: isMarkdownPath,
    classifyImageSource: classifyImageSource,
    savedState: savedState,
    afterSaveEvent: afterSaveEvent,
    unsavedChanges: unsavedChanges,
    needsRecoveryCopy: needsRecoveryCopy,
    closeAction: closeAction,
    MARKDOWN_EXTENSIONS: MARKDOWN_EXTENSIONS
  };
})(typeof window !== 'undefined' ? window : globalThis);
