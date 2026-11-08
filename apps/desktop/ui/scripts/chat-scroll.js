const SCROLL_DOWN_SCREENS = 1.5;

function scrollDownShown(scrollHeight, scrollTop, clientHeight) {
  if (clientHeight <= 0) return false;
  return scrollHeight - scrollTop - clientHeight > clientHeight * SCROLL_DOWN_SCREENS;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { scrollDownShown };
}
