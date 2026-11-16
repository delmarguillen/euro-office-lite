// The editor the Markdown page loads when Edit is pressed (markdown.js,
// loadCodeMirror). Bundled into src/markdown/vendor/codemirror.min.js by
// scripts/vendor-codemirror.sh; the page sees only window.EoCodeMirror.
import { EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, drawSelection } from '@codemirror/view';
import { history, historyKeymap, defaultKeymap } from '@codemirror/commands';
import { search, searchKeymap } from '@codemirror/search';
import { markdown } from '@codemirror/lang-markdown';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';

// The start screen's colours, as in markdown.css.
const theme = EditorView.theme({
  '&': { height: '100%', backgroundColor: '#181825', color: '#cdd6f4' },
  '.cm-scroller': {
    fontFamily: "ui-monospace, 'Cascadia Mono', Consolas, 'DejaVu Sans Mono', monospace",
    fontSize: '0.9rem',
    lineHeight: '1.5',
  },
  '.cm-content': { caretColor: '#cdd6f4', padding: '1rem 0' },
  '.cm-cursor': { borderLeftColor: '#cdd6f4' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': { backgroundColor: '#45475a' },
  '.cm-gutters': { backgroundColor: '#181825', color: '#6c7086', border: 'none' },
  '.cm-panels': { backgroundColor: '#1e1e2e', color: '#cdd6f4' },
  '.cm-searchMatch': { backgroundColor: '#f9e2af55' },
  '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: '#89b4fa66' },
}, { dark: true });

const highlight = HighlightStyle.define([
  { tag: tags.heading, color: '#89b4fa', fontWeight: 'bold' },
  { tag: tags.strong, fontWeight: 'bold' },
  { tag: tags.emphasis, fontStyle: 'italic' },
  { tag: tags.strikethrough, textDecoration: 'line-through' },
  { tag: tags.link, color: '#89dceb' },
  { tag: tags.url, color: '#89dceb' },
  { tag: tags.monospace, color: '#a6e3a1' },
  { tag: tags.quote, color: '#a6adc8', fontStyle: 'italic' },
  { tag: tags.list, color: '#f9e2af' },
  { tag: [tags.processingInstruction, tags.meta, tags.contentSeparator], color: '#6c7086' },
  { tag: [tags.tagName, tags.angleBracket], color: '#f38ba8' },
  { tag: tags.attributeName, color: '#fab387' },
  { tag: tags.comment, color: '#6c7086' },
]);

// CodeMirror's styles (style-mod) go into adoptedStyleSheets only inside a
// shadow root; in a document they go into a <style> element, which the page's
// CSP (style-src 'self') refuses. So the editor lives in a shadow root of
// `host`, and without adoptedStyleSheets it is not started at all.
function shadowFor(host) {
  const win = host.ownerDocument.defaultView;
  if (!win.ShadowRoot || !('adoptedStyleSheets' in win.ShadowRoot.prototype)) {
    throw new Error('this webview has no adoptedStyleSheets');
  }
  const shadow = host.attachShadow({ mode: 'open' });
  const wrapper = host.ownerDocument.createElement('div');
  wrapper.style.height = '100%';
  shadow.appendChild(wrapper);
  return wrapper;
}

// `options`: { host, text, label, onChange() }.
// The text goes in and comes out with LF line ends (EditorState splits on
// \r\n, \r and \n and joins with \n); tabs and trailing spaces are kept.
function createEditor(options) {
  const parent = shadowFor(options.host);
  const extensions = [
    lineNumbers(),
    drawSelection(),
    history(),
    search({ top: true }),
    // No continued list markup on Enter and no tag completion: plain editing.
    markdown({ addKeymap: false, completeHTMLTags: false }),
    syntaxHighlighting(highlight),
    EditorView.lineWrapping,
    keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap]),
    EditorView.contentAttributes.of({ 'aria-label': options.label || '', spellcheck: 'false' }),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) options.onChange();
    }),
    theme,
  ];
  const view = new EditorView({
    parent,
    state: EditorState.create({ doc: options.text, extensions }),
  });
  return {
    view,
    getText: () => view.state.doc.toString(),
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}

window.EoCodeMirror = { createEditor };
