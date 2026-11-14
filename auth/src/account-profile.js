export const NAME_MAX_CHARS = 64;
export const IMAGE_URL_MAX = 2048;
const COMBINING_MARKS_KEPT_PER_LETTER = 3;

const ZERO_WIDTH_JOINER = 0x200d;
const HANGUL_FILLERS = new Set([0x115f, 0x1160, 0x3164, 0xffa0]);
const HIDDEN_OR_CONTROL = /[\p{Cc}\p{Cf}\p{Co}\p{Cs}]/u;
const WHITESPACE_RUN = /[\s\p{Z}]+/gu;
const COMBINING_MARK = /\p{M}/u;
const graphemes = new Intl.Segmenter("en", { granularity: "grapheme" });

const isEmojiTag = (codePoint) => codePoint >= 0xe0020 && codePoint <= 0xe007f;

function isHidden(char) {
  const codePoint = char.codePointAt(0);
  if (codePoint === ZERO_WIDTH_JOINER || isEmojiTag(codePoint)) return false;
  return HANGUL_FILLERS.has(codePoint) || HIDDEN_OR_CONTROL.test(char);
}

function withoutHiddenCharacters(text) {
  return [...text].filter((char) => !isHidden(char)).join("");
}

function withoutStackedMarks(text) {
  let marksInARow = 0;
  let out = "";
  for (const char of text) {
    marksInARow = COMBINING_MARK.test(char) ? marksInARow + 1 : 0;
    if (marksInARow <= COMBINING_MARKS_KEPT_PER_LETTER) out += char;
  }
  return out;
}

function clippedToWholeCharacters(text, maxChars) {
  let out = "";
  let used = 0;
  for (const { segment } of graphemes.segment(text)) {
    const size = [...segment].length;
    if (used + size > maxChars) break;
    out += segment;
    used += size;
  }
  return out;
}

export function cleanAccountName(value) {
  if (typeof value !== "string") return "";
  const spaced = value.normalize("NFC").replace(WHITESPACE_RUN, " ");
  const visible = withoutStackedMarks(withoutHiddenCharacters(spaced)).replace(WHITESPACE_RUN, " ").trim();
  return clippedToWholeCharacters(visible, NAME_MAX_CHARS).trim();
}

export function cleanAccountImage(value) {
  if (typeof value !== "string" || value.length === 0 || value.length > IMAGE_URL_MAX) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" && !url.username && !url.password ? url.href : null;
  } catch {
    return null;
  }
}

export function cleanAccountProfile(data) {
  const cleaned = { ...data };
  if ("name" in cleaned) cleaned.name = cleanAccountName(cleaned.name);
  if ("image" in cleaned && cleaned.image != null) cleaned.image = cleanAccountImage(cleaned.image);
  return cleaned;
}
