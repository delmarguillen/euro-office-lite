// fonts.js [U+XXXX | family:NAME]...: the families the editor has, the one it
// takes for each code point the font of the text may lack, and whether each
// family asked for is listed (a + in NAME stands for a space).
function (words, editor, win, need) {
  var infos = need(win, "AscFonts.g_font_infos");
  var picker = need(win, "AscFonts.FontPickerByCharacter");
  need(picker, "getFontBySymbol");
  var names = infos.map(function (info) { return need(info, "Name"); });
  var lines = ["fonts=" + names.length];
  names.forEach(function (name) { lines.push("font=" + name); });
  function pick(word) {
    var code = parseInt(word.replace(/^U\+/i, ""), 16);
    if (isNaN(code)) throw new Error("fonts.js takes U+XXXX or family:NAME, not " + word);
    var family = picker.getFontBySymbol(code);
    var point = "U+" + code.toString(16).toUpperCase();
    lines.push("pick." + point + "=" + (family || "none"));
    lines.push("drawn." + point + "=" + (family ? "yes" : "no"));
  }
  function family(word) {
    var name = word.slice("family:".length).replace(/\+/g, " ");
    lines.push("family." + name + "=" + (names.indexOf(name) < 0 ? "missing" : "listed"));
  }
  words.forEach(function (word) {
    if (word.indexOf("family:") === 0) family(word);
    else pick(word);
  });
  return lines;
}
