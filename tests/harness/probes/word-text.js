// word-text.js: the pages of the document and its text as a copy of the whole
// document gives it, with paragraph numbers and field results; a tab reads
// as " | ". Selecting the whole document is the only thing it changes.
function (args, editor, win, need) {
  var pages = need(editor, "private_GetLogicDocument").call(editor).GetPagesCount();
  need(editor, "asc_GetSelectedText");
  var lines = ["pages=" + pages];
  editor.asc_EditSelectAll();
  var text = editor.asc_GetSelectedText(false, { NewLineSeparator: "\n", Numbering: true }) || "";
  text.split(/\r?\n/).forEach(function (line) {
    if (line.trim()) lines.push("text=" + line.replace(/\t/g, " | ").trim());
  });
  return lines;
}
