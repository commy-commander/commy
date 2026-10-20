import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { parseMessage, parseInline, stripMarkdown, hasFormatting, highlightCode, normalizeLang, toggleWrap, formatShortcut, composerRows } = require("../scripts/markdown.js");

const types = nodes => nodes.map(n => n.type);
const text = nodes => nodes.map(n => (n.type === "text" ? n.text : `<${n.type}>`)).join("");

test("inline markers wrap bold, italic, underline, strikethrough, spoilers and code", () => {
  const nodes = parseInline("a **b** *c* __d__ ~~e~~ ||f|| `g` _h_");
  assert.deepEqual(types(nodes).filter(t => t !== "text"), ["strong", "em", "underline", "strike", "spoiler", "code", "em"]);
  assert.equal(nodes[1].children[0].text, "b");
  assert.equal(nodes[11].text, "g");
  const both = parseInline("***both***");
  assert.equal(both[0].type, "strong");
  assert.equal(both[0].children[0].type, "em");
  assert.equal(both[0].children[0].children[0].text, "both");
});

test("underscores inside words, urls and custom emoji stay literal", () => {
  assert.equal(text(parseInline("snake_case_name and more_words_here")), "snake_case_name and more_words_here");
  assert.equal(text(parseInline("https://a.b/c_d_e*f*")), "https://a.b/c_d_e*f*");
  assert.equal(text(parseInline("<:ferris_wave:ab@cd> __x__")), "<:ferris_wave:ab@cd> <underline>");
  assert.equal(text(parseInline("2 * 3 * 4")), "2 * 3 * 4");
  assert.equal(text(parseInline("a ** b")), "a ** b");
});

test("backslashes escape markers and unclosed markers stay literal", () => {
  assert.equal(text(parseInline("\\*not italic\\*")), "*not italic*");
  assert.equal(text(parseInline("**unclosed")), "**unclosed");
  assert.equal(text(parseInline("`")), "`");
  assert.equal(text(parseInline("||")), "||");
});

test("inline code shields markers and masked links need an http url and a non-url label", () => {
  const nodes = parseInline("`**raw**` [site](https://commy.chat) [bad](javascript:alert(1)) [https://evil.example](https://commy.chat)");
  assert.equal(nodes[0].type, "code");
  assert.equal(nodes[0].text, "**raw**");
  const link = nodes.find(n => n.type === "link");
  assert.equal(link.url, "https://commy.chat");
  assert.equal(link.children[0].text, "site");
  assert.equal(nodes.filter(n => n.type === "link").length, 1);
  assert.match(text(nodes), /\[bad\]\(javascript:alert\(1\)\)/);
});

test("timestamps parse with a style and default to the short date and time", () => {
  const nodes = parseInline("at <t:1700000000:R> and <t:1700000000> and <t:abc:R>");
  const times = nodes.filter(n => n.type === "time");
  assert.deepEqual(times.map(t => [t.unix, t.style]), [[1700000000, "R"], [1700000000, "f"]]);
  assert.match(text(nodes), /<t:abc:R>/);
});

test("blocks cover headings, lists, quotes, paragraphs with line breaks and gaps", () => {
  const blocks = parseMessage("# Title\n#### not a heading\n- one\n* two\n1. first\n2. second\n> quoted **q**\n> more\n\nafter gap\nsecond line");
  assert.deepEqual(blocks.map(b => b.type), ["heading", "paragraph", "list", "list", "quote", "paragraph"]);
  assert.equal(blocks[0].level, 1);
  assert.equal(blocks[2].ordered, false);
  assert.equal(blocks[2].items.length, 2);
  assert.equal(blocks[3].ordered, true);
  assert.equal(blocks[4].children[0].type, "paragraph");
  assert.equal(blocks[4].children[0].children.some(n => n.type === "br"), true);
  assert.equal(blocks[4].gapAfter, true);
  assert.equal(blocks[5].children.filter(n => n.type === "br").length, 1);
  assert.equal(parseMessage("5. five")[0].start, 5);
});

