const PRIVACY_NOTICE = {
  id: "self-hosting-2026",
  updatedOn: "",
  url: "https://commy.chat/privacy",
};
const PRIVACY_NOTICE_KEY = "commy.privacyNoticeSeen";

function privacyNoticeTitle(updatedOn, locale) {
  const date = updatedOn ? new Date(`${updatedOn}T12:00:00Z`) : null;
  if (!date || isNaN(date)) return "We updated our Privacy Policy.";
  const shown = date.toLocaleDateString(locale, { month: "long", day: "numeric", year: "numeric", timeZone: "UTC" });
  return `We updated our Privacy Policy on ${shown}.`;
}

function privacyNoticeDue(storage, id = PRIVACY_NOTICE.id) {
  try {
    return storage.getItem(PRIVACY_NOTICE_KEY) !== id;
  } catch {
    return false;
  }
}

function finishPrivacyNotice(storage, id = PRIVACY_NOTICE.id) {
  try {
    storage.setItem(PRIVACY_NOTICE_KEY, id);
  } catch {}
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { PRIVACY_NOTICE, PRIVACY_NOTICE_KEY, privacyNoticeTitle, privacyNoticeDue, finishPrivacyNotice };
}
