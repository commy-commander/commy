const PROFILE_POLICIES = [
  { value: "everyone", label: "Everyone" },
  { value: "shared_servers", label: "Friends and servers we share" },
  { value: "friends", label: "Friends only" },
];

const PROFILE_DETAIL_FIELDS = ["about", "banner", "pronouns", "activity", "activity_cover", "activity_since"];

const FRIEND_NOTE_MAX = 200;

function canSeeProfile(policy, viewer) {
  const { friend = false, sharesServer = false } = viewer || {};
  switch (policy) {
    case "friends": return friend;
    case "shared_servers": return friend || sharesServer;
    default: return true;
  }
}

function publicProfile(person, allowed) {
  if (!person || allowed) return person;
  const bare = { ...person };
  for (const field of PROFILE_DETAIL_FIELDS) delete bare[field];
  return bare;
}

function cleanFriendNote(raw) {
  const text = String(raw || "").replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " ").trim();
  return [...text].slice(0, FRIEND_NOTE_MAX).join("").trim();
}

function friendNoteState(raw) {
  const text = cleanFriendNote(raw);
  const used = [...text].length;
  return { text: text || null, used, counter: `${used}/${FRIEND_NOTE_MAX}`, ok: used <= FRIEND_NOTE_MAX };
}

function friendRequestLine(request) {
  const note = request && request.note ? cleanFriendNote(request.note) : "";
  return note || "Wants to be friends";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PROFILE_POLICIES, PROFILE_DETAIL_FIELDS, FRIEND_NOTE_MAX, canSeeProfile, publicProfile, cleanFriendNote, friendNoteState, friendRequestLine };
}
