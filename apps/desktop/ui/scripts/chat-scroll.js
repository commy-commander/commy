const SCROLL_DOWN_SCREENS = 1.5;

function scrollDownShown(scrollHeight, scrollTop, clientHeight) {
  if (clientHeight <= 0) return false;
  return scrollHeight - scrollTop - clientHeight > clientHeight * SCROLL_DOWN_SCREENS;
}

function centeredScrollTop(rowTop, rowHeight, clientHeight, scrollHeight) {
  const ideal = rowTop - (clientHeight - rowHeight) / 2;
  const furthest = Math.max(0, scrollHeight - clientHeight);
  return Math.round(Math.min(Math.max(ideal, 0), furthest));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { scrollDownShown, centeredScrollTop };
}
