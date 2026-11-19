function agoWords(secs, nowMs = Date.now()) {
  const mins = Math.max(0, Math.floor((nowMs / 1000 - Number(secs || 0)) / 60));
  if (mins < 1) return t("ui.just-now");
  if (mins < 60) return t(mins === 1 ? "ui.n.minutes-ago.one" : "ui.n.minutes-ago.many", { count: mins });
  const hours = Math.floor(mins / 60);
  if (hours < 24) return t(hours === 1 ? "ui.n.hours-ago.one" : "ui.n.hours-ago.many", { count: hours });
  const days = Math.floor(hours / 24);
  return t(days === 1 ? "ui.n.days-ago.one" : "ui.n.days-ago.many", { count: days });
}

function updateStatusText(status, nowMs = Date.now()) {
  const s = status || {};
  const checked = s.checked_at ? t("ui.checked", { value: agoWords(s.checked_at, nowMs) }) : "";
  switch (s.phase) {
    case "checking": return t("ui.looking-for-a-new-version");
    case "downloading": return t("ui.downloading-commy", { version: s.version, value: s.progress != null ? `, ${s.progress}%` : "" });
    case "ready": return t("ui.commy-is-ready-restart-to-finish", { version: s.version });
    case "available":
      if (s.reason === "portable") return t("ui.commy-is-out-this-copy-runs-without-the", { version: s.version });
      if (s.can_install) return t("ui.commy-is-out-automatic-updates-are-off", { version: s.version });
      return t("ui.commy-is-out-update-it-the-way-you", { version: s.version });
    case "up_to_date": return t("ui.you-have-the-latest-version", { checked });
    case "failed": return `${s.error || "The check did not finish."}${checked}`;
  }
  if (s.reason === "dev") return t("ui.development-builds-do-not-update");
  if (s.reason === "portable") return t("ui.this-copy-runs-without-the-installer-so");
  if (s.reason === "package") return t("ui.installed-from-a-package-so-updates-come");
  return t("ui.commy-has-not-looked-for-updates-yet");
}

function updateAction(status) {
  const s = status || {};
  if (s.reason === "dev") return { label: t("ui.check-for-updates"), action: "check", disabled: true };
  switch (s.phase) {
    case "checking": return { label: t("ui.checking"), action: "check", disabled: true };
    case "downloading": return { label: t("ui.downloading"), action: "check", disabled: true };
    case "ready": return { label: t("ui.restart-now"), action: "restart", disabled: false };
    case "available":
      if (s.can_install) return { label: t("ui.update-now"), action: "check", disabled: false };
      return { label: t("ui.download-page"), action: "page", disabled: !s.page };
  }
  return { label: t("ui.check-for-updates"), action: "check", disabled: false };
}

const OUTDATED_SKIPS = new Set(["dev", "store"]);

function outdatedNotice(status) {
  const s = status || {};
  if (s.phase !== "available" || !s.version || OUTDATED_SKIPS.has(s.reason)) return null;
  return t("ui.commy-is-out-until-you-update-you-may", { version: s.version });
}

function readyNotice(status) {
  const s = status || {};
  if (s.phase !== "ready" || !s.version) return null;
  return t("ui.commy-is-ready-restart-to-update", { version: s.version });
}

if (typeof module !== "undefined") module.exports = { agoWords, updateStatusText, updateAction, outdatedNotice, readyNotice };
