// sheet-cells.js REF...: what the spreadsheet editor holds for each cell REF
// of the active sheet (text, style, font, format, fill, each border side) and
// for its column (width). Selecting each cell is the only thing it changes.
function (refs, editor, win, need) {
  var lines = [];
  var sheet = need(editor, "wb").getWorksheet();
  need(sheet, "getColumnWidth");
  var styleNames = {};
  var styles = need(win, "Asc.c_oAscBorderStyles");
  Object.keys(styles).forEach(function (name) { styleNames[styles[name]] = name; });
  function hex(n) { return ("000000" + (n >>> 0).toString(16).toUpperCase()).slice(-6); }
  // A side with no line has the style None, which is 0.
  function side(border, name) {
    var line = need(border, name);
    if (!line || !need(line, "s")) return "none";
    var colour = line.c && line.c.getRgb ? hex(line.c.getRgb()) : "auto";
    return (styleNames[line.s] || line.s) + " " + colour;
  }
  function letters(ref) { return ref.replace(/[0-9]/g, ""); }
  function columnIndex(ref) {
    var name = letters(ref), index = 0, i;
    for (i = 0; i < name.length; i++) index = index * 26 + name.charCodeAt(i) - 64;
    return index - 1;
  }
  function fontOf(xfs) {
    var font = xfs.asc_getFontName() + " " + xfs.asc_getFontSize();
    if (xfs.asc_getFontBold()) font += " bold";
    if (xfs.asc_getFontItalic()) font += " italic";
    if (xfs.asc_getFontUnderline()) font += " underline";
    return font;
  }
  function fillOf(xfs) {
    var fill = need(xfs, "getFill").call(xfs);
    if (!fill || !fill.hasFill()) return "none";
    var colour = xfs.asc_getFillColor();
    return hex(colour.asc_getR() * 65536 + colour.asc_getG() * 256 + colour.asc_getB());
  }
  refs.forEach(function (ref) {
    editor.asc_findCell(ref);
    var info = editor.asc_getCellInfo();
    var xfs = info.asc_getXfs();
    // A cell without borders has the default border, with every side empty.
    var border = need(xfs, "getBorder").call(xfs) || need(win, "AscCommonExcel.g_oDefaultFormat.Border");
    lines.push(ref + ".text=" + info.asc_getText());
    lines.push(ref + ".style=" + info.asc_getStyleName());
    lines.push(ref + ".font=" + fontOf(xfs));
    lines.push(ref + ".format=" + xfs.asc_getNumFormat());
    lines.push(ref + ".fill=" + fillOf(xfs));
    lines.push(ref + ".border.left=" + side(border, "l"));
    lines.push(ref + ".border.top=" + side(border, "t"));
    lines.push(ref + ".border.right=" + side(border, "r"));
    lines.push(ref + ".border.bottom=" + side(border, "b"));
    lines.push("col." + letters(ref) + ".px=" + sheet.getColumnWidth(columnIndex(ref), 0));
    lines.push("col." + letters(ref) + ".mm=" + sheet.getColumnWidth(columnIndex(ref), 3).toFixed(2));
  });
  return lines;
}
