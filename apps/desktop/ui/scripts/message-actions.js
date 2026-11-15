const QUICK_MESSAGE_ACTIONS = ["react", "reply", "edit", "thread", "save", "pin"];
const QUICK_MESSAGE_ACTION_COUNT = 4;

function splitMessageActions(actions, count = QUICK_MESSAGE_ACTION_COUNT) {
  const list = actions || [];
  const quick = QUICK_MESSAGE_ACTIONS.map(id => list.find(action => action.id === id)).filter(Boolean).slice(0, count);
  return { quick, rest: list.filter(action => !quick.includes(action)) };
}

function messageMenuLabel(action) {
  return action.label || action.title;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { QUICK_MESSAGE_ACTIONS, QUICK_MESSAGE_ACTION_COUNT, splitMessageActions, messageMenuLabel };
}
