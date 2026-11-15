const MENU_EDGE = 8;

function menuFitHeight(height, viewHeight, edge = MENU_EDGE) {
  const room = viewHeight - edge * 2;
  return height > room ? room : null;
}

function menuSpot({ x, y, flipY = null, width, height, viewWidth, viewHeight, edge = MENU_EDGE }) {
  const floor = viewHeight - edge;
  let top = y;
  if (y + height > floor) top = flipY !== null && flipY - height >= edge ? flipY - height : floor - height;
  const left = Math.min(x, viewWidth - width - edge);
  return { left: Math.max(edge, left), top: Math.max(edge, top) };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MENU_EDGE, menuFitHeight, menuSpot };
}
