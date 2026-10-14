// The privacy policy is the owner's HTML export in content/privacy-policy.html.
// Its words are fixed: these helpers only drop presentation so site styles apply.

const STYLE_BLOCK = /<style\b[^>]*>[\s\S]*?<\/style>/gi;
const SCRIPT_BLOCK = /<script\b[^>]*>[\s\S]*?<\/script>/gi;
const STYLE_ATTR = /\s+style\s*=\s*("[^"]*"|'[^']*')/gi;
const EVENT_ATTR = /\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*')/gi;

/** Policy markup ready to embed: same text, no inline styles, no scripts. */
export function policyHtml(source: string): string {
  return source
    .replace(/\r\n?/g, "\n")
    .replace(STYLE_BLOCK, "")
    .replace(SCRIPT_BLOCK, "")
    .replace(STYLE_ATTR, "")
    .replace(EVENT_ATTR, "");
}

const ENTITIES: Record<string, string> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
  rsquo: "’",
  lsquo: "‘",
  rdquo: "”",
  ldquo: "“",
};

function decode(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, name: string) => {
    if (name[0] === "#") {
      const code = name[1].toLowerCase() === "x" ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
      return String.fromCodePoint(code);
    }
    return ENTITIES[name.toLowerCase()] ?? whole;
  });
}

/** Visible words of an HTML document, whitespace collapsed, for drift checks. */
export function visibleText(html: string): string {
  const text = html
    .replace(STYLE_BLOCK, " ")
    .replace(SCRIPT_BLOCK, " ")
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/<[^>]+>/g, " ");
  return decode(text).replace(/\s+/g, " ").trim();
}

/** Every link target, in document order. */
export function linkTargets(html: string): string[] {
  return Array.from(html.matchAll(/<a\b[^>]*\shref\s*=\s*"([^"]*)"/gi), (m) => m[1]);
}
