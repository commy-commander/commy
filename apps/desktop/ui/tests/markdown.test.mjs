import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { parseMessage, parseInline, stripMarkdown, hasFormatting, highlightCode, normalizeLang, toggleWrap, formatShortcut, composerRows,
  TEXT_COLORS, TEXT_FONTS, TEXT_EFFECTS, readStyleNames, textStyleClasses, splitsLetters, applyTextStyle, activeTextStyles } = require("../scripts/markdown.js");

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

test("style tags colour, change the font of, or add an effect to text, nesting other formatting", () => {
  const nodes = parseInline("hi {red+wave:big **news**} and {serif:old} {pink+mono+glow:x}");
  const styled = nodes.filter(n => n.type === "style");
  assert.deepEqual(styled.map(n => [n.color, n.font, n.effects]), [["red", null, ["wave"]], [null, "serif", []], ["pink", "mono", ["glow"]]]);
  assert.deepEqual(types(styled[0].children), ["text", "strong"]);
  assert.equal(stripMarkdown("hi {red+wave:big **news**}!"), "hi big news!");
  assert.ok(hasFormatting("{teal:hey}"));
  assert.deepEqual(textStyleClasses(styled[0]), ["tx", "tx-red", "tx-wave"]);
  assert.deepEqual(textStyleClasses(styled[2]), ["tx", "tx-pink", "tx-font-mono", "tx-glow"]);
  assert.ok(splitsLetters(styled[0]));
  assert.ok(!splitsLetters(styled[2]));
  assert.equal(TEXT_COLORS.length + TEXT_FONTS.length + TEXT_EFFECTS.length, 18);
});

test("only known style names make a style tag, everything else stays literal", () => {
  for (const raw of ["{nope:x}", "{red:}", "{red: }", "{red+red:x}", "{red+blue:x}", "{serif+mono:x}", "{red+wave+glow+shake:x}", "{Red:x}", "{red:open", "{red:a\nb}", "a {b} c", "{\"red\": 1}"]) {
    assert.equal(parseInline(raw).some(n => n.type === "style"), false, raw);
  }
  assert.equal(text(parseInline("\\{red:y}")), "{red:y}");
  assert.equal(text(parseInline("`{red:x}`")), "<code>");
  assert.equal(text(parseInline("{red:a {b} c}")), "<style>");
  assert.equal(parseInline("{red:a {b} c}")[0].children[0].text, "a {b} c");
  assert.equal(text(parseInline("{red:`}` here}")), "<style>");
  assert.equal(readStyleNames("blue+serif+rainbow").effects[0], "rainbow");
  assert.equal(readStyleNames("blue+serif+rainbow+glow"), null);
});

test("the style picker wraps the selection and toggles names on a tag it already made", () => {
  const wrapped = applyTextStyle("say hello now", 4, 9, "red");
  assert.deepEqual(wrapped, { value: "say {red:hello} now", start: 9, end: 14 });
  const wave = applyTextStyle(wrapped.value, wrapped.start, wrapped.end, "wave");
  assert.deepEqual(wave, { value: "say {red+wave:hello} now", start: 14, end: 19 });
  const blue = applyTextStyle(wave.value, wave.start, wave.end, "blue");
  assert.equal(blue.value, "say {wave+blue:hello} now");
  const font = applyTextStyle(blue.value, blue.start, blue.end, "mono");
  const swapped = applyTextStyle(font.value, font.start, font.end, "hand");
  assert.equal(swapped.value, "say {wave+blue+hand:hello} now");
  const capped = applyTextStyle(swapped.value, swapped.start, swapped.end, "glow");
  assert.equal(capped.value, "say {blue+hand+glow:hello} now");
  let off = capped;
  for (const name of ["blue", "hand", "glow"]) off = applyTextStyle(off.value, off.start, off.end, name);
  assert.deepEqual(off, { value: "say hello now", start: 4, end: 9 });
  assert.deepEqual(applyTextStyle("", 0, 0, "teal"), { value: "{teal:}", start: 6, end: 6 });
  assert.deepEqual(applyTextStyle("abc", 0, 3, "bogus"), { value: "abc", start: 0, end: 3 });
  assert.deepEqual(activeTextStyles(wave.value, wave.start, wave.end), ["red", "wave"]);
  assert.deepEqual(activeTextStyles("say hello", 4, 9), []);
});
