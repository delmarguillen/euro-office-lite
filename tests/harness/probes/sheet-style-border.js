// sheet-style-border.js NAME SIDE STYLE RRGGBB: for a `do`. Gives one side
// of the cell style NAME a line style and a colour and keeps the others, as
// the cell style dialog does; fails where the editor has no asc_setBorderSide.
function (args, editor, win, need) {
  var name = args[0], side = String(args[1] || ""), style = args[2], colour = String(args[3] || "");
  var xfs = need(editor, "asc_getCellStyleXfs").call(editor, name);
  if (!xfs) throw new Error("no cell style " + name);
  need(xfs, "asc_setBorderSide");
  var sides = need(win, "Asc.c_oAscBorderOptions");
  var sideName = side.charAt(0).toUpperCase() + side.slice(1).toLowerCase();
  if (!(sideName in sides)) throw new Error("no border side " + side);
  var styles = need(win, "Asc.c_oAscBorderStyles");
  if (!(style in styles)) throw new Error("no border style " + style);
  if (!/^[0-9A-Fa-f]{6}$/.test(colour)) throw new Error("a colour is RRGGBB, not " + colour);
  var Colour = need(win, "Asc.asc_CColor");
  var rgb = parseInt(colour, 16);
  xfs.asc_setBorderSide(sides[sideName], styles[style], new Colour(rgb >> 16, (rgb >> 8) & 255, rgb & 255));
  var error = need(editor, "asc_modifyCellStyle").call(editor, name, xfs);
  if (error) throw new Error("asc_modifyCellStyle refused the change: " + error);
  return "OK " + side + " border of " + name + " set to " + style + " " + colour.toUpperCase();
}
