// start-screen.js: what the page of the app shows: whether the start screen
// is there and takes clicks, whether something is opening, whether an editor
// is in the page, and the documents offered for recovery. It changes nothing.
function (args, editor, win, need) {
  function element(selector) {
    var found = document.querySelector(selector);
    if (!found) throw new Error("no " + selector + " in the page: this probe was written for another start screen");
    return found;
  }
  function drawn(node) { return window.getComputedStyle(node).display !== "none"; }
  var start = element("#start-screen");
  var opening = element("#opening");
  var open = element("#start-screen button[data-open]");
  var block = element("#recover-block");
  var items = element("#recover-list").querySelectorAll(".recover-item");
  var types = [];
  start.querySelectorAll("button[data-type]").forEach(function (button) { types.push(button.dataset.type); });
  var lines = [
    "start=" + (drawn(start) ? "shown" : "hidden"),
    "start.inert=" + (start.inert === true),
    "opening=" + (opening.classList.contains("active") ? "active" : "idle"),
    "editor=" + (document.querySelector('iframe[name="frameEditor"]') ? "open" : "none"),
    "create=" + types.join(" "),
    "open.button=" + (open.disabled ? "disabled" : "enabled"),
    "recover=" + (drawn(block) ? "shown" : "hidden"),
    "recover.items=" + items.length
  ];
  items.forEach(function (item) {
    lines.push("recover.item=" + item.querySelector(".recover-name").textContent);
  });
  return lines;
}
