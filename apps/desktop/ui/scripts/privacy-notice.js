const PRIVACY_NOTICE = {
  id: "self-hosting-2026",
  updatedOn: "",
  url: "https://commy.chat/privacy",
};
const PRIVACY_NOTICE_KEY = "commy.privacyNoticeSeen";
const NEW_ACCOUNT_MS = 24 * 60 * 60 * 1000;

function privacyNoticeTitle(updatedOn, locale) {
  const date = updatedOn ? new Date(`${updatedOn}T12:00:00Z`) : null;
  if (!date || isNaN(date)) return "We updated our Privacy Policy.";
  const shown = date.toLocaleDateString(locale, { month: "long", day: "numeric", year: "numeric", timeZone: "UTC" });
  return `We updated our Privacy Policy on ${shown}.`;
}

function agreedToCurrentPolicy(user, now, updatedOn = PRIVACY_NOTICE.updatedOn) {
  const created = Date.parse((user && user.created_at) || "");
  if (isNaN(created)) return false;
  const updated = Date.parse(updatedOn ? `${updatedOn}T00:00:00Z` : "");
  return now - created < NEW_ACCOUNT_MS || (!isNaN(updated) && created >= updated);
}

function privacyNoticeDue(storage, user, now = Date.now(), id = PRIVACY_NOTICE.id) {
  try {
    if (storage.getItem(PRIVACY_NOTICE_KEY) === id) return false;
  } catch {
    return false;
  }
  if (!agreedToCurrentPolicy(user, now)) return true;
  finishPrivacyNotice(storage, id);
  return false;
}

function finishPrivacyNotice(storage, id = PRIVACY_NOTICE.id) {
  try {
    storage.setItem(PRIVACY_NOTICE_KEY, id);
  } catch {}
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PRIVACY_NOTICE, PRIVACY_NOTICE_KEY, privacyNoticeTitle, agreedToCurrentPolicy, privacyNoticeDue, finishPrivacyNotice };
}
