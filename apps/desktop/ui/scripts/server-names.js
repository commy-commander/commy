const SERVER_NAME_EMOJI_RE = /:([a-z0-9_]{2,32}):/g;

function serverNameCodes(name) {
  return [...String(name || "").matchAll(SERVER_NAME_EMOJI_RE)].filter(m => /[a-z_]/.test(m[1]));
}

function tidyNameParts(parts) {
  const merged = [];
  for (const part of parts) {
    const last = merged[merged.length - 1];
    if (typeof part === "string" && typeof last === "string") merged[merged.length - 1] = last + part;
    else merged.push(part);
  }
  const tidy = merged.map(part => typeof part === "string" ? part.replace(/\s+/g, " ") : part);
  if (typeof tidy[0] === "string") tidy[0] = tidy[0].trimStart();
  if (typeof tidy[tidy.length - 1] === "string") tidy[tidy.length - 1] = tidy[tidy.length - 1].trimEnd();
  return tidy.filter(part => part !== "");
}

function serverNameParts(name, emojis = []) {
  const text = String(name || "");
  const known = new Map((emojis || []).map(emoji => [emoji.name, emoji]));
  const parts = [];
  let last = 0;
  for (const m of serverNameCodes(text)) {
    parts.push(text.slice(last, m.index));
    const emoji = known.get(m[1]);
    if (emoji) parts.push({ emoji });
    last = m.index + m[0].length;
  }
  parts.push(text.slice(last));
  const tidy = tidyNameParts(parts);
  return tidy.length ? tidy : [serverNameWords(text)];
}

function serverNameWords(name) {
  return String(name || "").replace(SERVER_NAME_EMOJI_RE, (whole, code) => /[a-z_]/.test(code) ? code : whole).replace(/\s+/g, " ").trim();
}

function serverShownName(name) {
  const text = String(name || "");
  const words = tidyNameParts([text.replace(SERVER_NAME_EMOJI_RE, (whole, code) => /[a-z_]/.test(code) ? "" : whole)]).join("");
  return words || serverNameWords(text) || text;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { serverNameParts, serverShownName };
}
