import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const {
  botPermissions, botAbilityText, botCommandsFor, serverBots, canSeeBotsPage, cleanBotName, botSetupCommands, newBots, BOT_PORTING_URL, BOT_DOWNLOAD_URL,
} = require("../scripts/bots.js");

const server = {
  can_manage_server: false,
  can_manage_roles: true,
  roles: [
    { id: "r-mod", name: "Moderators", permissions: ["manage_messages", "ban_members"] },
    { id: "r-admin", name: "Admins", permissions: ["administrator"] },
    { id: "r-fun", name: "Fun", permissions: [] },
  ],
  members: [
    { id: "u-ana", name: "ana", bot: false, roles: ["r-admin"], presence: "online" },
    { id: "b-tidy", name: "Tidy", bot: true, roles: ["r-mod"], presence: "offline" },
    { id: "b-dice", name: "Dice", bot: true, roles: ["r-fun"], presence: "online" },
  ],
  commands: [{ bot: "b-dice", bot_name: "Dice", commands: [{ name: "roll", description: "Roll a die", options: [] }] }],
};

test("abilities come from the bot's roles in plain words", () => {
  assert.deepEqual([...botPermissions(server, server.members[1])].sort(), ["ban_members", "manage_messages"]);
  assert.equal(botAbilityText(new Set(["manage_messages", "ban_members"])), "Can remove messages and ban members.");
  assert.equal(botAbilityText(new Set(["manage_messages", "kick_members", "manage_roles"])), "Can remove messages, kick members and change roles.");
  assert.equal(botAbilityText(new Set(["administrator", "kick_members"])), "Can do anything in this server (Administrator).");
  assert.match(botAbilityText(new Set()), /Give it a role/);
});

test("bots list online first, with roles and commands", () => {
  const bots = serverBots(server);
  assert.deepEqual(bots.map(b => b.member.name), ["Dice", "Tidy"]);
  assert.equal(bots[0].online, true);
  assert.deepEqual(bots[0].commands, [{ name: "roll", description: "Roll a die" }]);
  assert.deepEqual(bots[1].roles.map(r => r.name), ["Moderators"]);
  assert.deepEqual(botCommandsFor(server, "b-tidy"), []);
  assert.deepEqual(serverBots({ members: [] }), []);
});

test("the page is for people who manage the server or its roles", () => {
  assert.equal(canSeeBotsPage(server), true);
  assert.equal(canSeeBotsPage({ can_manage_server: true }), true);
  assert.equal(canSeeBotsPage({ can_kick: true }), false);
  assert.equal(canSeeBotsPage(null), false);
});

test("setup commands use a safe bot name and the invite", () => {
  assert.equal(cleanBotName("  Mod  Helper!! "), "Mod Helper");
  assert.equal(cleanBotName("$$$"), "Helper");
  assert.equal(cleanBotName("x".repeat(50)).length, 32);
  const commands = botSetupCommands("Mod Helper", "commyabc123");
  assert.deepEqual(commands, {
    create: 'commy-bot init "Mod Helper"',
    join: "commy-bot join commyabc123",
    run: "commy-bot run",
    bridge: "commy-bot run --bridge",
  });
  assert.equal(botSetupCommands("Tidy", "").create, "commy-bot init Tidy");
  assert.equal(botSetupCommands("Tidy", "").join, "commy-bot join INVITE-CODE");
  assert.ok(BOT_PORTING_URL.endsWith("#porting-a-bot"));
  assert.ok(BOT_DOWNLOAD_URL.endsWith("/releases/latest"));
});

test("new bots are the ones not seen when the sheet opened", () => {
  assert.deepEqual(newBots(["b-tidy"], server).map(b => b.member.id), ["b-dice"]);
  assert.deepEqual(newBots(["b-tidy", "b-dice"], server), []);
});
