import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const {
  COMMAND_MIME, MAX_SLASH_COMMANDS, MAX_SLASH_NAME_CHARS, MAX_SLASH_DESCRIPTION_CHARS, MAX_SLASH_OPTIONS,
  validSlashName, slashCatalog, slashNames, slashSuggestions, matchSlashCommand, splitSlashArgs, slashUsage, slashOptionAt, slashHint, parseSlashOptions, slashChips,
} = require("../scripts/slash-commands.js");

const repo = join(import.meta.dirname, "..", "..", "..", "..");
const core = readFileSync(join(repo, "crates", "commy-core", "src", "command.rs"), "utf8");

const server = {
  members: [{ id: "m1", name: "ferris", username: "ferris" }, { id: "m2", name: "Nova", username: "nova" }, { id: "m4", name: "Tidy", bot: true }],
  channels: [{ id: "c1", name: "general", kind: "text" }, { id: "c2", name: "Lounge", kind: "voice" }],
  commands: [
    { bot: "m4", bot_name: "Tidy", commands: [
      { name: "roll", description: "Roll some dice", options: [{ name: "sides", description: "How many sides", required: false, kind: "integer" }] },
      { name: "remind", description: "Remind a member", options: [
        { name: "who", description: "Who to remind", required: true, kind: "user" },
        { name: "minutes", description: "In how many minutes", required: true, kind: "integer" },
        { name: "note", description: "What to say", required: false, kind: "string" },
      ] },
      { name: "Bad Name", description: "ignored", options: [] },
    ] },
    { bot: "m9", bot_name: "Other", commands: [{ name: "echo", description: "Say it back", options: [{ name: "text", description: "The words", required: true, kind: "string" }] }, { name: "move", description: "Move the chat", options: [{ name: "to", description: "Where", required: true, kind: "channel" }] }] },
  ],
};
const catalog = slashCatalog(server);
const by = name => catalog.find(c => c.name === name);

test("limits and the marker type match the core crate", () => {
  const constant = name => Number(new RegExp(`pub const ${name}: usize = (\\d+);`).exec(core)[1]);
  assert.equal(COMMAND_MIME, /pub const COMMAND_MIME: &str = "([^"]+)"/.exec(core)[1]);
  assert.equal(MAX_SLASH_COMMANDS, constant("MAX_COMMANDS"));
  assert.equal(MAX_SLASH_NAME_CHARS, constant("MAX_NAME_CHARS"));
  assert.equal(MAX_SLASH_DESCRIPTION_CHARS, constant("MAX_DESCRIPTION_CHARS"));
  assert.equal(MAX_SLASH_OPTIONS, constant("MAX_OPTIONS"));
});

test("names are lowercase ascii with dashes, up to 32 characters", () => {
  assert.ok(validSlashName("roll"));
  assert.ok(validSlashName("roll-d20"));
  assert.ok(validSlashName("a".repeat(32)));
  assert.ok(!validSlashName("a".repeat(33)));
  assert.ok(!validSlashName("Roll"));
  assert.ok(!validSlashName("-roll"));
  assert.ok(!validSlashName("roll-"));
  assert.ok(!validSlashName(""));
});

test("the catalog flattens every bot's list, drops bad names and sorts by name", () => {
  assert.deepEqual(catalog.map(c => `${c.name}@${c.bot_name}`), ["echo@Other", "move@Other", "remind@Tidy", "roll@Tidy"]);
  assert.deepEqual(slashNames(catalog), ["echo", "move", "remind", "roll"]);
  assert.deepEqual(slashCatalog(null), []);
  assert.deepEqual(slashCatalog({ commands: [{ bot: "x", commands: [{ name: "x", options: [{ name: "o", kind: "date" }] }] }] })[0].options[0].kind, "string");
});

test("suggestions narrow by prefix with the exact name first", () => {
  assert.deepEqual(slashSuggestions("r", catalog).map(c => c.name), ["remind", "roll"]);
  assert.deepEqual(slashSuggestions("roll", catalog).map(c => c.name), ["roll"]);
  assert.deepEqual(slashSuggestions("", catalog, 2).map(c => c.name), ["echo", "move"]);
  assert.deepEqual(slashSuggestions("zz", catalog), []);
});

