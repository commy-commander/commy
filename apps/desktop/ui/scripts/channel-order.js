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

function sectionStep(keys, key, dir) {
  const at = keys.indexOf(key);
  const neighbour = at < 0 ? undefined : keys[at + dir];
  return neighbour === undefined ? null : reorderKeys(keys, key, neighbour, dir > 0);
}

function peerIds(section, channel, voice) {
  return section.channels.filter(c => voice(c.kind) === voice(channel.kind)).map(c => c.id);
}

function stepAcross(sections, from, channel, dir, rules) {
  for (let i = from + dir; i >= 0 && i < sections.length; i += dir) {
    const section = sections[i];
    if (section.fixed || !rules.takes(section, channel)) continue;
    const ids = peerIds(section, channel, rules.voice);
    return { parent: section.parent, ids: dir < 0 ? [...ids, channel.id] : [channel.id, ...ids] };
  }
  return null;
}

function channelStep(sections, channelId, dir, rules) {
  const from = sections.findIndex(s => !s.fixed && s.channels.some(c => c.id === channelId));
  if (from < 0) return null;
  const home = sections[from];
  const channel = home.channels.find(c => c.id === channelId);
  const ids = peerIds(home, channel, rules.voice);
  const at = ids.indexOf(channelId);
  if (!ids[at + dir]) return stepAcross(sections, from, channel, dir, rules);
  ids.splice(at, 1);
  ids.splice(at + dir, 0, channelId);
  return { parent: home.parent, ids };
}

function stepMoves(step) {
  return step.ids.map((id, i) => ({ channel: id, parent: step.parent, position: i }));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { CATEGORY_BASE_POSITION, reorderKeys, looseStayFirst, categoryMoves, sectionChannelMoves, dropLandsAfter, sectionStep, channelStep, stepMoves };
}
