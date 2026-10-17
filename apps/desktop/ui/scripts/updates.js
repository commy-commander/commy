function agoWords(secs, nowMs = Date.now()) {
  const mins = Math.max(0, Math.floor((nowMs / 1000 - Number(secs || 0)) / 60));
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} minute${mins === 1 ? "" : "s"} ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.floor(hours / 24);
  return `${days} day${days === 1 ? "" : "s"} ago`;
}

function updateStatusText(status, nowMs = Date.now()) {
  const s = status || {};
  const checked = s.checked_at ? ` Checked ${agoWords(s.checked_at, nowMs)}.` : "";
  switch (s.phase) {
    case "checking": return "Looking for a new version...";
    case "downloading": return `Downloading Commy ${s.version}${s.progress != null ? `, ${s.progress}%` : ""}...`;
    case "ready": return `Commy ${s.version} is ready. Restart to finish updating.`;
    case "available":
      return s.reason === "portable"
        ? `Commy ${s.version} is out. This copy runs without the installer, so download the new version yourself.`
        : `Commy ${s.version} is out. Update it the way you installed Commy.`;
    case "up_to_date": return `You have the latest version.${checked}`;
    case "failed": return `${s.error || "The check did not finish."}${checked}`;
  }
  if (s.reason === "dev") return "Development builds do not update themselves.";
  if (s.reason === "portable") return "This copy runs without the installer, so it cannot update itself. Check for Updates tells you when a new version is out.";
  if (s.reason === "package") return "Installed from a package, so updates come through your package manager.";
  return "Commy has not looked for updates yet.";
}

function updateAction(status) {
  const s = status || {};
  if (s.reason === "dev") return { label: "Check for Updates", action: "check", disabled: true };
  switch (s.phase) {
    case "checking": return { label: "Checking...", action: "check", disabled: true };
    case "downloading": return { label: "Downloading...", action: "check", disabled: true };
    case "ready": return { label: "Restart Now", action: "restart", disabled: false };
    case "available": return { label: "Download Page", action: "page", disabled: !s.page };
  }
  return { label: "Check for Updates", action: "check", disabled: false };
}

if (typeof module !== "undefined") module.exports = { agoWords, updateStatusText, updateAction };
