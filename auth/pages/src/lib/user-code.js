export function cleanCode(raw) {
  return raw.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 16);
}

export function cleanInviteCode(raw) {
  let text = raw;
  try {
    text = decodeURIComponent(raw);
  } catch {
    return "";
  }
  const value = text.trim();
  if (/^[A-Za-z0-9]{8}$/.test(value)) return value.toUpperCase();
  const name = value.toLowerCase();
  return name.length >= 3 && name.length <= 32 && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(name) ? name : "";
}
