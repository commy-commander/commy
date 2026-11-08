const PHONE_MAX_WIDTH_PX = 700;
const PHONE_PANE_DEPTH = { list: 0, chat: 1, people: 2 };
const PHONE_APP_VIEWPORT = "width=device-width, initial-scale=1, minimum-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover, interactive-widget=resizes-content";
const TAP_AFTER_SCROLL_MS = 350;
const TWO_PANE_MIN_WIDTH_PX = 640;
const TWO_PANE_LAYOUT_WIDTH_PX = 704;
const TWO_PANE_VIEWPORT = `width=${TWO_PANE_LAYOUT_WIDTH_PX}, user-scalable=no, viewport-fit=cover, interactive-widget=resizes-content`;

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

function phoneAppViewport(screenWidth, portrait) {
  const twoPane = portrait && screenWidth >= TWO_PANE_MIN_WIDTH_PX && screenWidth <= PHONE_MAX_WIDTH_PX;
  return twoPane ? TWO_PANE_VIEWPORT : PHONE_APP_VIEWPORT;
}

function tapEndsScroll(lastScrollAt, now) {
  return now - lastScrollAt < TAP_AFTER_SCROLL_MS;
}

function headerTapLeavesPeople(pane, buttonId) {
  return pane === "people" && buttonId !== "members-btn";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PHONE_MAX_WIDTH_PX, PHONE_APP_VIEWPORT, TWO_PANE_VIEWPORT, TWO_PANE_LAYOUT_WIDTH_PX, isPhoneWidth, phoneAppViewport, phonePaneAfter, phonePaneGoesDeeper, rowOpensConversation, tapEndsScroll, headerTapLeavesPeople };
}
