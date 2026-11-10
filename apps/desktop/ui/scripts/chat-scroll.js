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

const NEW_LINE_GAP_PX = 8;

function landingScrollTop(scrollHeight, clientHeight, lineTop) {
  const bottom = Math.max(0, scrollHeight - clientHeight);
  if (lineTop == null || lineTop >= bottom) return bottom;
  return Math.max(0, lineTop - NEW_LINE_GAP_PX);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { scrollDownShown, centeredScrollTop, landingScrollTop };
}
