const MAX_NICKNAME_CHARS = 32;

function cleanNickname(raw) {
  const text = String(raw || "").trim();
  if (!text) return null;
  const fits = [...text].length <= MAX_NICKNAME_CHARS && !/\p{Cc}/u.test(text);
  return fits ? text : undefined;
}

function nicknameProblem(raw) {
  return cleanNickname(raw) === undefined ? t("ui.nicknames-can-be-up-to-characters", { MAX_NICKNAME_CHARS }) : null;
}

function nicknameAccountLine(member) {
  if (!member || !member.nickname || !member.username) return null;
  return member.username === member.nickname ? null : member.username;
}

function nicknamePrompt(member, server, me) {
  const self = Boolean(me && member && member.id === me);
  const where = server && server.name ? ` in ${server.name}` : "";
  const who = member && member.username ? member.username : t("ui.this-member");
  return self ? t("ui.how-your-name-appears", { where }) : t("ui.how-appears", { who, where });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { MAX_NICKNAME_CHARS, cleanNickname, nicknameProblem, nicknameAccountLine, nicknamePrompt };
}
