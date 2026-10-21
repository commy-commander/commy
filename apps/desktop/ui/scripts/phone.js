const PHONE_MAX_WIDTH_PX = 700;
const PHONE_PANE_DEPTH = { list: 0, chat: 1, people: 2 };

function isPhoneWidth(width) {
  return width <= PHONE_MAX_WIDTH_PX;
}

function phonePaneAfter(pane, action) {
  if (action === "open") return "chat";
  if (action === "people") return pane === "people" ? "chat" : "people";
  if (action === "back") return pane === "people" ? "chat" : "list";
  return pane;
}

function phonePaneGoesDeeper(from, to) {
  return (PHONE_PANE_DEPTH[to] ?? 0) > (PHONE_PANE_DEPTH[from] ?? 0);
}

function rowOpensConversation(rowClasses, tappedButton) {
  if (tappedButton) return false;
  const classes = new Set(rowClasses);
  return classes.has("channel") && !classes.has("sidebar-link") && !classes.has("friend-request");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PHONE_MAX_WIDTH_PX, isPhoneWidth, phonePaneAfter, phonePaneGoesDeeper, rowOpensConversation };
}
