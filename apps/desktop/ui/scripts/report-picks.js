const REPORT_MAX_PICKS = 10;
const REPORT_HOME_OPERATOR = "Commy.chat";

function reportCandidates(messages, userId, chosenId) {
  const theirs = (messages || []).filter(m => m && m.author_id === userId && (m.content || m.id === chosenId));
  const recent = theirs.slice(-REPORT_MAX_PICKS);
  const chosen = theirs.find(m => m.id === chosenId);
  if (chosen && !recent.includes(chosen)) recent.splice(0, 1, chosen);
  return recent.map(m => ({ id: m.id, content: m.content || "", checked: m.id === chosenId }));
}

function reportPicked(candidates) {
  return candidates.filter(c => c.checked).map(c => c.id).slice(0, REPORT_MAX_PICKS);
}

function teamTakesReport(server, meId, reportedId) {
  if (!server) return false;
  const takers = Array.isArray(server.report_takers) ? server.report_takers : [server.owner];
  return takers.some(id => id && id !== meId && id !== reportedId);
}

function canTakeReports(server, meId) {
  if (!server) return false;
  return typeof server.can_take_reports === "boolean" ? server.can_take_reports : Boolean(meId && server.owner === meId);
}

function reportDestination(server, ownerReportable) {
  const host = (server && server.sync_host) || REPORT_HOME_OPERATOR;
  const operator = t("ui.the-people-who-run-where-this-server-s", { host });
  return ownerReportable
    ? t("ui.your-report-goes-privately-to-the-server-2", { operator })
    : t("ui.your-report-goes-privately-to-they-will", { operator });
}

function reportSentToast(sent) {
  const host = sent && sent.host;
  const contact = host && host.contact ? t("ui.contact-them-at", { contact: host.contact }) : "";
  if (host && sent.owner) return t("ui.report-sent-to-the-server-owner-and", { operator: host.operator, contact });
  if (host) return t("ui.report-sent-to", { operator: host.operator, contact });
  if (sent && sent.owner) return t("ui.report-sent-to-the-server-owner-the-sync");
  return t("ui.report-sent");
}

if (typeof module !== "undefined") {
  module.exports = { REPORT_MAX_PICKS, reportCandidates, reportPicked, reportDestination, reportSentToast, teamTakesReport, canTakeReports };
}