test("fenced code blocks keep their language and raw text, even on one line or unclosed", () => {
  const [block] = parseMessage("```rust\nfn main() {}\n```");
  assert.deepEqual(block, { type: "codeblock", lang: "rust", code: "fn main() {}" });
  assert.deepEqual(parseMessage("```one **liner**```")[0], { type: "codeblock", lang: "", code: "one **liner**" });
  assert.equal(parseMessage("```js\nlet a")[0].code, "let a");
  const mixed = parseMessage("before\n```\nx\n```\nafter");
  assert.deepEqual(mixed.map(b => b.type), ["paragraph", "codeblock", "paragraph"]);
  assert.equal(parseMessage("a ```not a block``` b")[0].type, "paragraph");
});

test("plain text drops markup for excerpts and notifications", () => {
  assert.equal(stripMarkdown("# Hi\n**bold** [x](https://a.b) ||s||\n```js\nlet a\n```"), "Hi\nbold x s\nlet a");
  assert.equal(stripMarkdown("> q\n\n\n- a\n- b"), "q\na\nb");
  assert.equal(hasFormatting("plain words"), false);
  assert.equal(hasFormatting("some **bold**"), true);
  assert.equal(hasFormatting("line\nline"), true);
});

test("syntax colouring finds keywords, strings, numbers and comments per language", () => {
  const kinds = tokens => tokens.filter(t => t.kind !== "plain").map(t => [t.kind, t.text]);
  assert.deepEqual(kinds(highlightCode('let x = "s"; // c\n0x1F', "js")), [["keyword", "let"], ["string", '"s"'], ["comment", "// c"], ["number", "0x1F"]]);
  assert.deepEqual(kinds(highlightCode("def f(): # hi\n  return 1.5", "python")), [["keyword", "def"], ["comment", "# hi"], ["keyword", "return"], ["number", "1.5"]]);
  assert.deepEqual(kinds(highlightCode("SELECT a -- x", "sql")), [["keyword", "SELECT"], ["comment", "-- x"]]);
  assert.deepEqual(kinds(highlightCode("x = 1 // no comment", "python")), [["number", "1"]]);
  assert.deepEqual(kinds(highlightCode("let a = b1", "nonsense")), []);
  assert.equal(highlightCode("a\nb", "").map(t => t.text).join(""), "a\nb");
  assert.equal(normalizeLang("TypeScript"), "ts");
  assert.equal(normalizeLang("c++"), "c");
});

test("format shortcuts toggle markers around the selection", () => {
  assert.deepEqual(toggleWrap("hello world", 0, 5, "**"), { value: "**hello** world", start: 2, end: 7 });
  assert.deepEqual(toggleWrap("**hello** world", 2, 7, "**"), { value: "hello world", start: 0, end: 5 });
  assert.deepEqual(toggleWrap("**hello** world", 0, 9, "**"), { value: "hello world", start: 0, end: 5 });
  assert.deepEqual(toggleWrap("ab", 1, 1, "*"), { value: "a**b", start: 2, end: 2 });
  assert.equal(formatShortcut({ ctrlKey: true, key: "b" }), "**");
  assert.equal(formatShortcut({ metaKey: true, key: "I" }), "*");
  assert.equal(formatShortcut({ ctrlKey: true, key: "u" }), "__");
  assert.equal(formatShortcut({ ctrlKey: true, shiftKey: true, key: "S" }), "~~");
  assert.equal(formatShortcut({ ctrlKey: true, key: "s" }), null);
  assert.equal(formatShortcut({ ctrlKey: true, shiftKey: true, key: "b" }), null);
  assert.equal(formatShortcut({ key: "b" }), null);
  assert.equal(formatShortcut({ ctrlKey: true, altKey: true, key: "b" }), null);
});

test("the composer grows with its lines up to a cap", () => {
  assert.equal(composerRows(""), 1);
  assert.equal(composerRows("a\nb\nc"), 3);
  assert.equal(composerRows("x\n".repeat(20)), 8);
});
