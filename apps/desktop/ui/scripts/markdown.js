const MAX_HEADING_LEVEL = 3;
const MAX_LIST_ITEMS = 200;
const TIMESTAMP_STYLES = new Set(["t", "T", "d", "D", "f", "F", "R"]);
const INLINE_MARKERS = ["***", "**", "__", "~~", "||", "*", "_"];
const MARKER_TYPES = { "**": "strong", "__": "underline", "~~": "strike", "||": "spoiler", "*": "em", "_": "em" };
const URL_RE = /^https?:\/\/[^\s<>"']+/i;
const TIME_RE = /^<t:(\d{1,13})(?::([a-zA-Z]))?>/;
const CUSTOM_EMOJI_RE = /^<:[a-z0-9_]{2,32}:[A-Za-z0-9]{1,128}@[A-Za-z0-9]{1,128}>/;
const FENCE = "```";
const TEXT_COLORS = ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "gray"];
const TEXT_FONTS = ["serif", "mono", "hand", "display"];
const TEXT_EFFECTS = ["rainbow", "shimmer", "wave", "glow", "shake"];
const LETTER_EFFECTS = ["rainbow", "shimmer", "wave"];
const MAX_TEXT_STYLES = 3;
const STYLE_HEAD_RE = /^\{([a-z]+(?:\+[a-z]+){0,2}):/;

function isWordChar(ch) {
  return Boolean(ch) && /[\p{L}\p{N}_]/u.test(ch);
}

function textNode(text) {
  return { type: "text", text };
}

function pushText(nodes, text) {
  if (!text) return;
  const last = nodes[nodes.length - 1];
  if (last && last.type === "text") last.text += text;
  else nodes.push(textNode(text));
}

function findCloser(text, marker, from) {
  let depth = 0;
  for (let i = from; i < text.length; i++) {
    const ch = text[i];
    if (ch === "\\") {
      i++;
      continue;
    }
    if (ch === "`") {
      const end = text.indexOf("`", i + 1);
      if (end === -1) return -1;
      i = end;
      continue;
    }
    if (text.startsWith(marker, i)) {
      if (depth === 0) return i;
      depth--;
      i += marker.length - 1;
    }
  }
  return -1;
}

function emphasisAllowed(text, marker, open, close) {
  const inner = text.slice(open + marker.length, close);
  if (!inner.trim()) return false;
  if (marker === "*" || marker === "_" || marker === "***") {
    if (/^\s/.test(inner) || /\s$/.test(inner)) return false;
  }
  if (marker === "_" || marker === "__") {
    if (isWordChar(text[open - 1]) || isWordChar(text[close + marker.length])) return false;
  }
  return true;
}

function tryMarker(text, i, nodes) {
  for (const marker of INLINE_MARKERS) {
    if (!text.startsWith(marker, i)) continue;
    if (marker === "_" && text.startsWith("__", i)) continue;
    if (marker === "*" && text.startsWith("**", i)) continue;
    if (marker.length === 1 && text[i + 1] === marker) continue;
    const close = findCloser(text, marker, i + marker.length);
    if (close === -1 || !emphasisAllowed(text, marker, i, close)) continue;
    const children = parseInline(text.slice(i + marker.length, close));
    if (marker === "***") nodes.push({ type: "strong", children: [{ type: "em", children }] });
    else nodes.push({ type: MARKER_TYPES[marker], children });
    return close + marker.length;
  }
  return -1;
}

function styleCategory(name) {
  if (TEXT_COLORS.includes(name)) return "color";
  if (TEXT_FONTS.includes(name)) return "font";
  if (TEXT_EFFECTS.includes(name)) return "effect";
  return null;
}

function readStyleNames(spec) {
  const names = String(spec || "").split("+");
  if (!names.length || names.length > MAX_TEXT_STYLES || new Set(names).size !== names.length) return null;
  const style = { color: null, font: null, effects: [] };
  for (const name of names) {
    const kind = styleCategory(name);
    if (!kind) return null;
    if (kind === "effect") style.effects.push(name);
    else if (style[kind]) return null;
    else style[kind] = name;
  }
  return style;
}

function findStyleClose(text, from) {
  let depth = 0;
  for (let i = from; i < text.length; i++) {
    const ch = text[i];
    if (ch === "\\") {
      i++;
      continue;
    }
    if (ch === "\n") return -1;
    if (ch === "`") {
      const end = text.indexOf("`", i + 1);
      if (end === -1) continue;
      i = end;
      continue;
    }
    if (ch === "{") depth++;
    if (ch === "}") {
      if (depth === 0) return i;
      depth--;
    }
  }
  return -1;
}

function tryStyle(text, i, nodes) {
  const head = STYLE_HEAD_RE.exec(text.slice(i, i + 40));
  if (!head) return -1;
  const style = readStyleNames(head[1]);
  if (!style) return -1;
  const from = i + head[0].length;
  const close = findStyleClose(text, from);
  if (close === -1 || !text.slice(from, close).trim()) return -1;
  nodes.push({ type: "style", ...style, children: parseInline(text.slice(from, close)) });
  return close + 1;
}

function textStyleClasses(node) {
  const classes = ["tx"];
  if (node.color) classes.push(`tx-${node.color}`);
  if (node.font) classes.push(`tx-font-${node.font}`);
  for (const effect of node.effects || []) classes.push(`tx-${effect}`);
  return classes;
}

function splitsLetters(node) {
  return (node.effects || []).some(effect => LETTER_EFFECTS.includes(effect));
}

function tryMaskedLink(text, i, nodes) {
  if (text[i] !== "[") return -1;
  const close = text.indexOf("](", i + 1);
  if (close === -1) return -1;
  const label = text.slice(i + 1, close);
  if (!label.trim() || label.includes("[") || label.includes("\n")) return -1;
  const end = text.indexOf(")", close + 2);
  if (end === -1) return -1;
  const url = text.slice(close + 2, end).trim();
  if (!URL_RE.test(url) || /\s/.test(url)) return -1;
  if (/https?:\/\/|www\./i.test(label)) return -1;
  nodes.push({ type: "link", url, children: parseInline(label) });
  return end + 1;
}

function tryTimestamp(text, i, nodes) {
  if (!text.startsWith("<t:", i)) return -1;
  const m = TIME_RE.exec(text.slice(i));
  if (!m) return -1;
  const style = m[2] || "f";
  if (!TIMESTAMP_STYLES.has(style)) return -1;
  nodes.push({ type: "time", unix: Number(m[1]), style });
  return i + m[0].length;
}

function tryVerbatim(text, i) {
  const rest = text.slice(i);
  const url = URL_RE.exec(rest);
  if (url) return url[0];
  const emoji = CUSTOM_EMOJI_RE.exec(rest);
  if (emoji) return emoji[0];
  return null;
}

function parseInline(text) {
  const nodes = [];
  let i = 0;
  let plain = "";
  const flush = () => {
    pushText(nodes, plain);
    plain = "";
  };
  while (i < text.length) {
    const ch = text[i];
    if (ch === "\\" && /[\\*_~|`[\]()<>#>{}-]/.test(text[i + 1] || "")) {
      plain += text[i + 1];
      i += 2;
      continue;
    }
    if (ch === "`") {
      const end = text.indexOf("`", i + 1);
      if (end > i + 1) {
        flush();
        nodes.push({ type: "code", text: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    const verbatim = tryVerbatim(text, i);
    if (verbatim) {
      plain += verbatim;
      i += verbatim.length;
      continue;
    }
    if (ch === "<" || ch === "[" || ch === "{" || INLINE_MARKERS.some(m => m[0] === ch)) {
      const scratch = [];
      const next = ch === "<" ? tryTimestamp(text, i, scratch)
        : ch === "[" ? tryMaskedLink(text, i, scratch)
        : ch === "{" ? tryStyle(text, i, scratch)
        : tryMarker(text, i, scratch);
      if (next >= 0) {
        flush();
        nodes.push(...scratch);
        i = next;
        continue;
      }
    }
    plain += ch;
    i++;
  }
  flush();
  return nodes;
}

function fenceAt(text, i) {
  if (!text.startsWith(FENCE, i)) return null;
  const close = text.indexOf(FENCE, i + FENCE.length);
  const bodyEnd = close === -1 ? text.length : close;
  const raw = text.slice(i + FENCE.length, bodyEnd);
  const firstBreak = raw.indexOf("\n");
  const head = firstBreak === -1 ? "" : raw.slice(0, firstBreak);
  const lang = /^[A-Za-z0-9+#.-]{1,20}$/.test(head.trim()) ? head.trim().toLowerCase() : "";
  let code = lang ? raw.slice(firstBreak + 1) : raw;
  if (code.startsWith("\n")) code = code.slice(1);
  if (code.endsWith("\n")) code = code.slice(0, -1);
  const end = close === -1 ? text.length : close + FENCE.length;
  return { node: { type: "codeblock", lang, code }, end };
}

function splitFences(text) {
  const parts = [];
  let i = 0;
  let from = 0;
  let lineStart = true;
  while (i < text.length) {
    if (lineStart || i === 0) {
      const fence = fenceAt(text, i);
      if (fence) {
        if (i > from) parts.push({ text: text.slice(from, i) });
        parts.push({ node: fence.node });
        i = fence.end;
        from = i;
        lineStart = text[i] === "\n" || i >= text.length;
        if (text[i] === "\n") {
          i++;
          from = i;
        }
        continue;
      }
    }
    lineStart = text[i] === "\n";
    i++;
  }
  if (from < text.length) parts.push({ text: text.slice(from) });
  return parts;
}

function listItem(line) {
  const bullet = /^\s{0,3}[-*]\s+(.*)$/.exec(line);
  if (bullet) return { ordered: false, text: bullet[1] };
  const numbered = /^\s{0,3}(\d{1,3})[.)]\s+(.*)$/.exec(line);
  if (numbered) return { ordered: true, start: Number(numbered[1]), text: numbered[2] };
  return null;
}

function parseLines(lines) {
  const blocks = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      const last = blocks[blocks.length - 1];
      if (last) last.gapAfter = true;
      continue;
    }
    const heading = /^(#{1,3})\s+(.+)$/.exec(line);
    if (heading && heading[1].length <= MAX_HEADING_LEVEL) {
      blocks.push({ type: "heading", level: heading[1].length, children: parseInline(heading[2].trim()) });
      i++;
      continue;
    }
    if (/^>\s?/.test(line)) {
      const inner = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        inner.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      blocks.push({ type: "quote", children: parseLines(inner) });
      continue;
    }
    const item = listItem(line);
    if (item) {
      const items = [];
      let ordered = item.ordered;
      let start = item.start || 1;
      while (i < lines.length && items.length < MAX_LIST_ITEMS) {
        const next = listItem(lines[i]);
        if (!next || next.ordered !== ordered) break;
        items.push(parseInline(next.text));
        i++;
      }
      blocks.push({ type: "list", ordered, start, items });
      continue;
    }
    const para = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,3})\s+/.test(lines[i]) && !/^>\s?/.test(lines[i]) && !listItem(lines[i])) {
      para.push(lines[i]);
      i++;
    }
    const children = [];
    para.forEach((text, n) => {
      if (n > 0) children.push({ type: "br" });
      children.push(...parseInline(text));
    });
    blocks.push({ type: "paragraph", children });
  }
  return blocks;
}

function parseMessage(text) {
  const blocks = [];
  for (const part of splitFences(String(text || "").replace(/\r\n?/g, "\n"))) {
    if (part.node) blocks.push(part.node);
    else blocks.push(...parseLines(part.text.split("\n")));
  }
  return blocks;
}

function inlineToText(nodes) {
  let out = "";
  for (const node of nodes || []) {
    if (node.type === "text" || node.type === "code") out += node.text;
    else if (node.type === "br") out += "\n";
    else if (node.type === "time") out += "";
    else if (node.children) out += inlineToText(node.children);
  }
  return out;
}

function blocksToText(blocks) {
  return blocks.map(block => {
    if (block.type === "codeblock") return block.code;
    if (block.type === "list") return block.items.map(inlineToText).join("\n");
    if (block.type === "quote") return blocksToText(block.children);
    return inlineToText(block.children);
  }).join("\n");
}

function stripMarkdown(text) {
  return blocksToText(parseMessage(text)).replace(/\n{2,}/g, "\n").trim();
}

function hasFormatting(text) {
  return parseMessage(text).some(block => block.type !== "paragraph" || block.children.some(n => n.type !== "text"));
}

const KEYWORDS = {
  js: "async await break case catch class const continue debugger default delete do else export extends finally for from function if import in instanceof let new of return static super switch this throw try typeof var void while with yield true false null undefined",
  ts: "abstract any as async await boolean break case catch class const constructor continue declare default delete do else enum export extends false finally for from function if implements import in instanceof interface keyof let namespace never new null number of private protected public readonly return static string super switch this throw true try type typeof undefined unknown var void while yield",
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
  py: "False None True and as assert async await break class continue def del elif else except finally for from global if import in is lambda nonlocal not or pass raise return try while with yield self",
  go: "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var true false nil",
  c: "auto break case char const continue default do double else enum extern float for goto if inline int long register return short signed sizeof static struct switch typedef union unsigned void volatile while true false NULL nullptr class namespace new delete this template typename public private protected virtual override using bool",
  java: "abstract assert boolean break byte case catch char class const continue default do double else enum extends final finally float for if implements import instanceof int interface long native new package private protected public return short static strictfp super switch synchronized this throw throws transient try void volatile while true false null var record",
  cs: "abstract as base bool break byte case catch char checked class const continue decimal default delegate do double else enum event explicit extern false finally fixed float for foreach goto if implicit in int interface internal is lock long namespace new null object operator out override params private protected public readonly ref return sbyte sealed short sizeof stackalloc static string struct switch this throw true try typeof uint ulong unchecked unsafe ushort using var virtual void volatile while async await",
  sh: "if then else elif fi for while do done case esac in function return exit export local echo set unset source true false",
  sql: "select from where insert into values update set delete create table drop alter join inner left right outer on as and or not null primary key index group by order having limit offset distinct union all exists between like in is case when then else end",
  lua: "and break do else elseif end false for function goto if in local nil not or repeat return then true until while",
  rb: "alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield",
  php: "abstract and array as break callable case catch class clone const continue declare default do echo else elseif empty enddeclare endfor endforeach endif endswitch endwhile extends final finally fn for foreach function global if implements include instanceof insteadof interface isset list match namespace new or print private protected public readonly require return static switch throw trait try unset use var while xor yield true false null",
  swift: "associatedtype class deinit enum extension fileprivate func import init inout internal let open operator private protocol public rethrows static struct subscript typealias var break case continue default defer do else fallthrough for guard if in repeat return switch where while as catch false is nil super self Self throw throws true try",
  kotlin: "as break class continue do else false for fun if in interface is null object package return super this throw true try typealias val var when while by catch constructor delegate dynamic field file finally get import init param property receiver set setparam where abstract actual annotation companion const crossinline data enum expect external final infix inline inner internal lateinit noinline open operator out override private protected public reified sealed suspend tailrec vararg",
  css: "important inherit initial unset none auto",
};

const LANG_ALIASES = {
  javascript: "js", jsx: "js", mjs: "js", cjs: "js", node: "js",
  typescript: "ts", tsx: "ts",
  rs: "rust",
  python: "py", py3: "py",
  golang: "go",
  cpp: "c", "c++": "c", cc: "c", h: "c", hpp: "c", cxx: "c",
  csharp: "cs",
  bash: "sh", shell: "sh", zsh: "sh", console: "sh",
  ruby: "rb",
  kt: "kotlin",
  yml: "yaml",
};

const HASH_COMMENT_LANGS = new Set(["py", "sh", "rb", "toml", "yaml", "ini", "dockerfile", "makefile", "r", "perl"]);
const DASH_COMMENT_LANGS = new Set(["sql", "lua", "haskell"]);
const NO_SLASH_COMMENT_LANGS = new Set(["py", "sh", "rb", "toml", "yaml", "ini", "sql", "lua", "html", "xml", "json", "markdown", "md"]);

function normalizeLang(lang) {
  const key = String(lang || "").toLowerCase();
  return LANG_ALIASES[key] || key;
}

function commentAt(code, i, lang) {
  if (lang === "html" || lang === "xml") {
    if (code.startsWith("<!--", i)) {
      const end = code.indexOf("-->", i + 4);
      return end === -1 ? code.length : end + 3;
    }
    return -1;
  }
  if (HASH_COMMENT_LANGS.has(lang) && code[i] === "#") return lineEnd(code, i);
  if (DASH_COMMENT_LANGS.has(lang) && code.startsWith("--", i)) return lineEnd(code, i);
  if (!NO_SLASH_COMMENT_LANGS.has(lang)) {
    if (code.startsWith("//", i)) return lineEnd(code, i);
    if (code.startsWith("/*", i)) {
      const end = code.indexOf("*/", i + 2);
      return end === -1 ? code.length : end + 2;
    }
  }
  return -1;
}

function lineEnd(code, i) {
  const end = code.indexOf("\n", i);
  return end === -1 ? code.length : end;
}

function stringAt(code, i) {
  const quote = code[i];
  if (quote !== '"' && quote !== "'" && quote !== "`") return -1;
  for (let j = i + 1; j < code.length; j++) {
    if (code[j] === "\\") {
      j++;
      continue;
    }
    if (code[j] === quote) return j + 1;
    if (code[j] === "\n" && quote !== "`") return j;
  }
  return code.length;
}

function highlightCode(code, lang) {
  const key = normalizeLang(lang);
  const keywords = new Set((KEYWORDS[key] || "").split(" ").filter(Boolean));
  const tokens = [];
  const push = (kind, text) => {
    if (!text) return;
    const last = tokens[tokens.length - 1];
    if (last && last.kind === kind) last.text += text;
    else tokens.push({ kind, text });
  };
  let i = 0;
  while (i < code.length) {
    const comment = commentAt(code, i, key);
    if (comment > i) {
      push("comment", code.slice(i, comment));
      i = comment;
      continue;
    }
    const str = stringAt(code, i);
    if (str > i) {
      push("string", code.slice(i, str));
      i = str;
      continue;
    }
    const word = /^[\p{L}_$][\p{L}\p{N}_$]*/u.exec(code.slice(i));
    if (word) {
      const lookup = key === "sql" ? word[0].toLowerCase() : word[0];
      push(keywords.has(lookup) ? "keyword" : "plain", word[0]);
      i += word[0].length;
      continue;
    }
    const number = /^(0[xXbBoO][0-9a-fA-F_]+|\d[\d_]*(\.\d+)?([eE][+-]?\d+)?)/.exec(code.slice(i));
    if (number && !isWordChar(code[i - 1])) {
      push("number", number[0]);
      i += number[0].length;
      continue;
    }
    push("plain", code[i]);
    i++;
  }
  return tokens;
}

function toggleWrap(value, start, end, marker) {
  const before = value.slice(0, start);
  const selected = value.slice(start, end);
  const after = value.slice(end);
  const n = marker.length;
  if (before.endsWith(marker) && after.startsWith(marker)) {
    return { value: before.slice(0, -n) + selected + after.slice(n), start: start - n, end: end - n };
  }
  if (selected.startsWith(marker) && selected.endsWith(marker) && selected.length >= 2 * n) {
    const inner = selected.slice(n, -n);
    return { value: before + inner + after, start, end: start + inner.length };
  }
  return { value: before + marker + selected + marker + after, start: start + n, end: end + n };
}

function enclosingStyleTag(value, start, end) {
  const open = /\{([a-z]+(?:\+[a-z]+){0,2}):$/.exec(value.slice(0, start));
  if (!open || !value.slice(end).startsWith("}") || !readStyleNames(open[1])) return null;
  return { index: open.index, names: open[1].split("+") };
}

function activeTextStyles(value, start, end) {
  const tag = enclosingStyleTag(value, start, end);
  return tag ? tag.names : [];
}

function applyTextStyle(value, start, end, name) {
  const kind = styleCategory(name);
  if (!kind) return { value, start, end };
  const before = value.slice(0, start);
  const selected = value.slice(start, end);
  const after = value.slice(end);
  const open = enclosingStyleTag(value, start, end);
  if (open) {
    let names = open.names;
    if (names.includes(name)) names = names.filter(n => n !== name);
    else {
      if (kind !== "effect") names = names.filter(n => styleCategory(n) !== kind);
      names.push(name);
      if (names.length > MAX_TEXT_STYLES) names = names.slice(-MAX_TEXT_STYLES);
    }
    const outer = before.slice(0, open.index);
    if (!names.length) return { value: outer + selected + after.slice(1), start: outer.length, end: outer.length + selected.length };
    const tag = `{${names.join("+")}:`;
    return { value: outer + tag + selected + after, start: outer.length + tag.length, end: outer.length + tag.length + selected.length };
  }
  const tag = `{${name}:`;
  return { value: before + tag + selected + "}" + after, start: start + tag.length, end: end + tag.length };
}

const FORMAT_SHORTCUTS = { b: "**", i: "*", u: "__", s: "~~", e: "`" };

function formatShortcut(event) {
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return null;
  const key = String(event.key || "").toLowerCase();
  if (key === "s" && !event.shiftKey) return null;
  if (key !== "s" && event.shiftKey) return null;
  return FORMAT_SHORTCUTS[key] || null;
}

function composerRows(value, max = 8) {
  const lines = String(value || "").split("\n").length;
  return Math.max(1, Math.min(max, lines));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    parseMessage, parseInline, stripMarkdown, hasFormatting, highlightCode, normalizeLang, toggleWrap, formatShortcut, composerRows, inlineToText, TIMESTAMP_STYLES,
    TEXT_COLORS, TEXT_FONTS, TEXT_EFFECTS, LETTER_EFFECTS, MAX_TEXT_STYLES, readStyleNames, textStyleClasses, splitsLetters, applyTextStyle, activeTextStyles,
  };
}
