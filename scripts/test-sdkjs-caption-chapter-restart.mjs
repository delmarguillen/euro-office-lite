// Checks the captions that Insert caption adds with "Include chapter number",
// in src/sdkjs with the sdkjs patches applied. The dialog gives the heading as
// an index from 0 (Heading 1 is 0); the fields name the level from 1, as Word
// writes them:
//   Table { STYLEREF "Heading 1" \s }-{ SEQ Table \* Arabic \s 1 }
// \s 1 starts the count again after each Heading 1, so two captions in the
// first chapter and one in the second read 1-1, 1-2 and 2-1. With Heading 2
// the fields are STYLEREF "Heading 2" and \s 2.
//
// Run: node scripts/apply-patches.mjs apply --only sdkjs
//      node scripts/test-sdkjs-caption-chapter-restart.mjs [path/to/sdkjs]
//      node scripts/apply-patches.mjs revert --only sdkjs
import { makeReport, openWordEditor } from './sdkjs-word-test-harness.mjs';

const { same, finish } = makeReport();

// Inside the vm: what the caption dialog and its OK button do.
const CAPTIONS = `
  var captions = [];
  // headingIndex is the value of the dialog's heading list: 0 for Heading 1.
  function insertCaption(after, label, headingIndex, separator) {
    var props = new Asc.CAscCaptionProperties();
    props.put_Before(false);
    props.put_Label(label);
    props.put_ExcludeLabel(false);
    props.put_Format(Asc.c_oAscNumberingFormat.Decimal);
    props.put_IncludeChapterNumber(undefined !== headingIndex);
    props.put_HeadingLvl(undefined === headingIndex ? 0 : headingIndex);
    props.put_Separator(separator || '-');
    after.SetThisElementCurrent();
    after.MoveCursorToEndPos();
    Asc.asc_docs_api.prototype.asc_AddObjectCaption.call(AscTest.Editor, props);
    var caption = doc.GetElement(after.GetIndex() + 1);
    captions.push(caption);
    recalculate();
    return caption;
  }
  function instructionsOf(p) {
    return p.GetAllFields().map(function (field) { return field.GetInstructionLine().trim(); });
  }
  function updateAllFields() {
    var fields = [];
    for (var index = 0; index < captions.length; ++index) fields = fields.concat(captions[index].GetAllFields());
    updateFields(fields);
  }
`;

// --- Heading 1: two captions in the first chapter, one in the second ---
let run = openWordEditor(process.argv[2]);
run(CAPTIONS);
run(`
  numberHeadings(['Chapter %1.', '%1.%2.']);
  addParagraph('First', heading(1));
  var body1 = addParagraph('One');
  var body2 = addParagraph('Two');
  addParagraph('Second', heading(1));
  var body3 = addParagraph('Three');
  recalculate();

  var table1 = insertCaption(body1, 'Table', 0);
  var table2 = insertCaption(body2, 'Table', 0);
  var table3 = insertCaption(body3, 'Table', 0);
`);

same('Heading 1: the fields name Heading 1 and level 1', run('instructionsOf(table1)'), [
  'STYLEREF "Heading 1" \\s',
  'SEQ Table \\* Arabic \\s 1',
]);
same('as inserted: 1-1, 1-2 and 2-1', run('[textOf(table1), textOf(table2), textOf(table3)]'), ['Table 1-1', 'Table 1-2', 'Table 2-1']);
run('updateAllFields();');
same('after updating the fields: 1-1, 1-2 and 2-1', run('[textOf(table1), textOf(table2), textOf(table3)]'), [
  'Table 1-1',
  'Table 1-2',
  'Table 2-1',
]);

run(`
  var body4 = addParagraph('Four');
  recalculate();
  var table4 = insertCaption(body4, 'Table', 0);
  var figure1 = insertCaption(body1, 'Figure', 0);
  updateAllFields();
`);
same('the count goes on inside the second chapter', run('textOf(table4)'), 'Table 2-2');
same('another label has its own count', run('textOf(figure1)'), 'Figure 1-1');

