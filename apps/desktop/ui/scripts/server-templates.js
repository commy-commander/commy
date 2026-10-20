const TEMPLATE_CODE_PREFIX = "commy-template:";

function looksLikeTemplateCode(text) {
  return typeof text === "string" && text.trimStart().startsWith(TEMPLATE_CODE_PREFIX);
}

function templateGroups(template) {
  const channels = template.channels || [];
  const looseText = { category: "Text Channels", channels: [] };
  const looseVoice = { category: "Voice Channels", channels: [] };
  const groups = [looseText, looseVoice];
  const byIndex = new Map();
  channels.forEach((channel, index) => {
    if (channel.kind !== "category") return;
    const group = { category: channel.name, channels: [] };
    byIndex.set(index, group);
    groups.push(group);
  });
  for (const channel of channels) {
    if (channel.kind === "category") continue;
    const group = byIndex.get(channel.category) || (channel.kind === "voice" || channel.kind === "stage" ? looseVoice : looseText);
    group.channels.push({ name: channel.name, kind: channel.kind });
  }
  return groups.filter(g => g.channels.length);
}

function plural(count, word) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

function templateSummary(template) {
  const channels = (template.channels || []).filter(c => c.kind !== "category");
  const voice = channels.filter(c => c.kind === "voice" || c.kind === "stage").length;
  const parts = [plural(channels.length - voice, "text channel"), plural(voice, "voice channel")];
  const roles = (template.roles || []).length;
  if (roles) parts.push(plural(roles, "role"));
  if (template.has_rules) parts.push("rules");
  return parts.join(", ");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { looksLikeTemplateCode, templateGroups, templateSummary, TEMPLATE_CODE_PREFIX };
}
