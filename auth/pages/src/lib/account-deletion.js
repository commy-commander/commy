export const DELETE_CODE_LENGTH = 6;
export const DELETE_PAGE = "/delete-account";

export const DELETED_RIGHT_AWAY = [
  "Your account: its email address, display name and picture",
  "Every way you sign in: password, Google and Twitch logins, passkeys and two-step sign-in",
  "Your linked devices and every signed-in session, so each app and browser is signed out",
  "The locked copy of your server list that your devices share",
  "The invite links you made and your Discover listings",
];

export const NOT_DELETED = [
  "Messages, files and reactions you already sent stay with the people and servers that received them, because Commy delivers them straight between devices. Servers you own keep running for their members.",
  "Encrypted copies on the sync service, which only your devices can read and which are not tied to your account, expire within 90 days.",
  "The app you delete from removes its copy of your chats for that account. Other devices keep theirs until you sign out there or clear Commy's data in Settings > Storage.",
];

export function startMode(start) {
  if (start?.confirm === "email") return "code";
  if (start?.password) return "password";
  return "none";
}

export function typedCode(value) {
  return String(value ?? "").replace(/[\s-]/g, "");
}

export function deletionProof(mode, code, password) {
  if (mode === "code") {
    const digits = typedCode(code);
    return new RegExp(`^\\d{${DELETE_CODE_LENGTH}}$`).test(digits) ? { code: digits } : null;
  }
  if (mode === "password") {
    const typed = String(password ?? "");
    return typed ? { password: typed } : null;
  }
  return {};
}
