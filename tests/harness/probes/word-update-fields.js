// word-update-fields.js: updates every field of the document, as "Update all
// fields" does, through the API of the editor and without the keyboard.
// For the `do` of a case: answers OK and whether the document had fields.
function (args, editor, win, need) {
  var had = need(editor, "asc_HaveFields").call(editor);
  need(editor, "asc_UpdateFields").call(editor, false);
  return "OK updated, fields=" + had;
}