test("typed text matches a command by its first word", () => {
  assert.equal(matchSlashCommand("/roll 20", catalog).entry.name, "roll");
  assert.equal(matchSlashCommand("/roll 20", catalog).rest, "20");
  assert.equal(matchSlashCommand("/ROLL", catalog).rest, "");
  assert.equal(matchSlashCommand("/rolls 20", catalog), null);
  assert.equal(matchSlashCommand("roll 20", catalog), null);
  assert.equal(matchSlashCommand(" /roll", catalog), null);
});

test("arguments split on spaces and keep quoted text together", () => {
  assert.deepEqual(splitSlashArgs('@nova 10 "see you soon"').map(t => [t.value, t.quoted]), [["@nova", false], ["10", false], ["see you soon", true]]);
  assert.deepEqual(splitSlashArgs("").length, 0);
});

test("usage and hints follow the option being typed", () => {
  assert.equal(slashUsage(by("remind")), "/remind <who> <minutes> [note]");
  assert.equal(slashOptionAt(by("remind"), ""), 0);
  assert.equal(slashOptionAt(by("remind"), "@no"), 0);
  assert.equal(slashOptionAt(by("remind"), "@nova "), 1);
  assert.equal(slashOptionAt(by("remind"), "@nova 10 bring the"), 2);
  assert.equal(slashOptionAt(by("roll"), "20 extra words"), 0);
  assert.equal(slashHint(by("remind"), "@nova ").text, "In how many minutes (a whole number)");
  assert.equal(slashHint(by("remind"), "@nova 10 ").text, "What to say (text, optional)");
  assert.equal(slashHint(by("remind"), "").text, "Who to remind (@member)");
  assert.equal(slashHint(by("move"), "").text, "Where (#channel)");
  const bare = slashHint({ name: "ping", description: "Pong", options: [] }, "");
  assert.deepEqual([bare.at, bare.text, bare.usage], [-1, "Pong", "/ping"]);
});

test("options parse in order and resolve members and channels to ids", () => {
  const context = { members: server.members, channels: server.channels };
  assert.deepEqual(parseSlashOptions(by("remind"), "@Nova 10 bring the snacks", context), { options: { who: "m2", minutes: "10", note: "bring the snacks" }, problem: null });
  assert.deepEqual(parseSlashOptions(by("remind"), "nova 010", context).options, { who: "m2", minutes: "10" });
  assert.deepEqual(parseSlashOptions(by("roll"), "", context), { options: {}, problem: null });
  assert.deepEqual(parseSlashOptions(by("move"), "#general", context).options, { to: "c1" });
  assert.deepEqual(parseSlashOptions(by("echo"), '"quoted words" and more', context).options, { text: "quoted words" });
});

test("problems read as a sentence the composer can show", () => {
  const context = { members: server.members, channels: server.channels };
  assert.equal(parseSlashOptions(by("remind"), "", context).problem, "<who> is needed: /remind <who> <minutes> [note]");
  assert.equal(parseSlashOptions(by("remind"), "@nova", context).problem, "<minutes> is needed: /remind <who> <minutes> [note]");
  assert.equal(parseSlashOptions(by("remind"), "@nobody 10", context).problem, "who needs a member of this server, written as @name");
  assert.equal(parseSlashOptions(by("remind"), "@nova ten", context).problem, "minutes needs a whole number");
  assert.equal(parseSlashOptions(by("move"), "#Lounge", context).problem, "to needs a text channel, written as #name");
  assert.equal(parseSlashOptions(by("roll"), "20 21", context).problem, "/roll takes one option: /roll [sides]");
  assert.equal(parseSlashOptions({ name: "ping", description: "", options: [] }, "now", context).problem, "/ping takes no options");
});

test("chips show the options in declared order with names for ids", () => {
  const names = { memberName: id => ({ m2: "nova" })[id] || null, channelName: id => ({ c1: "general" })[id] || null };
  const used = { bot: "m4", name: "remind", options: { note: "snacks", who: "m2", minutes: "10" } };
  assert.deepEqual(slashChips(used, by("remind"), names), [{ name: "who", value: "@nova" }, { name: "minutes", value: "10" }, { name: "note", value: "snacks" }]);
  assert.deepEqual(slashChips({ options: { to: "c1" } }, by("move"), names), [{ name: "to", value: "#general" }]);
  assert.deepEqual(slashChips({ options: { b: "2", a: "1" } }, null, names), [{ name: "a", value: "1" }, { name: "b", value: "2" }]);
  assert.deepEqual(slashChips({ options: { who: "m9" } }, by("remind"), names), [{ name: "who", value: "@someone" }]);
});
