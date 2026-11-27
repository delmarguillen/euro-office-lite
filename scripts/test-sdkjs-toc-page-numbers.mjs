// Checks the page numbers a table of contents gets in src/sdkjs with the sdkjs
// patches applied: an entry on a page of a section numbered in roman numerals
// or letters (w:pgNumType w:fmt) shows that page as its PAGE field does, and
// not as a decimal number.
//
// The document has the sections of a report with front matter:
//   1. a title page, with no page number settings;
//   2. a preface, lowerRoman with no w:start, so it goes on counting: page ii;
//   3. the contents, lowerRoman with w:start="1": pages i and ii;
//   4. the body, decimal with w:start="1": pages 1 and 2.
// The table is the field TOC \o "1-9" \h, updated whole and then only its page
// numbers, the two choices the editor offers.
//
// Run: node scripts/apply-patches.mjs apply --only sdkjs
//      node scripts/test-sdkjs-toc-page-numbers.mjs [path/to/sdkjs]
//      node scripts/apply-patches.mjs revert --only sdkjs
import { makeReport, openWordEditor } from './sdkjs-word-test-harness.mjs';

const { same, finish } = makeReport();
const run = openWordEditor(process.argv[2]);

run(`
  var Format = Asc.c_oAscNumberingFormat;
  numberHeadings(['Chapter %1.', '%1.%2.']);

  endSection(addParagraph('Report'), Format.Decimal);

  addParagraph('Preface', null, 0);
  var prefaceSection = endSection(addParagraph(''), Format.LowerRoman);

  addParagraph('Contents', null, 0);
  var toc = addField(addParagraph(''), 'TOC \\\\o "1-9" \\\\h');
  var figures = addParagraph('Figures', null, 0);
  figures.Set_PageBreakBefore(true);
  var contentsSection = endSection(addParagraph(''), Format.LowerRoman, 1);

  addParagraph('First', heading(1));
  var second = addParagraph('Second', heading(1));
  second.Set_PageBreakBefore(true);
  setPageNumbers(doc.GetFinalSectPr(), Format.Decimal, 1);

  recalculate();

  // As the editor's "Update entire table" and "Update page numbers only".
  function updateTable(pageNumbersOnly) {
    Asc.asc_docs_api.prototype.asc_UpdateTableOfContents.call(AscTest.Editor, pageNumbersOnly, toc);
    recalculate();
  }
  // The entries: the paragraphs from the one the field begins in to the one it ends in.
  function tocParagraphs() {
    toc.SelectField();
    var paragraphs = doc.GetCurrentParagraph(false, true);
    doc.RemoveSelection();
    return paragraphs;
  }
  function entries() {
    return tocParagraphs().map(textOf).filter(function (line) { return '' !== line; });
  }
  // What each PAGEREF of the table calculates, without writing it to the document.
  function pageRefValues() {
    var values = [];
    tocParagraphs().forEach(function (p) {
      p.GetComplexFieldsArrayByType(AscWord.fieldtype_PAGEREF).forEach(function (field) { values.push(field.CalculateValue()); });
    });
    return values;
  }
`);

same('the document has six pages', run('doc.GetPagesCount()'), 6);

run('updateTable(false);');
same('whole table: roman pages in roman, body pages in decimal', run('entries()'), [
  'Preface|ii',
  'Contents|i',
  'Figures|ii',
  'Chapter 1.|First|1',
  'Chapter 2.|Second|2',
]);
same('whole table: each PAGEREF calculates what the entry shows', run('pageRefValues()'), ['ii', 'i', 'ii', '1', '2']);

// A page more before "Figures": only the page numbers are updated.
run(`
  var extra = AscTest.CreateParagraph();
  doc.AddToContent(figures.GetIndex(), extra);
  addText(extra, 'Tables');
  extra.Set_PageBreakBefore(true);
  recalculate();
  updateTable(true);
`);
same('page numbers only: the entry that moved is iii', run('entries()'), [
  'Preface|ii',
  'Contents|i',
  'Figures|iii',
  'Chapter 1.|First|1',
  'Chapter 2.|Second|2',
]);

// Other formats of the section.
run('contentsSection.SetPageNumFormat(Format.UpperRoman); recalculate(); updateTable(true);');
same('upperRoman section', run('entries().slice(0, 3)'), ['Preface|ii', 'Contents|I', 'Figures|III']);

run('contentsSection.SetPageNumFormat(Format.LowerLetter); prefaceSection.SetPageNumFormat(Format.UpperLetter); recalculate(); updateTable(false);');
same('letter sections', run('entries().slice(0, 3)'), ['Preface|B', 'Contents|a', 'Figures|c']);

run('contentsSection.SetPageNumFormat(Format.Decimal); prefaceSection.SetPageNumFormat(Format.Decimal); recalculate(); updateTable(false);');
same('decimal sections stay decimal', run('entries()'), [
  'Preface|2',
  'Contents|1',
  'Figures|3',
  'Chapter 1.|First|1',
  'Chapter 2.|Second|2',
]);

finish();
