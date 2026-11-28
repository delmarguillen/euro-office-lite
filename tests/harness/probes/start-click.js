// start-click.js recover|discard: for a `do_after_restart`. Presses that
// button of the first document the start screen offers to recover. It answers
// first and clicks after: Recover loads the page again.
function (args, editor, win, need) {
  var action = String(args[0] || "").toLowerCase();
  if (action !== "recover" && action !== "discard") throw new Error("start-click.js takes recover or discard, not " + args[0]);
  var start = document.querySelector("#start-screen");
  if (!start || window.getComputedStyle(start).display === "none") throw new Error("the start screen is not shown");
  var item = document.querySelector("#recover-list .recover-item");
  if (!item) throw new Error("the start screen offers no document to recover");
  var button = item.querySelector('button[data-i18n="' + action + '"]');
  if (!button) throw new Error("no " + action + " button: this probe was written for another start screen");
  var name = item.querySelector(".recover-name");
  window.setTimeout(function () { button.click(); }, 300);
  return "OK " + action + " of " + (name ? name.textContent : "a document") + " pressed";
}
