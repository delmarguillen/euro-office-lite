// Checks the chapter number of a caption in src/sdkjs with the sdkjs patches
// applied. A caption with "Include chapter number" is the label and two
// fields, as Word writes it too:
//   Table { STYLEREF "Heading 1" \s }-{ SEQ Table \* Arabic \s 1 }
// With Heading 1 numbered "Chapter %1." the STYLEREF field gives the number
// alone, "1", so the caption reads "Table 1-1" and not "Table Chapter 1-1".
// The same holds for \t next to \n, \r or \w, which ECMA-376 defines as leaving
// out the text that is neither a number nor a delimiter.
//
// Run: node scripts/apply-patches.mjs apply --only sdkjs
//      node scripts/test-sdkjs-caption-chapter-number.mjs [path/to/sdkjs]
//      node scripts/apply-patches.mjs revert --only sdkjs
import { makeReport, openWordEditor } from './sdkjs-word-test-harness.mjs';

const { same, finish } = makeReport();

// --- Headings numbered "Chapter 1." and "1.1." ---
let run = openWordEditor(process.argv[2]);
run(`
  numberHeadings(['Chapter %1.', '%1.%2.']);
  var fields = [];
  // A caption: the label, the chapter number field, a hyphen and the SEQ field.
  function addCaption(label, chapterInstruction, seqInstruction) {
    var p = addParagraph(label + ' ');
    if (chapterInstruction) {
      fields.push(addField(p, chapterInstruction));
      addText(p, '-');
    }
    fields.push(addField(p, seqInstruction));
    return p;
  }
  // A paragraph with one field, to read its result alone.
  function addLoneField(instruction) {
    var p = addParagraph('');
    fields.push(addField(p, instruction));
    return p;
  }

  addParagraph('First', heading(1));
  var table1 = addCaption('Table', ' STYLEREF "Heading 1" \\\\s', ' SEQ Table \\\\* Arabic \\\\s 1');
  var table2 = addCaption('Table', ' STYLEREF "Heading 1" \\\\s', ' SEQ Table \\\\* Arabic \\\\s 1');
  var figure1 = addCaption('Figure', null, ' SEQ Figure \\\\* Arabic');

  addParagraph('Second', heading(1));
  addParagraph('Part', heading(2));
  var table3 = addCaption('Table', ' STYLEREF "Heading 1" \\\\s', ' SEQ Table \\\\* Arabic \\\\s 1');
  var table4 = addCaption('Table', ' STYLEREF 1 \\\\s', ' SEQ Table \\\\* Arabic \\\\s 1');
  var figure2 = addCaption('Figure', null, ' SEQ Figure \\\\* Arabic');

  var level2 = addLoneField(' STYLEREF "Heading 2" \\\\s');
  var plainNumber = addLoneField(' STYLEREF "Heading 1" \\\\n');
  var numberOnly = addLoneField(' STYLEREF "Heading 1" \\\\n \\\\t');
  var fullNumberOnly = addLoneField(' STYLEREF "Heading 2" \\\\w \\\\t');

  recalculate();
  updateFields(fields);
`);

same('headings keep their numbering text', run("documentText().filter(function (line) { return -1 !== line.indexOf('|'); })"), [
  'Chapter 1.|First',
  'Chapter 2.|Second',
  '2.1.|Part',
]);
same('\\s: captions of the first chapter', run('[textOf(table1), textOf(table2)]'), ['Table 1-1', 'Table 1-2']);
same('\\s: the SEQ with \\s 1 starts again in the second chapter', run('textOf(table3)'), 'Table 2-1');
same('\\s: STYLEREF 1 is Heading 1', run('textOf(table4)'), 'Table 2-2');
same('a SEQ without \\s goes on counting', run('[textOf(figure1), textOf(figure2)]'), ['Figure 1', 'Figure 2']);
same('\\s on a second level keeps the delimiter between the numbers', run('textOf(level2)'), '2.1');
same('\\n alone keeps the text of the level', run('textOf(plainNumber)'), 'Chapter 2');
same('\\n \\t gives the number alone', run('textOf(numberOnly)'), '2');
same('\\w \\t gives the numbers alone', run('textOf(fullNumberOnly)'), '2.1');

// --- Other numberings ---
run = openWordEditor(process.argv[2]);
run(`
  numberHeadings(['Appendix %1'], Asc.c_oAscNumberingFormat.UpperLetter);
  addParagraph('Tables', heading(1));
  var lettered = addParagraph('Table ');
  var letteredField = addField(lettered, ' STYLEREF "Heading 1" \\\\s');
  recalculate();
  updateFields([letteredField]);
`);
same('\\s keeps a number written as a letter', run('textOf(lettered)'), 'Table A');

run = openWordEditor(process.argv[2]);
run(`
  addParagraph('Plain', heading(1));
  var unnumbered = addParagraph('Table ');
  var unnumberedField = addField(unnumbered, ' STYLEREF "Heading 1" \\\\s');
  recalculate();
  updateFields([unnumberedField]);
`);
same('\\s on a heading without numbering gives 0', run('textOf(unnumbered)'), 'Table 0');

finish();
