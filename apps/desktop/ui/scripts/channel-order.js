const CATEGORY_BASE_POSITION = 1000;

function reorderKeys(keys, moving, target, after) {
  if (!keys.includes(moving) || moving === target) return null;
  const rest = keys.filter(k => k !== moving);
  const at = rest.indexOf(target);
  const index = at < 0 ? (after ? rest.length : 0) : at + (after ? 1 : 0);
  rest.splice(index, 0, moving);
  return rest.every((k, i) => k === keys[i]) ? null : rest;
}

function looseStayFirst(order, looseKeys) {
  return looseKeys.every((k, i) => order[i] === k);
}

function categoryMoves(order, idOf) {
  return order.map((key, i) => ({ channel: idOf(key), parent: null, position: CATEGORY_BASE_POSITION + i }));
}

function sectionChannelMoves(channels, parent) {
  return channels.map((c, i) => ({ channel: c.id, parent, position: i }));
}

function dropLandsAfter(clientY, rect) {
  return clientY > rect.top + rect.height / 2;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { CATEGORY_BASE_POSITION, reorderKeys, looseStayFirst, categoryMoves, sectionChannelMoves, dropLandsAfter };
}
