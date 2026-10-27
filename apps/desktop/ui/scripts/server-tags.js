const SERVER_TAG_COLORS = ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "gray"];
const SERVER_TAG_EFFECTS = [
  { value: "none", label: "None" },
  { value: "shine", label: "Shine" },
  { value: "glow", label: "Glow" },
  { value: "pulse", label: "Pulse" },
];
const MAX_SERVER_TAG_LETTERS = 4;
const NO_SERVER_TAG = "";

function cleanTagLetters(raw) {
  return Array.from(String(raw || "").toUpperCase().replace(/[^\p{L}\p{N}]/gu, "")).slice(0, MAX_SERVER_TAG_LETTERS).join("");
}

function serverTagTitle(tag) {
  return tag && tag.server ? `${tag.server} server tag` : "Server tag";
}

function wearableTags(servers) {
  return (servers || [])
    .filter(s => s && s.tag && !s.group)
    .map(s => ({ value: s.id, label: `${s.name} (${s.tag.letters})` }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

function wornServerId(profile) {
  return (profile && profile.server_tag && profile.server_tag.server) || NO_SERVER_TAG;
}

function serverTagDraft(tag) {
  return {
    letters: (tag && tag.letters) || "",
    emoji: (tag && tag.emoji) || "",
    color: tag && SERVER_TAG_COLORS.includes(tag.color) ? tag.color : "blue",
    effect: tag && SERVER_TAG_EFFECTS.some(e => e.value === tag.effect) ? tag.effect : "none",
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SERVER_TAG_COLORS, SERVER_TAG_EFFECTS, MAX_SERVER_TAG_LETTERS, NO_SERVER_TAG,
    cleanTagLetters, serverTagTitle, wearableTags, wornServerId, serverTagDraft,
  };
}
