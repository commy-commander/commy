const DELETE_CODE_LENGTH = 6;

function typedCode(value) {
  return String(value || "").replace(/[\s-]/g, "");
}

function deleteConfirmView(start) {
  const answer = start || {};
  const byEmail = answer.confirm === "email";
  const hasPassword = Boolean(answer.password);
  const mode = byEmail ? "code" : hasPassword ? "password" : "none";
  return {
    mode,
    canUsePassword: byEmail && hasPassword,
    sentLine: byEmail
      ? t("ui.we-emailed-a-digit-code-to-it-expires-in", { DELETE_CODE_LENGTH, value: answer.email || "your address" })
      : mode === "password"
        ? t("ui.enter-your-password-to-confirm")
        : t("ui.your-account-server-does-not-send-email"),
  };
}

function deleteProof(mode, code, password) {
  if (mode === "code") {
    const digits = typedCode(code);
    return new RegExp(`^\\d{${DELETE_CODE_LENGTH}}$`).test(digits) ? { code: digits, password: null } : null;
  }
  if (mode === "password") return password ? { code: null, password } : null;
  if (mode === "none") return { code: null, password: null };
  return null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { DELETE_CODE_LENGTH, typedCode, deleteConfirmView, deleteProof };
}