// --- Heading 2, numbered "Section 1.": the count starts again in each section ---
run = openWordEditor(process.argv[2]);
run(CAPTIONS);
run(`
  numberHeadings(['Chapter %1.', 'Section %2.']);
  addParagraph('First', heading(1));
  addParagraph('Part', heading(2));
  var body1 = addParagraph('One');
  var body2 = addParagraph('Two');
  addParagraph('Other part', heading(2));
  var body3 = addParagraph('Three');
  addParagraph('Second', heading(1));
  addParagraph('Part', heading(2));
  var body4 = addParagraph('Four');
  recalculate();

  var table1 = insertCaption(body1, 'Table', 1);
  var table2 = insertCaption(body2, 'Table', 1);
  var table3 = insertCaption(body3, 'Table', 1);
  var table4 = insertCaption(body4, 'Table', 1);
  updateAllFields();
`);
same('Heading 2: the fields name Heading 2 and level 2', run('instructionsOf(table1)'), [
  'STYLEREF "Heading 2" \\s',
  'SEQ Table \\* Arabic \\s 2',
]);
same('Heading 2: the count starts again in each section', run('[textOf(table1), textOf(table2), textOf(table3), textOf(table4)]'), [
  'Table 1-1',
  'Table 1-2',
  'Table 2-1',
  'Table 1-1',
]);

// --- Heading 9, the last entry of the list ---
run = openWordEditor(process.argv[2]);
run(CAPTIONS);
run(`
  var body = addParagraph('One');
  recalculate();
  var table = insertCaption(body, 'Table', 8);
`);
same('Heading 9: the fields name Heading 9 and level 9', run('instructionsOf(table)'), [
  'STYLEREF "Heading 9" \\s',
  'SEQ Table \\* Arabic \\s 9',
]);

// --- A caption saved before with \s 0 next to a new one ---
// This editor used to write \s 0 for Heading 1; Word writes the level from 1
// and never 0. A document with both counts once per chapter.
run = openWordEditor(process.argv[2]);
run(CAPTIONS);
run(`
  numberHeadings(['Chapter %1.', '%1.%2.']);
  var oldFields = [];
  function addOldCaption(label) {
    var p = addParagraph(label + ' ');
    oldFields.push(addField(p, ' STYLEREF "Heading 1" \\\\s'));
    addText(p, '-');
    oldFields.push(addField(p, ' SEQ ' + label + ' \\\\* Arabic \\\\s 0'));
    captions.push(p);
    return p;
  }
  addParagraph('First', heading(1));
  var old1 = addOldCaption('Table');
  addParagraph('Second', heading(1));
  var old2 = addOldCaption('Table');
  var body = addParagraph('Body');
  recalculate();
  updateFields(oldFields);
  var table = insertCaption(body, 'Table', 0);
`);
same('\\s 0: the old field keeps its instruction', run('instructionsOf(old2)'), [
  'STYLEREF "Heading 1" \\s',
  'SEQ Table \\* Arabic \\s 0',
]);
same('\\s 0 next to \\s 1, as inserted: 1-1, 2-1 and 2-2', run('[textOf(old1), textOf(old2), textOf(table)]'), [
  'Table 1-1',
  'Table 2-1',
  'Table 2-2',
]);
run('updateAllFields();');
same('\\s 0 next to \\s 1, after updating the fields: 1-1, 2-1 and 2-2', run('[textOf(old1), textOf(old2), textOf(table)]'), [
  'Table 1-1',
  'Table 2-1',
  'Table 2-2',
]);

// --- Without the chapter number ---
run = openWordEditor(process.argv[2]);
run(CAPTIONS);
run(`
  addParagraph('First', heading(1));
  var body1 = addParagraph('One');
  addParagraph('Second', heading(1));
  var body2 = addParagraph('Two');
  recalculate();
  var table1 = insertCaption(body1, 'Table');
  var table2 = insertCaption(body2, 'Table');
  updateAllFields();
`);
same('no chapter number: one SEQ field without \\s', run('instructionsOf(table1)'), ['SEQ Table \\* Arabic']);
same('no chapter number: the count goes on', run('[textOf(table1), textOf(table2)]'), ['Table 1', 'Table 2']);

finish();
