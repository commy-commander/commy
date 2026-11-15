export const APPLY_PAGE = "/servers/apply";
export const REVIEW_PAGE = "/servers/review";

export const PLEDGE = [
  ["Moderate actively", "Someone reads reports and acts on them within 72 hours, under rules that forbid illegal content, harassment and spam."],
  ["Publish an abuse contact", "An email address or page that a person answers within 72 hours."],
  ["Back up daily", "At least 7 days of database backups kept off the server, with a restore tested at least once."],
  ["Name an emergency admin", "A second person with access who can keep the server running, or shut it down cleanly."],
  ["Give 3 months' notice before shutting down", "To the people who use it and to the listing contact."],
  ["Publish your own Terms and privacy page", "Written for your server and linked in its settings."],
  ["Keep the 18+ check on", "For servers that host accounts."],
  ["Stay current", "A supported release, at most one minor release behind, with a valid HTTPS certificate."],
];

export const CHECK_NAMES = {
  address: "Address",
  tls: "TLS certificate",
  discovery: "Discovery",
  version: "Version",
  contact: "Abuse contact",
  legal: "Terms and privacy",
  adults: "18+ check",
};

export function cleanToken(value, max = 1000) {
  const text = String(value ?? "").trim();
  return /^[A-Za-z0-9_.-]+$/.test(text) && text.length <= max ? text : "";
}
