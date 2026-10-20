const FRIEND_ONLINE_WINDOW_SECS = 10 * 60;

const ONLINE_PRESENCES = new Set(["online", "idle", "do_not_disturb"]);

function isVisiblyOnline(presence) {
  return ONLINE_PRESENCES.has(presence);
}

function cameOnline(before, after) {
  return before === "offline" && isVisiblyOnline(after);
}

function mergedPresence(memberLists, userId) {
  let found = null;
  for (const members of memberLists) {
    for (const member of members || []) {
      if (member.id !== userId) continue;
      if (isVisiblyOnline(member.presence)) return member.presence;
      found = "offline";
    }
  }
  return found;
}

function wantsFriendOnline(prefs, userId) {
  const picks = (prefs && prefs.friendOnlinePicks) || {};
  if (Object.prototype.hasOwnProperty.call(picks, userId)) return Boolean(picks[userId]);
  return Boolean(prefs && prefs.friendOnline);
}

function withFriendOnlinePick(prefs, userId, on) {
  const picks = { ...((prefs && prefs.friendOnlinePicks) || {}) };
  if (on === null || on === undefined || Boolean(on) === Boolean(prefs && prefs.friendOnline)) delete picks[userId];
  else picks[userId] = Boolean(on);
  const next = { ...prefs };
  if (Object.keys(picks).length) next.friendOnlinePicks = picks;
  else delete next.friendOnlinePicks;
  return next;
}

function createFriendOnlineTracker({ windowSecs = FRIEND_ONLINE_WINDOW_SECS } = {}) {
  return { known: new Map(), lastToast: new Map(), swept: false, windowSecs };
}

function sweepFriendPresence(tracker, { friends, presenceOf, prefs, myPresence, nowSecs }) {
  const quiet = !tracker.swept || myPresence === "do_not_disturb" || myPresence === "invisible";
  const due = [];
  for (const friend of friends || []) {
    const now = presenceOf(friend.id) || "offline";
    const was = tracker.known.get(friend.id);
    tracker.known.set(friend.id, now);
    if (quiet || !cameOnline(was, now) || !wantsFriendOnline(prefs, friend.id)) continue;
    const last = tracker.lastToast.get(friend.id);
    if (last !== undefined && nowSecs - last < tracker.windowSecs) continue;
    tracker.lastToast.set(friend.id, nowSecs);
    due.push(friend);
  }
  tracker.swept = true;
  return due;
}

function friendOnlineNotice(friend) {
  return { title: `${friend.name} is now online`, body: "Click to send a message", target: `dm:${friend.id}` };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    FRIEND_ONLINE_WINDOW_SECS, isVisiblyOnline, cameOnline, mergedPresence, wantsFriendOnline, withFriendOnlinePick,
    createFriendOnlineTracker, sweepFriendPresence, friendOnlineNotice,
  };
}
